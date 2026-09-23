# Live WireGuard acceptance — 2026-09-23

**PASS — process exit status 0, including scoped cleanup.**

The run used a separate disposable Kubernetes 1.35.8 / Kind 0.33.0 cluster,
Helm 4.3.0, actual Sonarr, and Gluetun v3.41.3. Charts were pinned to
`01e45468f233b03b4a7d70a320aec9075f77c213`. The Gluetun OCI digest was verified as
`sha256:fa19cc76b2af13d57a8d3dc3066f2ada061b1c761b8aecf989b3877c0486e027`.

The existing `kubarr-local` environment was not used or modified.

## Measured results

| Phase | Evidence |
| --- | --- |
| Before VPN | Sonarr reached the public fixture as `203.0.113.10`; the private fixture was unreachable and recorded no request token. |
| UI assignment | Playwright created a custom WireGuard provider and assigned it to Sonarr; the queued update succeeded and replaced the pod. |
| Tunnel established | A real server handshake and client `tun0` WireGuard interface were observed. Public/private fixtures saw `203.0.113.2` / `10.77.0.1`. Server RX/TX increased by **2,784 / 2,688 bytes** over controlled probes. |
| Gateway stopped | Three rounds kept the non-VPN control connected while protected public/private requests failed. Endpoint event histories contained none of the protected tokens. Sonarr remained running in the same pod. |
| Tunnel route removed | Gluetun PID 1 was paused and its pod-local `tun0` removed. The public route fell back through `eth0` via `10.244.0.1`. With firewall enabled, protected traffic still failed and the public endpoint recorded no protected token; the control remained reachable. |
| Recovery | Gluetun resumed and the server restarted. A new handshake occurred; both routed source identities returned. Fresh RX/TX counters increased by **2,464 / 2,688 bytes**. |
| UI removal | Playwright removed the assignment and deleted the unassigned provider. Sidecar and managed Secret disappeared; direct public access returned as `203.0.113.10`, while private access remained unavailable. |
| Cleanup | Sonarr uninstalled, its NFS sentinel remained intact, and all run-owned cluster/container/network/key-volume resources were removed. |

Independent endpoint event recording is important: a failed HTTP client alone
would not prove that its request never escaped. The route-loss phase is also
important: a dead tunnel route alone could block traffic even without a working
firewall.

The local transcript and exit status are retained in the git-ignored
`.acceptance-runs/vpn-third-live/`. No operation or Helm release record was
manually marked successful. Generated keys were removed during cleanup.

## Scope

This proves the exercised **controlled IPv4 HTTP routing and kill-switch paths**,
not universal leak prevention. IPv6, DNS leaks, malformed/wrong peer keys, OpenVPN,
commercial-provider integrations, hostile workloads with networking privileges,
and arbitrary internet destinations remain separate test cases. This is local
evidence; it does not claim that GitHub Actions has already run successfully.
