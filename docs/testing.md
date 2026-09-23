# Testing

## Required Fast Lane

`.github/workflows/fast-test.yml` runs on every PR and push, independently of
static checks and before image builds. A failing backend, CLI, or frontend job
blocks the build and publication. Repository branch-protection settings are
managed separately; require these checks there to block merging as well.

From `code/api`:

```sh
cargo test --locked --lib
cargo test --locked --test app_operations_tests
cargo test --locked --test apps_endpoint_tests queues_without_k8s
```

The new worker tests use real migrated SQLite databases and fake only external
execution. They check queue claims, attempts, timestamps, terminal status/errors,
desired-state linkage, competing claims, and older completions arriving after a
new request. The separate-connection SQLite race test is not proof of PostgreSQL
concurrency behavior.

`app_operations_tests` exercises actual routes and permission extractors with an
explicit user extension. The selected `apps_endpoint_tests` additionally log in
through the real session endpoints before enqueueing work. The fixture catalogs
are explicit; no chart downloads, Helm subprocess, or worker loop is needed.

Deployment tests run actual command construction against an injected executor
and scripted Kubernetes transport. They verify Helm 4 flags, namespace ownership,
namespace visibility before deployment, VPN credential placement, CIDR serialization,
and temporary-file cleanup on success and failure. Namespace timeout tests use
paused time rather than wall-clock sleeps.

From `code/cli`:

```sh
cargo test --locked --bin kubarr
```

From `code/frontend` with Node 26 and the pinned pnpm version:

```sh
pnpm test
pnpm test:typecheck
pnpm run build
pnpm exec playwright install --with-deps chromium
pnpm test:browser
```

The deterministic browser lane starts local Vite and mocks API responses. It does
not use `BASE_URL`, live sessions, or a running backend. Unexpected API/external
requests and uncaught browser exceptions fail the tests. Tests cover current VPN,
notification, Domains, and storage UI behavior with fresh state per test and no retries.
See `code/frontend/tests/README.md` for fixtures, scenarios, and remaining gaps.

## Live And External Tests

### Controlled VPN acceptance

`tests/acceptance/vpn/` creates a real WireGuard server and two controlled HTTP
endpoints on isolated Docker networks. The production Sonarr/Gluetun integration
is configured through the browser in a separate disposable cluster. The suite
checks handshake/counters, source identity, three server-outage rounds, route-loss
kill-switch behavior, recovery, and UI removal. Independent endpoint logs check
for escaped requests while an unprotected control verifies reachability.

This lane passed locally with exit status 0 and complete cleanup on 2026-09-23.
See `tests/acceptance/vpn/validated-run-2026-09-23.md`. It protects the existing
`kubarr-local` environment and does not use commercial credentials. Coverage is
explicitly IPv4 HTTP, not IPv6/DNS or universal VPN leak protection.

The dedicated `vpn-acceptance.yml` workflow can run manually and is required for
tagged publication. Its kernel-module and inotify setup applies only to ephemeral
CI runners; local host changes require explicit operator action.

### Real application lifecycle acceptance

`.github/workflows/acceptance.yml` runs `tests/acceptance/app-lifecycle.sh`.
Manual dispatch builds current images; release-tag CI passes its existing image
tag and reuses the build artifacts without uploading duplicate artifacts.
Tagged publication requires both this acceptance job and the separate legacy
browser release suite to succeed. Pull requests retain the fast test lane.

Acceptance has independent `api` and `frontend` matrix jobs. The frontend job uses
Chromium against the real gateway for 14 UI-driven settings, VPN configuration,
and app lifecycle scenarios. It does not mock API responses. App operations must
finish in the worker and workloads must become healthy; reload/read-only API
checks verify settings persistence. VPN tests use unassigned disposable providers
and do not claim tunnel connectivity. See `code/frontend/tests/real/README.md` and
`tests/acceptance/validated-frontend-run-2026-09-22.md` for the passing local run.

The acceptance harness uses an isolated Kind kubeconfig, real CLI bootstrap,
managed NFS, a local OCI registry, and actual Sonarr workloads. It installs through
Kubarr's API/worker, checks Sonarr's API through the authenticated gateway, changes
a real application setting, writes an NFS sentinel, upgrades between two chart
versions, replaces the worker during a running upgrade, restarts Sonarr, and
uninstalls it. The old worker's logs must show the shutdown signal before completion
of that exact operation; observing only a ready replacement is insufficient.
Version/revision, pod replacement,
application configuration, and exact sentinel contents are asserted. Both chart
versions deliberately use the same application image; this does not test Sonarr
binary/database migration compatibility or a previous-to-current Kubarr upgrade.

See `tests/acceptance/README.md` for build commands and the explicit disposable-run
opt-in. Host NFS modules must already be loaded locally; CI loads them explicitly.
The offline helper checks run on PRs but do not count as a successful deployment.

Runtime catalog injection uses:

| Variable | Purpose |
| --- | --- |
| `KUBARR_CHARTS_SOURCE_DIR` | Optional read-only chart metadata tree replacing GitHub discovery. ConfigMap-projected metadata symlinks are supported only within that tree. |
| `KUBARR_CHARTS_REGISTRY` | OCI source for real chart pulls and installs. |
| `KUBARR_CHARTS_PLAIN_HTTP` | Explicit `true` to allow a disposable HTTP registry; defaults to false. Never enable for untrusted networks. |
| `KUBARR_CHARTS_DIR` | Downloaded chart cache, distinct from the metadata source. |

Catalog sync serializes concurrent requests within a process and reports pull
failures rather than recording a successful sync. Registry images and third-party
application images still need to be downloaded before execution; the suite is not
fully offline. Default Kind networking does not prove NetworkPolicy enforcement.

## Audit regression coverage

Audit contract tests are in `code/api/tests/{auth_audit_tests,admin_audit_tests,
vpn_audit_tests,app_audit_tests,audit_contract_tests}.rs`; the frontend audit UI
tests are in `code/frontend/src/components/settings/tabs/AuditTab.test.tsx` and
`code/frontend/src/api/__tests__/security.test.ts`. They cover emitted successful
and failed logins, logout and session revocation; user, role, invite and settings
mutations; VPN provider and assignment changes; app-operation requests and worker
terminal outcomes written through the transactional audit outbox. Audit reads
require `audit.view`; manual clearing requires `audit.manage` and records the
attributed clear in the same transaction. Detail fields use a sensitive-key
allowlist/redaction policy and bounded values; this is not a guarantee against
secrets embedded in arbitrary free-form text.

The UI is in **Security** and displays canonical snake_case action names. Audit
event totals are event counts, not 2FA enrollment/adoption statistics; only 2FA
enable/disable mutations are audited here, not OAuth sign-in, 2FA verification,
or invite use. Recorded peer IP is the API's immediate connection peer (often the
gateway), not necessarily the browser address. App-access events represent a
requested access, not proof the app rendered; the backend cannot verify rendered
content. If a worker disappears during an operation, recovery marks it
indeterminate after 15 minutes and does not rerun it. These logs therefore do
not claim complete coverage or prove every action's external effect.

Retention runs every 24 hours (no immediate first cleanup), defaults to 90 days,
and accepts `KUBARR_AUDIT_RETENTION_DAYS` from 1 through 3650; invalid values fall
back to 90 days. Automatic cleanup and its system event commit together. Manual
clear is likewise audited transactionally.

Do not test or deploy against the running `kubarr-local` instance for this change:
it uses an older image. Updated code requires a separate rebuild and redeploy.

The release-only workflow remains separate because it needs images, a disposable
Kind cluster, NFS kernel support, and published chart artifacts. See
`.github/E2E.md` for its prerequisites and the outstanding chart publication/pin
coordination. Never run the legacy live browser suite against a user's cluster;
some remaining tests mutate shared accounts and resources.

Do not promote arbitrary API integration targets into CI without auditing them:
some older tests call external registries, mutate process-global configuration,
or use an inherited `DATABASE_URL` for destructive migration checks. Cargo's
`--offline` flag only disables dependency downloads, not network calls made by tests.
The existing library suite also contains legacy provider/configuration tests;
the new contract tests do not certify the entire historical suite as hermetic.

The next priorities are isolated PostgreSQL migration/claim tests, recovery after
abrupt worker/node loss (as distinct from tested graceful shutdown), isolated live
2FA scenarios, application-binary/platform upgrade pairs, and NetworkPolicy
enforcement checks. Fast mocks cannot establish cluster admission,
VPN connectivity, provider delivery, DNS/TLS, or network isolation.
