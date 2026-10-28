# Network Flow Classification — Design

**Status:** approved in chat 2026-10-28, ready for implementation planning
**Scope:** sub-project A of two. Sub-project B (network page and dashboard tile rework) gets its own spec and consumes the API defined here.

## Goal

Show how much of the cluster's network activity goes to the internet, to the
LAN, and between pods — on the dashboard as one overview, and on the network
page in detail.

## Why this needs new data

Both the network page and the dashboard tile are built on cAdvisor per-pod
counters. Those count bytes per interface and carry no information about where
the bytes went, so no amount of frontend work can answer the question.

Two facts, verified against the running cluster, shape everything below:

- **Hubble is already enabled** on the agents (`enable-hubble=true`,
  listening `:4244`, 1.96M flows observed) but exports nothing: no metrics
  server, no relay. Cilium's own agent Prometheus endpoint is disabled
  (`prometheus-serve-addr` empty).
- **Hubble metrics cannot express bytes.** The available metrics are event
  counters (`dns, drop, tcp, flow, icmp, http`). Flow records carry source and
  destination identity but no byte counts.

Therefore: **flows per second by destination class is exactly measurable; bytes
per second by destination class is not.** This design measures flows by class
and keeps bytes as the existing total-volume signal. Nothing on screen is
inferred.

## Decisions

| Decision | Choice | Rejected alternative and why |
|---|---|---|
| How to express the split | Flows/sec by class, plus measured total bytes | Estimating per-class bytes by splitting cAdvisor volume in proportion to flow counts. A few large transfers among many small flows skew it badly, and it would read as measurement. |
| Where the LAN/internet distinction is made | Worker consumes raw Hubble flows over gRPC and classifies by destination IP | Minting CIDR identities via a `CiliumCIDRGroup` plus a policy, then reading the split from Hubble metrics. Less code, but in Cilium any policy selecting an endpoint flips it to default-deny for that direction — a live-cluster connectivity risk this project already hit once. |
| Who owns the Cilium install | `kubarr-cli`, as a pinned bootstrap step with Hubble values | A wrapper chart in `kubarr-charts` (a CNI installed by machinery that needs the CNI already running is awkward ordering), or a documented manual `helm upgrade` (a rebuilt cluster silently loses the data). |
| Where the series live | VictoriaMetrics, scraped from the worker | Our own bucket table in PostgreSQL. VM already holds 7 days, already backs the dashboard sparkline, and already has retention and range queries. Hand-rolling buckets meant writing retention trimming for no gain. |

## Architecture

```
kubarr-cli  ──installs──>  Cilium 1.20.4 (hubble.enabled, hubble.relay.enabled)
                                   │
                            hubble-relay:4245  (Service port 80, in-cluster plaintext)
                                   │ gRPC Observe(follow=true)
                        kubarr-worker  flow_watch
                                   │ classify by destination
                        in-memory counters ──> GET /metrics  (:9100)
                                   │ scraped by existing kubernetes-pods job
                            VictoriaMetrics (retention 7d)
                                   │ rate() over 1m
                        kubarr-backend  GET /api/networking/flow-classes
                                   │
                            dashboard tile + network page   (sub-project B)
```

Relay's server TLS defaults to off, so its in-cluster listener is plaintext and
the worker needs no client certificates; relay-to-agent keeps the mTLS the
cluster already has (`hubble-disable-tls=false`) via the chart's automatic TLS.

## Components

### kubarr-cli: `src/cni.rs`

A bootstrap step that installs Cilium idempotently (`helm upgrade --install`),
following the existing pattern in `src/observability.rs`, which already
installs third-party charts through Helm with `--set` arguments.

Produces:
- `pub fn install_cni(dry_run: bool) -> anyhow::Result<()>`
- Pinned chart version constant, matching the cluster's current `cilium-1.20.4`.
- Values set: `hubble.enabled=true`, `hubble.relay.enabled=true`.
- A `doctor` check reporting whether relay is reachable, so a cluster missing
  flow data says so in the CLI rather than only in the UI.

The CLI must not silently reinstall or downgrade a working CNI: if a Cilium
release already exists at a different version, the step reports and leaves it
alone unless explicitly asked to upgrade.

### Worker: flow consumption

Three units, split so the part that can be wrong in subtle ways is pure:

**`code/api/src/services/flow_classifier.rs`** — pure classification.

```rust
pub enum FlowClass { Internet, Lan, Cluster }

/// Classify by destination. `identity_labels` are Cilium's reserved labels for
/// the destination endpoint; `destination_ip` is the raw address.
pub fn classify(identity_labels: &[String], destination_ip: Option<IpAddr>, lan: &[IpNet]) -> Option<FlowClass>
```

Rules, in order:

| Destination | Class |
|---|---|
| labels contain a cluster identity (pod, service, `reserved:remote-node`, `reserved:host`, `reserved:kube-apiserver`) | `Cluster` |
| IP inside a configured LAN CIDR | `Lan` |
| any other routable IP | `Internet` |
| no destination IP and no usable identity | `None` — rendered under `class="unclassified"`, never guessed |

`reserved:host` and `reserved:remote-node` classify as `Cluster` even though
the nodes themselves sit on the LAN: that traffic is kubelet, apiserver and
node-to-node overlay, which is cluster infrastructure rather than the LAN
activity this split is meant to surface. Traffic to *other* LAN hosts — the NFS
server, for instance — carries no node identity and classifies as `Lan`.

The identity check runs before the CIDR check deliberately. A pod CIDR that
overlaps an RFC1918 LAN range would otherwise misclassify in-cluster traffic as
LAN, which is why that case is an explicit test.

LAN defaults to RFC1918 (`10/8`, `172.16/12`, `192.168/16`) minus the cluster's
pod and service CIDRs, overridable with `KUBARR_LAN_CIDRS` as a comma-separated
list, read once at worker start like the worker's other interval settings.
Loopback and link-local are excluded from all three classes.

**`code/api/src/services/flow_metrics.rs`** — counter registry and text
rendering.

```rust
pub struct FlowCounters { /* app × class × verdict → u64 */ }
impl FlowCounters {
    pub fn record(&self, app: &str, class: FlowClass, verdict: Verdict);
    // Verdict is Forwarded or Dropped, taken from the flow's own verdict field.
    pub fn render(&self) -> String;   // Prometheus text format
    pub fn set_connected(&self, connected: bool);
}
```

Rendered by hand rather than through a Prometheus client crate: it is a handful
of counters, it keeps the dependency list flat, and the exact output becomes
assertable in a unit test.

```
# TYPE kubarr_network_flows_total counter
kubarr_network_flows_total{app="sonarr",class="internet",verdict="forwarded"} 48213
# TYPE kubarr_network_flow_watch_connected gauge
kubarr_network_flow_watch_connected 1
```

Cardinality is roughly 20 apps × 3 classes × 2 verdicts ≈ 120 series.

**`code/api/src/services/flow_watch.rs`** — the gRPC consumer. Opens Hubble's
`observer.Observer/GetFlows` streaming RPC with `follow=true`, classifies each
flow, increments counters, and reconnects with backoff.

Only flows with `traffic_direction == EGRESS` and `Type == L3_L4` are counted.
Every pod-to-pod flow is reported at both ends, so counting ingress as well
would double every figure, and L7 events shadow the L3 flow they belong to.
Verdicts other than `FORWARDED` and `DROPPED` describe policy evaluation rather
than traffic and are skipped. Apps are attributed from the flow's *source*
namespace, which is what makes egress-only counting the right choice. Sets the connected gauge to 0 while
disconnected so staleness is visible downstream.

Hubble's protos (`observer.proto`, `flow.proto`, and `relay.proto`, which
`observer.proto` imports) are vendored from cilium/cilium at the pinned version and compiled with `tonic-build` in a
`build.rs`; `tonic` and `prost` are new dependencies.

**Metrics listener.** The worker currently has no HTTP surface at all — no
Service, no container ports, no probes. `run_worker` gains a small axum server
serving `GET /metrics` on `:9100`, spawned alongside the pod watch.

### Charts

- **`system/kubarr-worker`**: pod gains `containerPort: 9100` and the
  `prometheus.io/scrape: "true"` / `prometheus.io/port: "9100"` annotations the
  existing `kubernetes-pods` scrape job keys on. No new VM scrape config.
- **`monitoring/victoriametrics`**: VM runs default-deny egress with an
  explicit per-namespace allow-list (sonarr, radarr, qbittorrent, …) that does
  **not** include `kubarr-worker`. Without adding it, VM cannot scrape the
  worker and the failure mode is an empty graph with no error reported
  anywhere. Add the worker namespace and port to that egress list.

Both charts take a version bump under the existing immutable-revision
convention.

### API: `GET /api/networking/flow-classes`

In `code/api/src/endpoints/networking.rs`, reusing `query_vm_range` — the same
VictoriaMetrics range-query helper that backs the dashboard's network history.

Query parameter `window`: `15m` | `1h` | `6h` | `24h` (default `15m`), all
within VM's 7-day retention.

```json
{
  "observed_at": 1793193085,
  "stale": false,
  "totals": {
    "internet": { "flows_per_sec": 12.4, "drops_per_sec": 0.0 },
    "lan":      { "flows_per_sec": 31.8, "drops_per_sec": 0.0 },
    "cluster":  { "flows_per_sec": 94.2, "drops_per_sec": 0.2 }
  },
  "by_app": [
    { "app": "sonarr",
      "internet": { "flows_per_sec": 2.1, "drops_per_sec": 0.0 },
      "lan":      { "flows_per_sec": 0.4, "drops_per_sec": 0.0 },
      "cluster":  { "flows_per_sec": 8.8, "drops_per_sec": 0.0 } }
  ],
  "series": {
    "internet": [{ "t": 1793193000, "flows_per_sec": 11.9 }],
    "lan":      [{ "t": 1793193000, "flows_per_sec": 30.2 }],
    "cluster":  [{ "t": 1793193000, "flows_per_sec": 92.7 }]
  }
}
```

`drops_per_sec` is the same counter filtered to `verdict="dropped"`, and
`observed_at` is the timestamp of the newest sample VM returned, so a client can
see how old the numbers are. `stale` is true when
`kubarr_network_flow_watch_connected` is 0 or absent. It
exists so the UI can say "flow data unavailable" instead of drawing a
convincing zero. Authorization matches the existing networking routes;
responses carry `short_cache(10)`.

## Failure handling

| Failure | Behaviour |
|---|---|
| Relay unreachable or stream drops | Worker reconnects with backoff, gauge goes 0, API reports `stale: true`, page falls back to the byte volumes it already shows |
| Cilium installed without Hubble | No series in VM; API reports `stale: true`; `kubarr doctor` reports relay unreachable |
| VM unreachable | API returns `stale: true` with empty series rather than an error, matching how existing monitoring endpoints degrade |
| Flow with no usable destination | Counted as unclassified; never assigned a class by guesswork |
| Worker restart | Counters reset; VM's `rate()` handles counter resets natively |

The network page must never go blank because flow data is missing. Bytes,
topology, and the per-app table all keep working from existing sources.

## Testing

| Unit | How |
|---|---|
| `flow_classifier` | Table-driven tests over destination identity/IP combinations: pod-to-pod, pod-to-service, world, each RFC1918 range, a LAN address that overlaps the pod CIDR (must classify as cluster), loopback, link-local, IPv6, and missing destination. This is the one place a wrong answer silently misattributes everything, so it is tested exhaustively and in isolation. |
| `flow_metrics` | Exact rendered-text assertions, including label escaping and the connected gauge. |
| `flow_watch` | Driven against a fake flow stream rather than a live relay: verifies classification wiring, reconnect behaviour, and that the gauge drops on disconnect. |
| API handler | Seeded VM responses, including the empty and error cases that must produce `stale: true`. |
| Charts | Rendered-manifest assertions that the worker carries the scrape annotations and port, and that VM's egress allow-list includes the worker namespace. |
| CLI | Dry-run assertion of the Helm arguments, and that an existing different-version release is reported rather than overwritten. |

## Out of scope

- Sub-project B: the dashboard tile's three-way row, the network page's summary
  cards, per-app class columns, window selector, splitting the 1063-line
  `NetworkingPage.tsx`, replacing `window.location.reload()` with query
  invalidation, moving VPN configs onto the shared query cache, and raising the
  1 s polling fallback. Separate spec, consumes this API.
- Per-class byte volumes. Not measurable; explicitly rejected above.
- Hubble UI, flow-level drill-down, and DNS/HTTP layer-7 metrics.
- Replacing the existing WebSocket topology pipeline.
