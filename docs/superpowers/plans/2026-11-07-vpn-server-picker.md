# VPN Server Picker and VPN Deploy Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** VPN apps deploy on a `baseline` cluster, WireGuard providers accept server filters, and a sidecar picks the fastest measurable VPN server at start and after sustained degradation.

**Architecture:** Backend (Rust, `code/api`) labels app namespaces `privileged` while VPN is on and passes WireGuard server filters into the gluetun Secret. A new Go program `code/vpn-picker` runs twice in every VPN pod: as an init container (latency-ranks candidates from gluetun's server list before the tunnel exists) and as a sidecar (switches gluetun through its control API, speed-tests the shortlist, monitors). `kubarr-common` renders both and opens gluetun's control API (network policy only).

**Tech Stack:** Rust (axum, kube-rs, sea-orm), React + vitest, Go 1.26, Helm, gluetun v3.41.3.

**Spec:** `docs/superpowers/specs/2026-11-07-vpn-server-picker-design.md` (Kubarr repo). The DNS part is already shipped (kubarr-charts `a50c577`, kubarr-common 0.8.3).

## Global Constraints

- gluetun pinned at `v3.41.3`; server list for that tag: `https://raw.githubusercontent.com/qdm12/gluetun/v3.41.3/internal/storage/servers.json` (verified 200, 7.4 MB).
- Switch body for gluetun: `PUT http://127.0.0.1:8001/v1/vpn/settings` with `{"provider":{"server_selection":{"hostnames":["<hostname>"]}}}`; status `GET /v1/vpn/status` → `{"status":"running"}`.
- Control API protection: network policy only. gluetun's default public role does **not** include `PUT /v1/vpn/settings`; set gluetun env `HTTP_CONTROL_SERVER_AUTH_DEFAULT_ROLE={"auth":"none"}` when the picker is enabled.
- Picker defaults: `shortlist: 5`, `sample: 40`, `initBudget: 60s`, `testSeconds: 8`, `interval: 15m`, `degradeThreshold: 0.4`, `degradeChecks: 3`, `cooldown: 6h`, `switchWhileRunning: true`, idle threshold 1 MB/s.
- A failed or slow picker never blocks the pod: init exits 0 on any error or after `initBudget`; the sidecar logs and keeps gluetun's current server.
- Picker container: non-root (UID 1000), read-only root, `automountServiceAccountToken: false`, drop ALL; only the init container adds `NET_RAW`.
- The picker reads only `VPN_SERVICE_PROVIDER`, `VPN_TYPE`, `SERVER_COUNTRIES`, `SERVER_CITIES`, `SERVER_HOSTNAMES` from the VPN Secret (`env.valueFrom`, `optional: true`), never `envFrom` (keeps the private key out).
- Proton's `servers.json` has no P2P flag; for every provider the "P2P" filter means `port_forward == true`.
- Port forwarding needs no new code: the qBittorrent chart already passes a default `VPN_PORT_FORWARDING_UP_COMMAND` that POSTs `listen_port` to qBittorrent; it fires on every reconnect, including picker switches. Enabling `vpn.portForwarding.enabled` stays the owner's choice.
- Library bumps follow repo convention: kubarr-common version + every consumer's dependency, lock and build suffix (see `a50c577`).

## Review Focus

1. Filters that match zero servers (typo'd country, provider gluetun doesn't list) → init writes no shortlist and exits 0; sidecar does nothing; gluetun connects as today. Test in Task 4 and Task 5.
2. gluetun never reaches `running` after a switch (dead server) → sidecar gives up on that candidate after a timeout and moves on; if all fail, returns to the round's first working server. Test in Task 5.
3. Server list download fails or is truncated at pod start → embedded fallback list is used. Test in Task 4.
4. Pod restarts repeatedly (crash loop elsewhere) → speed round runs at most once per `cooldown` per pod lifetime is fine, but must not hammer: the round is skipped if a shortlist result file younger than `cooldown` exists in the shared emptyDir. Test in Task 5.
5. VPN turned off for an app that had it → namespace labels removed, picker not rendered, gluetun env unchanged. Tests in Task 3 and Task 6.

---

### Task 1: Unblock Kubarr CI (trivy CVE in dns-webhook)

**Files:**
- Modify: `code/dns-webhook/go.mod`, `code/dns-webhook/go.sum`

- [ ] **Step 1:** `cd code/dns-webhook && go get golang.org/x/net@v0.62.0 && go mod tidy`
- [ ] **Step 2:** `go build ./... && go vet ./...` → no output, exit 0.
- [ ] **Step 3:** `grep 'golang.org/x/net' go.mod` → `v0.62.0`.
- [ ] **Step 4: Commit** `fix(deps): bump golang.org/x/net to 0.62.0 for CVE-2026-78661`

### Task 2: WireGuard server filters (backend + form)

**Files:**
- Modify: `code/api/src/services/vpn.rs` (`build_wireguard_secret_data` ~L1143, `build_openvpn_secret_data` ~L1184, tests module)
- Modify: `code/frontend/src/components/vpn/VpnProviderForm.tsx`, `VpnProviderForm.test.tsx`

**Interfaces:**
- Produces: `fn add_server_selection_secret_data(credentials: &serde_json::Value, data: &mut BTreeMap<String, String>)` used by both builders; credential keys `server_countries`, `server_cities`, `server_hostnames` (strings, comma-separated) → `SERVER_COUNTRIES`, `SERVER_CITIES`, `SERVER_HOSTNAMES`.

- [ ] **Step 1: Failing Rust test** `test_build_wireguard_secret_data_server_selection`: credentials `{"private_key":"k","addresses":["10.2.0.2/32"],"server_countries":"Netherlands,Belgium","server_hostnames":"nl-01.protonvpn.net"}` → data has `SERVER_COUNTRIES=Netherlands,Belgium`, `SERVER_HOSTNAMES=nl-01.protonvpn.net`, no `SERVER_CITIES`.
- [ ] **Step 2:** `cargo test --lib vpn::tests::test_build_wireguard_secret_data_server_selection` → FAIL.
- [ ] **Step 3:** Extract the OpenVPN "Server selection" block into `add_server_selection_secret_data` and call it from both builders.
- [ ] **Step 4:** `cargo test --lib vpn::` → all pass (existing OpenVPN selection tests unchanged).
- [ ] **Step 5: Failing vitest** in `VpnProviderForm.test.tsx`: "shows server filters for WireGuard with a named provider and submits them" — select provider `protonvpn`, type `wireguard`, fill private key, Server Countries `Netherlands`; submitted credentials include `server_countries: 'Netherlands'`; with provider `custom` the country field is absent.
- [ ] **Step 6:** `pnpm vitest run src/components/vpn/VpnProviderForm.test.tsx` → FAIL.
- [ ] **Step 7:** Render the countries/cities fields (reuse the OpenVPN inputs' labels "Server Countries", "Server Cities", plus "Server Hostnames") for WireGuard when `serviceProvider !== 'custom'`; include them in the WireGuard credentials object.
- [ ] **Step 8:** vitest passes; `pnpm lint` clean.
- [ ] **Step 9: Commit** `feat(vpn): server filters for WireGuard providers`

### Task 3: Pod Security level follows VPN

**Files:**
- Modify: `code/api/src/services/k8s.rs` (next to `ensure_namespace` L267)
- Modify: `code/api/src/services/deployment.rs:~360` (after `ensure_namespace`, non-system apps only)
- Test: `code/api/src/services/k8s_namespace_tests.rs`, `code/api/src/services/deployment_tests.rs`

**Interfaces:**
- Produces: `pub async fn set_namespace_pod_security(&self, namespace: &str, privileged: bool) -> Result<()>` — JSON merge patch on the Namespace: `privileged=true` sets `pod-security.kubernetes.io/{enforce,audit,warn}: privileged`; `false` sets those three keys to `null` (removal).
- Consumes: the VPN-enabled decision deployment.rs already makes when building `vpn.enabled` (`vpn_values_file.is_some()` path vs the `vpn.enabled=false` branch ~L470).

- [ ] **Step 1: Failing tests** in `k8s_namespace_tests.rs` with the existing scripted client: `privileged_vpn_namespace_gets_three_labels` asserts one `PATCH` with `Content-Type: application/merge-patch+json` and body `{"metadata":{"labels":{"pod-security.kubernetes.io/enforce":"privileged","pod-security.kubernetes.io/audit":"privileged","pod-security.kubernetes.io/warn":"privileged"}}}`; `disabling_vpn_removes_the_labels` asserts the same keys with `null`.
- [ ] **Step 2:** `cargo test --lib k8s_namespace` → FAIL.
- [ ] **Step 3:** Implement `set_namespace_pod_security`.
- [ ] **Step 4: Failing test** in `deployment_tests.rs`: `vpn_app_namespace_is_labelled_before_helm` — deploying `qbittorrent` with a VPN config records the namespace PATCH (privileged) before the helm invocation; without VPN records the removal PATCH; a system app records no PATCH.
- [ ] **Step 5:** Call `set_namespace_pod_security(namespace, vpn_enabled)` right after `ensure_namespace` in the non-system branch; compute `vpn_enabled` before the call (move the existing VPN lookup up if needed).
- [ ] **Step 6:** `cargo test --lib` all pass; `cargo fmt --check`; `cargo clippy --all-targets` clean.
- [ ] **Step 7: Commit** `fix(vpn): label app namespaces privileged while VPN is on`

### Task 4: vpn-picker — candidates and latency ranking

**Files:**
- Create: `code/vpn-picker/go.mod` (module `github.com/smokeythebandit/kubarr-vpn-picker`, go 1.26.0)
- Create: `code/vpn-picker/servers/servers.go`, `servers_test.go`, `servers/fallback.json.gz` (gzip of the pinned servers.json, `//go:embed`)
- Create: `code/vpn-picker/probe/probe.go`, `probe_test.go`

**Interfaces:**
- Produces:
  - `type Server struct { Hostname string; ServerName string; Country string; City string; IPs []netip.Addr; PortForward bool; SecureCore bool; Tor bool; Free bool; VPN string }`
  - `func Load(ctx context.Context, url string, client *http.Client) ([]byte, error)` — download with 20 s timeout, falls back to embedded on any error or invalid JSON.
  - `type Filter struct { Provider, VPNType string; Countries, Cities, Hostnames []string; P2P bool }`
  - `func Candidates(list []byte, f Filter) ([]Server, error)` — provider key lowercased; case-insensitive matches; excludes SecureCore/Tor/Free; `P2P` requires `PortForward`.
  - `func Sample(s []Server, n int, rnd *rand.Rand) []Server`
  - `type Pinger interface { RTT(ctx context.Context, ip netip.Addr) (time.Duration, error) }` with `ICMPPinger` (golang.org/x/net/icmp, privileged) and `TCPPinger{Port: 443}`; `func Rank(ctx, servers []Server, p Pinger, keep int) []Server` — 3 probes per server, median, drops unreachable, sorted ascending, parallelism 8.

- [ ] **Step 1: Failing tests** (`servers_test.go`) on a small fixture JSON with protonvpn + another provider: `TestCandidatesFiltersProviderTypeCountryP2P` (Netherlands + P2P → only NL port_forward wireguard entries), `TestCandidatesExcludesSecureCoreTorFree`, `TestCandidatesUnknownProviderReturnsEmptyNoError`, `TestLoadFallsBackOnHTTPErrorAndOnTruncatedJSON` (httptest server returning 500, then 200 with `{"protonvpn":`) → returns embedded bytes that parse.
- [ ] **Step 2:** `go test ./servers/...` → FAIL.
- [ ] **Step 3:** Implement; generate `fallback.json.gz` with `curl -sL <pinned url> | gzip -9 > servers/fallback.json.gz`.
- [ ] **Step 4:** `go test ./servers/...` PASS.
- [ ] **Step 5: Failing tests** (`probe_test.go`) with a fake Pinger: `TestRankSortsByMedianAndDropsUnreachable`, `TestRankKeepsN`, `TestRankRespectsContextDeadline` (slow fake + 50 ms ctx → returns what finished, no hang).
- [ ] **Step 6:** Implement `probe`; `go test ./... && go vet ./...` PASS.
- [ ] **Step 7: Commit** `feat(vpn-picker): candidate filtering and latency ranking`

### Task 5: vpn-picker — gluetun control, speed round, monitor, main

**Files:**
- Create: `code/vpn-picker/gluetun/client.go`, `client_test.go`
- Create: `code/vpn-picker/picker/round.go`, `monitor.go`, `picker_test.go`
- Create: `code/vpn-picker/main.go`, `config.go`, `config_test.go`
- Create: `docker/Dockerfile.vpn-picker` (multi-stage like `Dockerfile.dns-webhook`; final `gcr.io/distroless/static:nonroot`)
- Modify: `.github/workflows/build.yml` matrix (+ `component: vpn-picker`, `image: kubarr-vpn-picker`, `dockerfile: docker/Dockerfile.vpn-picker`), `.github/workflows/publish.yml` image list

**Interfaces:**
- Consumes: `servers.Server`, `probe.Rank` (Task 4).
- Produces:
  - `gluetun.Client{Base string; HTTP *http.Client}`: `Switch(ctx, hostname string) error` (PUT body per Global Constraints), `WaitRunning(ctx, timeout time.Duration) error` (polls `GET /v1/vpn/status` every 1 s until `"running"`).
  - `picker.Meter interface { Bytes() (uint64, error) }`; `TunMeter{Path: "/sys/class/net/tun0/statistics/rx_bytes"}`.
  - `picker.Tester interface { Run(ctx, d time.Duration) error }` — `HTTPTester{URLs []string}` downloads until `d` or 100 MB, tries URLs in order on error.
  - `func Round(ctx, c Switcher, short []servers.Server, m Meter, t Tester, cfg Config) (best servers.Server, results []Result, err error)` — per candidate: Switch, WaitRunning(30 s), measure `Δbytes/Δt` across `t.Run`; skip failures; switch to best; if none worked, switch back to `short[0]`.
  - `func Monitor(ctx, ...)` applying `degradeThreshold`, `degradeChecks`, `cooldown`, `switchWhileRunning`, idle 1 MB/s.
  - Shared files in `/picker` emptyDir: `shortlist.json` (written by init), `result.json` (`{"best":..., "mbps":..., "at": RFC3339}`), status served on `127.0.0.1:8002/status`.
  - `main`: `vpn-picker init` and `vpn-picker run`; config from env `PICKER_*` (names = values keys upper-snake, e.g. `PICKER_SHORTLIST`) plus the VPN Secret keys.

- [ ] **Step 1: Failing tests** `client_test.go` (httptest): `TestSwitchSendsHostnameBody` asserts method PUT, path `/v1/vpn/settings`, JSON body exactly per Global Constraints; `TestWaitRunningTimesOut`.
- [ ] **Step 2:** FAIL → implement → PASS.
- [ ] **Step 3: Failing tests** `picker_test.go` with fakes: `TestRoundPicksHighestThroughput`, `TestRoundSkipsServerThatNeverRuns` (Review Focus 2), `TestRoundAllFailReturnsToFirst`, `TestMonitorNeedsConsecutiveDegradedChecks`, `TestMonitorHonoursCooldown`, `TestMonitorWithoutSwitchWhileRunningWaitsForIdle`, `TestRoundSkippedWhenRecentResultExists` (Review Focus 4), `TestEmptyShortlistIsNoop` (Review Focus 1).
- [ ] **Step 4:** FAIL → implement → PASS.
- [ ] **Step 5: Failing test** `config_test.go`: defaults equal the Global Constraints values; invalid durations error.
- [ ] **Step 6:** Implement `config.go` and `main.go`. `init`: Load → Candidates → Sample → Rank(ICMP, fallback TCP) within `initBudget` → write shortlist; always exit 0. `run`: wait for gluetun running → Round (unless recent result) → Monitor.
- [ ] **Step 7:** `go test ./... && go vet ./...` PASS; `docker build -f docker/Dockerfile.vpn-picker .` succeeds; image < 25 MB.
- [ ] **Step 8:** Add CI matrix entries; `actionlint` clean if available.
- [ ] **Step 9: Commit** `feat(vpn-picker): speed round, monitor and image`

### Task 6: Charts — render the picker (kubarr-charts)

**Files:**
- Modify: `system/kubarr-common/templates/_gluetun.tpl`, `system/kubarr-common/Chart.yaml` (0.8.4)
- Modify: the 14 VPN-capable `*/templates/deployment.yaml` (pass `"p2p" true` in the 4 download clients' `gluetun.initContainer`/`container` includes)
- Modify: all kubarr-common consumers (dependency, lock, build bump)
- Test: `tests/test_vpn_picker.py`

**Interfaces:**
- Consumes: image `ghcr.io/smokeythebandit/kubarr-vpn-picker:<tag>` (Task 5), env `PICKER_*`.
- Produces: values (all read with `dig` defaults so releases upgraded with `--reuse-values` work): `vpn.picker.{enabled,shortlist,sample,initBudget,testSeconds,testEndpoints,interval,degradeThreshold,degradeChecks,cooldown,switchWhileRunning,image,resources}`.

- [ ] **Step 1: Failing tests** `tests/test_vpn_picker.py`: rendering qbittorrent with `vpn.enabled=true` yields init container `vpn-picker-init` (args `["init"]`, caps add `NET_RAW` only), sidecar `vpn-picker` (args `["run"]`, no added caps, read-only root, UID 1000), shared emptyDir `picker` mounted at `/picker` in both, gluetun env `HTTP_CONTROL_SERVER_AUTH_DEFAULT_ROLE={"auth":"none"}`, picker env has `VPN_SERVICE_PROVIDER` via `secretKeyRef` and no `envFrom`; `PICKER_P2P=true` for qbittorrent and `false` for prowlarr; `vpn.picker.enabled=false` renders none of it and no auth env; `vpn.enabled=false` renders nothing; `vpn.picker=null` (reuse-values) still renders defaults (Review Focus 5).
- [ ] **Step 2:** `python3 -m unittest test_vpn_picker` → FAIL.
- [ ] **Step 3:** Implement helpers `kubarr-common.gluetun.pickerInit`, extend `gluetun.container` (sidecar + auth env) and `gluetun.volumes`; include `pickerInit` in each VPN chart's initContainers before `gluetun.initContainer`.
- [ ] **Step 4:** Bump kubarr-common 0.8.4 and consumers; `helm dependency update --skip-refresh` each.
- [ ] **Step 5:** Full suite `python3 -m unittest discover -p 'test_*.py'` → OK (update version-pinned tests as in `a50c577`).
- [ ] **Step 6: Commit** `feat(vpn): server picker sidecar for VPN pods`

### Task 7: Ship and verify on the cluster

- [ ] **Step 1:** Push Kubarr branch, open PR, wait for CI (`check`, `build`) green; merge to `main` (owner approved). PR #188 rebases on top and merges too.
- [ ] **Step 2:** Confirm publish pushed `ghcr.io/smokeythebandit/kubarr-vpn-picker:<sha>` and new backend/frontend/worker; set the chart default `vpn.picker.image.tag` to that tag; push kubarr-charts to `main`; publish run green.
- [ ] **Step 3:** Roll out backend/frontend/worker images (`helm upgrade --reuse-values --set image.tag=<sha>` as done for HA) and qbittorrent (`--reset-then-reuse-values`).
- [ ] **Step 4:** Verify: namespace labels set by the backend (remove the manual label first and redeploy); `kubectl logs -c vpn-picker-init` shows a ranked shortlist; `-c vpn-picker` shows the round results and the chosen server; `/picker/result.json` exists; 100 MB test from the qbittorrent container beats the pre-change 49 MB/s or the log explains why; DNS lookup < 0.5 s.
- [ ] **Step 5:** Toggle VPN off and on through Kubarr for qbittorrent: labels removed/added, pod admitted both times.
