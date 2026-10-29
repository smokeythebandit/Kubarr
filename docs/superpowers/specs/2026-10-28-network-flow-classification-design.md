# Network Flow Classification — Design

**Status:** revised 2026-10-28 after measuring on the production cluster. Supersedes the worker/gRPC design in this file's first revision.
**Scope:** sub-project A of two. Sub-project B (network page and dashboard tile rework) consumes the API defined here.

## Goal

Show how much of the cluster's network activity goes to the internet, to the
LAN, and between pods — on the dashboard as one overview, on the network page
in detail, and as real edges on the topology graph.

## Why this needs new data

The network page and dashboard tile are built on cAdvisor per-pod counters.
Those count bytes per interface with no information about where the bytes went.
The topology graph's edges come from `discover_service_connections`, which
infers links from Kubernetes Service definitions — what *could* talk, not what
did.

## What was measured

Every decision below was verified against the running cluster rather than
inferred from documentation.

- **Hubble was already enabled** on the agents and exporting nothing: no
  metrics server, no relay, a 4095-event ring buffer that had already churned
  ~2M events in five hours. Nothing retained.
- **Hubble cannot report bytes.** Its metrics are event counters; `flow.proto`
  has no byte field anywhere in 1033 lines. Flows per class are measurable,
  bytes per class are not.
- **CIDR identities can be minted with zero blast radius.** A
  `CiliumClusterwideNetworkPolicy` whose `endpointSelector` matches no pod still
  mints the identity: `cidr:192.168.1.0/24` appeared immediately, no endpoint
  was selected, nothing changed in the data path. All nine policy-bearing
  namespaces already enforce `Egress` anyway, so an allow rule can only permit
  more.
- **Flows carry the distinction**, over 3000 live egress flows:
  `reserved:world` alone = internet, `cidr:...,reserved:world` = LAN, a
  destination namespace = cluster, `reserved:host`/`reserved:kube-apiserver` =
  node infrastructure.
- **Context fallback works as needed.** Hubble's `destinationContext` accepts a
  chain and uses the *first non-empty* value, so `namespace|identity` yields a
  clean namespace for cluster traffic and identity labels for everything else.
- **End to end, it lands in VictoriaMetrics:** 274 Hubble series scraped,
  returning LAN 5.08/s and internet 4.13/s from one PromQL query.

## Decisions

| Decision | Choice | Rejected alternative and why |
|---|---|---|
| Source of the class split | Hubble's own Prometheus metrics, scraped by VM | A worker that streams Hubble flows over gRPC and classifies them in Rust. That was the previous revision of this spec. It works, but needs vendored protos, `tonic`/`prost`, a stream consumer and a metrics listener — roughly 600 lines — to produce what one Helm value and a PromQL query now produce. |
| How LAN is distinguished from internet | A no-op `CiliumClusterwideNetworkPolicy` mints CIDR identities for the LAN ranges; `destinationContext=namespace\|identity` then emits `cidr:...` for LAN and `reserved:world` for internet | `destinationContext=ip`, which makes destination IP a metric label. On a host running qBittorrent that is thousands of torrent peer IPs, each minting a permanent series — a cardinality bomb. |
| How the split is expressed | Flow events per second by class, alongside the existing measured byte totals | Estimating per-class bytes by apportioning cAdvisor volume by flow share. A few large transfers among many small flows skew it badly, and it would read as measurement. |
| Topology edges | Merge: Hubble supplies which edges exist and how busy they are; Services keep supplying names and ports | Replacing `discover_service_connections` outright. Hubble edges are namespace-to-namespace with no port, and `labelsContext` has no port label, so the Service metadata is still worth having. |
| Where the Cilium configuration lives | `kubarr-cli`, which already installs third-party charts through Helm | A manual `helm upgrade` (a rebuilt cluster silently loses the data). |

## Which trace point answers which question

Hubble emits events at several datapath points, and they are **not**
interchangeable. Measured counts per destination class:

| class | `to-endpoint` | `to-stack` | `to-network` |
|---|---|---|---|
| cluster (namespace) | 4252 | — | — |
| cluster (node) | — | 987 | — |
| internet | — | 444 | 177 |
| lan | — | — | 115 |

Two consequences that no amount of reading the documentation would reveal:

1. **Internet traffic fires at two points, LAN at one.** Summing all subtypes
   would inflate internet roughly 1.4x against LAN. Class totals therefore come
   from `to-network` only, where both are measured identically.
2. **`to-network` happens after masquerading, so the source is always the
   host.** Per-app attribution is impossible there and must come from
   `to-stack`, which is the last point where the pod identity survives —
   verified: `qbittorrent -> internet` at 10.99/s.

The queries therefore are:

| Question | Query |
|---|---|
| Class totals (internet, LAN) | `sum by (destination) (rate(hubble_flows_processed_total{traffic_direction="egress",subtype="to-network",verdict="FORWARDED"}[1m]))` |
| Class total (cluster) | same with `subtype="to-endpoint"` |
| Per-app external | `sum by (source, destination) (... subtype="to-stack" ...)` |
| Topology edges | `sum by (source, destination) (... subtype="to-endpoint" ...)` |
| Drops | same shape with `verdict="DROPPED"` |

### One scrape path, not two

Enabling `hubble.metrics.enabled` makes Cilium's own chart create a
`hubble-metrics` Service that already carries `prometheus.io/scrape`, and
VictoriaMetrics discovers Services as well as Pods. Annotating the agent Pods
as well therefore gives one counter two scrape paths. Measured on production:
146 identical series under `job="kubernetes-pods"` and 146 more under
`job="kubernetes-services"`, which made every `sum by (destination)` report
exactly twice the real flow rate — 1795 flows to `reserved:world` where the
agent had counted 1182 and the Service path 611 of the same events. The CLI
sets no Pod annotations for this reason, and a test asserts it stays that way.

A documented consequence: **LAN traffic is host-originated**, because the NFS
mounts are performed by the kubelet rather than by the pods. Per-app LAN
attribution is therefore legitimately near-empty — that is the system telling
the truth, not a bug.

## Architecture

```
kubarr-cli --helm upgrade--> Cilium 1.20.4
                               hubble.metrics.enabled = flow + drop,
                               destinationContext = namespace|identity
                                 (which makes Cilium's own chart create the
                                  already-annotated hubble-metrics Service)
                                     |
                     CiliumClusterwideNetworkPolicy (selects no pod)
                       mints cidr: identities for the LAN ranges
                                     |
                  VictoriaMetrics (existing kubernetes-services job)
                                     | rate() over 1m
                     kubarr-backend  GET /api/networking/flow-classes
                                     GET /api/networking/topology (edges merged)
                                     |
                        dashboard tile + network page  (sub-project B)
```

No new database tables, no new Rust dependencies, and no worker changes.

## Components

### Classification mapping (backend, tested)

The only logic that can silently misreport. A destination label string maps to
a class, **identity before address**:

| Destination label value | Class |
|---|---|
| contains `reserved:host`, `reserved:remote-node`, `reserved:kube-apiserver`, or `reserved:ingress` | `cluster` |
| contains no `:` at all (a bare namespace name) | `cluster` |
| contains a `cidr:` label | `lan` |
| `reserved:world`, `reserved:world-ipv4`, `reserved:world-ipv6` | `internet` |
| anything else, or empty | `unclassified` (never surfaced as a class) |

Ordering matters and is tested: apiserver traffic to `192.168.1.20:6443` carries
*both* `cidr:192.168.1.20/32` **and** `reserved:host`. A draft script that
checked `cidr:` first filed 438 control-plane flows as LAN traffic. Identity
first prevents exactly that.

### API

`GET /api/networking/flow-classes?window=15m|1h|6h|24h` returns class totals,
per-app rates, and series, with `stale` set when no Hubble series are present.
`GET /api/networking/topology` keeps its current shape and gains measured edge
weights, with Service-derived ports and names preserved.

### Charts

- **`monitoring/victoriametrics`**: `kube-system` added to
  `networkPolicy.scrapeNamespaces`. VM runs default-deny egress with an explicit
  allow-list, so without this the scrape is blocked and every panel is silently
  empty. Verified: the policy denied it until the namespace was added.
- **New chart resource**: the `CiliumClusterwideNetworkPolicy` that mints the
  CIDR identities, rendered only where the `cilium.io/v2` CRD exists, with the
  LAN ranges configurable.

### CLI

`kubarr-cli` enables Hubble metrics on an existing Cilium release using
`--reuse-values`, and adds `kube-system` to the VM scrape list it already
passes. It deliberately annotates nothing: see "One scrape path, not two".
Where no Cilium release exists it warns and continues: the single-node
path installs k3s with its own flannel, and replacing a working CNI is not
something an app installer should do quietly.

## Failure handling

| Failure | Behaviour |
|---|---|
| Hubble metrics disabled or agents unscraped | No series; API reports `stale: true`; page falls back to byte volumes and Service-derived edges |
| CIDR identity policy missing | LAN collapses into `reserved:world`, i.e. internet. The API reports LAN as zero rather than guessing; the CLI check surfaces it |
| VM unreachable | `stale: true` with empty series, matching how the other monitoring endpoints degrade |
| Cilium agent restart | Counters reset; `rate()` handles resets natively |

## Testing

| Unit | How |
|---|---|
| Classification mapping | Table-driven over real label strings captured from the cluster, including the apiserver case that carries both `cidr:` and `reserved:host`, bare namespaces, `world-ipv4`/`world-ipv6`, and empty |
| API handler | Seeded VM responses: populated, empty, malformed samples, and missing series -> `stale` |
| Topology merge | A Hubble edge with no Service keeps its weight; a Service with no traffic is not drawn; ports survive the merge |
| Charts | Rendered-manifest assertions that VM's egress includes `kube-system` and that the CCNP renders only with the Cilium CRD present |
| CLI | Dry-run assertion of the Helm arguments, including `--reuse-values`, no `--version` pin against a running CNI, and no Pod annotations (which would double-count) |

## Sub-project B: the UI

Built 2026-10-29 on the API above.

| Piece | What it does |
|---|---|
| Dashboard tile | Keeps the byte figures, adds total flow rate and a three-segment bar with per-class rates. Links to `/networking` rather than `/resources`. Falls back to the old rx/tx sparkline when Hubble has nothing. |
| Summary cards | Internet, LAN and In-cluster rates with each class's share of the total and its denied-flow rate, beside the two byte cards. The always-`0.00` Errors and Dropped cards are gone: those were cAdvisor interface counters, and policy denials are the drop that matters. |
| `Where traffic goes` | A window selector (15m/1h/6h/24h), the split bar, and a stacked area chart. |
| Per-app table | Three class columns beside the byte columns. Rows are the union of both sources, so a flow source with no pod behind it still appears. |
| Topology edges | Width and particle speed come from the measured flow rate on a log scale; a Service-derived edge with no traffic is drawn thin and dashed with nothing moving on it, and the graph carries a legend saying which is which. |

Three decisions worth keeping:

- **Flow rates are never converted to bytes.** Hubble counts events; cAdvisor
  counts bytes with no destination. The two stay in separate cards.
- **Classes can be hidden from the chart.** In-cluster runs an order of
  magnitude above the other two — 61/s against 3/s on the live cluster — so on
  one shared axis internet and LAN flatten onto the baseline. Clicking a legend
  entry drops that class and rescales the axis, which is the only way to read
  the small classes against each other.
- **A real but tiny class still gets a visible sliver** in the split bar. LAN at
  0.2% of the total would otherwise vanish, and "almost no LAN traffic" is a
  different answer from "none".

Also done here: `NetworkingPage.tsx` dropped from 1063 lines to a composition
over seven components, `window.location.reload()` became query invalidation, VPN
configs moved onto the query cache instead of their own timer, and the HTTP
polling fallback went from 1 s to 5 s.

## Out of scope

- Per-class byte volumes. Not measurable.
- Hubble relay, Hubble UI, and L7/DNS metrics. The `flow` and `drop` metrics are
  enough for this; DNS and HTTP metrics are a cheap follow-up once the scrape
  exists.
