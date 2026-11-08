# VPN server picker and VPN deploy fixes — design

Date: 2026-11-07
Status: reviewed, awaiting owner approval

## Why

Kubarr routes an app's traffic through a gluetun sidecar when VPN is enabled
(today mostly qBittorrent). On the owner's cluster, with a 2 Gbit/s symmetrical
fibre line and a 2.5 Gbit/s LAN, enabling VPN for qBittorrent surfaced four
problems:

1. **The deploy failed.** Talos enforces Pod Security `baseline` cluster-wide.
   Kubarr creates app namespaces itself (`namespace.create=false`) without Pod
   Security labels, and gluetun needs `NET_ADMIN` and a `/dev/net/tun` hostPath,
   which `baseline` forbids. The old pod was stopped and no new one could be
   created; the app was down until the namespace was labelled `privileged` by
   hand.
2. **DNS through the VPN takes 6–12 s per external name.** With VPN on, the pod
   resolves through gluetun (`nameserver 127.0.0.1`) but keeps Kubernetes'
   `ndots:5` and three `*.cluster.local` search suffixes, so each name is first
   tried three times with a cluster suffix; gluetun forwards those through the
   tunnel and each attempt hangs ~2 s. A fully-qualified lookup takes 2 ms.
3. **The server is random.** For a named provider with WireGuard (ProtonVPN),
   Kubarr only stores the private key and address. Country/city/hostname
   filters exist in the backend and form only for OpenVPN. gluetun therefore
   picks one of ~800 Proton WireGuard servers at random (it landed in Bosnia).
4. **Throughput is far below the line.** Measured ~170 Mbit/s through the tunnel
   vs ~1.1 Gbit/s direct from the same cluster, with gluetun using ~1m CPU —
   the limit is the chosen server/route, not gluetun.

The owner wants Kubarr to pick a good server automatically, the way Proton's
apps do ("closest server, low load, P2P + port forwarding for torrents"),
without signing in to Proton.

## Goals

- A new VPN pod is created successfully on a cluster that enforces `baseline`.
- External DNS lookups from VPN pods are as fast as without VPN.
- WireGuard with a named provider can be limited to countries, cities or
  hostnames, like OpenVPN can today.
- A sidecar picks the fastest server it can measure, at start and when the
  connection degrades, for every app with VPN enabled, for any provider gluetun
  supports.
- When port forwarding is on, the app (qBittorrent) always listens on the port
  the provider currently forwards.

## Non-goals

- **No Proton account login or session.** Proton's server list with load
  figures (`/vpn/logicals`) requires an authenticated account session
  (verified: 401 without one; gluetun's own updater signs in with email and
  password and refuses 2FA accounts). Storing a full-account session in the
  cluster was rejected by the owner. Load is inferred from measured speed.
- No provider-specific APIs at all; the picker works from gluetun's own server
  list.
- No change to how VPN credentials are entered or stored, beyond the WireGuard
  filters.
- No multi-tunnel or bonding to exceed a single server's throughput.

## Anti-goals (would make this a failure even if it works)

- Any traffic from the app leaving outside the tunnel, including during server
  probing or switching.
- A broken picker taking the app down: if the picker fails, gluetun must still
  connect the way it does today.
- Constant server hopping that keeps torrents reconnecting.

## Design

### 1. Pod Security level follows VPN (backend)

When Kubarr ensures an app namespace before Helm runs, it sets
`pod-security.kubernetes.io/{enforce,audit,warn}`:

- `privileged` when the app has VPN enabled (gluetun needs `NET_ADMIN`,
  `/dev/net/tun`, and the picker needs `NET_RAW`, see below);
- otherwise the level the chart declares for its namespace (the charts already
  pass a level to `kubarr-common.namespace`, e.g. `baseline` for qBittorrent).

Disabling VPN removes the three labels again on the next deploy, returning the
namespace to the cluster default (`baseline` on Talos). The backend only
touches namespaces it ensures for apps, never system namespaces Helm owns, and
needs no per-chart level: it either sets `privileged` (VPN on) or removes its
labels (VPN off). *(My call.)* The label change happens before
`helm upgrade`, so the new pod is admitted. The qBittorrent
chart's own `namespace.yaml` should follow the same rule for the
`namespace.create=true` path.

### 2. DNS options for VPN pods (charts)

`kubarr-common` adds, whenever `vpn.enabled`, a pod `dnsConfig` with
`options: [{name: ndots, value: "1"}]`. Cluster-internal names used from inside
VPN pods are fully qualified (`*.svc.cluster.local`), so they are unaffected.
Exposed as a helper (e.g. `kubarr-common.gluetun.dnsConfig`) that each
VPN-capable chart includes in its pod spec, the same way the container,
init container and volumes helpers are included today.

### 3. WireGuard server filters (backend + frontend)

`build_wireguard_secret_data` in `vpn.rs` accepts `server_countries`,
`server_cities`, `server_hostnames` (and writes `SERVER_COUNTRIES`,
`SERVER_CITIES`, `SERVER_HOSTNAMES`) exactly as the OpenVPN path does. The
provider form shows these fields for WireGuard with any named provider, not
only for OpenVPN. They are optional and, when the picker is enabled, act as the
candidate pool the picker chooses from.

### 4. The server picker sidecar

A small program shipped as its own image (`kubarr-vpn-picker`), added to every
VPN pod by `kubarr-common` when `vpn.enabled` and `vpn.picker.enabled`
(default on). It has two roles in the same pod:

**a) Init container `vpn-picker-init` (before gluetun starts)**

Runs before the gluetun container, so gluetun's firewall does not exist yet and
direct probes to candidate servers are possible. Only the picker's own probe
packets (ICMP echo / TCP connect, no payload traffic) leave outside a tunnel;
the app container has not started, so no app traffic can leak.

1. Load gluetun's server list for the pinned gluetun version.
2. Filter candidates: provider and VPN type from the VPN secret; the
   country/city/hostname filters if set; `port_forward` capable if port
   forwarding is on; P2P-capable only when the chart says the app is a
   download client (`vpn.picker.p2p: true`, set in the qBittorrent,
   Transmission, Deluge and ruTorrent charts) so Prowlarr or Jellyfin keep the
   full pool; exclude secure-core, Tor and free-tier entries.
3. If no country filter is set, prefer servers near the cluster: rank by
   measured latency over the whole filtered set, capped to a sane sample
   (e.g. 40 random candidates) to keep startup short.
4. Measure latency to each candidate's entry IP (ICMP echo, falling back to
   TCP connect on 443 where ICMP is filtered); keep the best N (default 5).
5. Write the shortlist to a shared `emptyDir` and set the first entry as the
   initial `SERVER_HOSTNAMES` gluetun starts with (see "handing the choice to
   gluetun").
6. Time budget: if anything fails or takes longer than the budget (default
   60 s), exit 0 without a choice; gluetun then starts exactly as today
   (random within its filters). The picker never blocks the pod.

**b) Sidecar container `vpn-picker` (after gluetun is up)**

Shares the pod network, so all its traffic goes through the tunnel.

1. Wait until gluetun reports the VPN connected (control API).
2. Speed round: for each shortlisted server, switch gluetun to it via
   `PUT /v1/vpn/settings`, wait for connected, run a short download test
   (default 8 s or 100 MB, whichever first) against a configurable list of
   endpoints, record the throughput. Then switch to the best one and stay.
   The first round runs once at pod start, before the app has had time to
   build up many connections.
**App-agnostic measurement.** *(My call.)* The picker knows nothing about the
app it sits next to. Throughput is measured as total traffic on the tunnel
interface during the test window (the test download plus whatever the app is
moving), so a busy torrent client does not make the current server look slow,
and a saturated tunnel counts as fast. "Idle" (for
`switchWhileRunning=false`) means tunnel traffic below a small threshold
(default 1 MB/s) for the whole check window.

3. Monitor: every interval (default 15 min) run a short test on the current
   server. Re-run the speed round only when throughput stays below a threshold
   (default 40 % of the round's best) for several consecutive checks
   (default 3), and never more often than a cool-down (default 6 h).
   Switching while downloads run is allowed (the owner did not object; ~10 s
   interruption) and can be turned off (`vpn.picker.switchWhileRunning=false`),
   in which case the picker only re-runs when the tunnel is idle (see above) or
   down. *(Default: my call.)*
4. Expose its state (current server, last results, next check) as a small JSON
   status endpoint and in logs, for Kubarr's UI later.

Kill switch stays on throughout: switching uses gluetun's own reconnect, so
traffic is blocked, not leaked, while the tunnel is down.

**Handing the choice to gluetun.** gluetun reads `SERVER_HOSTNAMES` at start;
the init container cannot change another container's environment. Options for
the builder, in order of preference:

1. gluetun starts with the normal filters, and the sidecar applies the chosen
   server through `PUT /v1/vpn/settings` as its first action (one extra
   reconnect at boot);
2. or the gluetun container's command is wrapped to source an env file the
   init container wrote.

Option 1 needs no change to the gluetun image and is preferred.

**gluetun control API.** Verified in gluetun v3.41.3 source:
`PUT /v1/vpn/settings` merges a partial VPN settings object into the current
settings, validates it and restarts only the VPN loop; `PUT /v1/vpn/status`
stops/starts it; `GET /v1/vpn/status`, `GET /v1/publicip/ip` read state.

*Protection (decided by the owner): network policy only.* The control API stays
without credentials. The picker reaches it over localhost inside the pod; from
outside, the existing NetworkPolicy already limits port 8001 to the Kubarr
backend pod, which reads `GET /v1/portforward` and `GET /v1/publicip/ip` today
and must keep working. gluetun's control-server auth assigns roles per route
and cannot filter by source address. If gluetun's default denies the `PUT`
routes without credentials, the chart configures an auth role of `none` for
`PUT /v1/vpn/settings` and `PUT /v1/vpn/status` (and keeps the read routes
open), rather than introducing a key. Accepted risk: anything running in the
backend pod could switch servers or stop the tunnel; the kill switch still
prevents leaks when the tunnel is stopped.

**Port forwarding.** When `vpn.portForwarding.enabled`, gluetun already runs
`VPN_PORT_FORWARDING_UP_COMMAND` on every new forwarded port. The qBittorrent
chart sets a default up-command that calls qBittorrent's local Web API
(`/api/v2/app/setPreferences` with `listen_port`, which works without a
password from localhost in the Kubarr chart) so the port follows every
reconnect, including picker switches.

### 5. Configuration surface (chart values)

```yaml
vpn:
  picker:
    enabled: true
    shortlist: 5
    sample: 40
    initBudget: 60s
    testSeconds: 8
    testEndpoints: [...]        # sensible public defaults
    interval: 15m
    degradeThreshold: 0.4
    degradeChecks: 3
    cooldown: 6h
    switchWhileRunning: true
    image: {repository, tag, pullPolicy}
    resources: {...}
```

Kubarr's VPN settings page may later expose `enabled` and the country filter;
not required for the first version.

## Constraints

- Cluster: Talos, Pod Security `baseline` by default; namespaces created by the
  backend before Helm runs.
- Storage for downloads is NFS; irrelevant to the picker.
- gluetun is pinned (`v3.41.3`); the picker must use the server list matching
  the pinned version and tolerate a newer one.
- The picker image must be small, run as non-root except for `NET_RAW` (ICMP),
  read-only root filesystem, no service-account token.
- Must not depend on the Kubarr backend at runtime: a VPN pod keeps working if
  Kubarr is down.

## Decided by the owner

- The sidecar (not the Kubarr backend/worker) does the server picking.
- No Proton login; infer load from measured speed.
- The scope includes the Pod Security, DNS and WireGuard filter fixes.
- The gluetun control API is protected by the NetworkPolicy only, no API key.

## My calls (owner can overrule)

- The backend sets `privileged` with VPN on and removes its labels with VPN off;
  no per-chart level lookup.
- The picker is app-agnostic: it measures total tunnel throughput, and P2P
  filtering applies only to download-client charts.

- Switching while downloads run is allowed, behind a sustained-degradation
  threshold and a 6 h cool-down, and can be turned off.
- Picker enabled by default for every VPN app.
- Language for the picker: Go (small static binary, like `dns-webhook`).
- Handing the choice to gluetun via its control API rather than wrapping its
  entrypoint.

## Left to the builder

- Where the server list comes from at runtime (gluetun's `servers.json` for the
  pinned tag from its repository, with a copy baked into the picker image as
  fallback) and how P2P/feature flags are read per provider.
- Exact ICMP vs TCP probing and how many probes per server.
- Default speed-test endpoints and how failures of a single endpoint are
  handled.
- The exact gluetun auth configuration that leaves the routes open, once tested.
- Tests: unit tests for candidate filtering and ranking; chart tests for the
  rendered pod (init container, sidecar, dnsConfig, Pod Security level); a live
  acceptance check on the cluster comparing throughput before/after.
