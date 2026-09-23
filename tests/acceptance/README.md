# Kubernetes acceptance test

This test creates a uniquely named Kind cluster and OCI registry, bootstraps Kubarr through the CLI, and exercises the real Sonarr lifecycle. It is intentionally opt-in and never uses the default kubeconfig.

The default scenario drives lifecycle mutations through the API and proves worker
draining during an in-flight update. Setting `KUBARR_ACCEPTANCE_FRONTEND=1` instead
drives settings, VPN provider CRUD, install, update, restart, and uninstall through
the real frontend at the gateway. The frontend scenario intentionally does not
repeat the worker-drain assertion; CI runs both scenarios and the API scenario
remains the required drain gate.

## Scope and validation status

The complete lifecycle passed on 2026-09-20, including scoped cleanup and exit
status 0. See [the recorded run](validated-run-2026-09-20.md).

The real frontend scenario also passed all 14 browser tests with scoped cleanup
and exit status 0 on 2026-09-22. See
[the frontend run](validated-frontend-run-2026-09-22.md).

Coverage is a chart-only A-to-B upgrade using the same Sonarr image, followed by
restart and uninstall persistence checks. This is not application-binary migration,
platform upgrade, abrupt node-loss recovery, or CNI-enforcement coverage.

The upgrade includes a required live worker-drain assertion. Chart B adds a
test-only 15-second BusyBox init-container delay to provide a controlled in-flight
window. The harness observes the update as `running`, restarts the singleton
worker, and requires its logs to show SIGTERM handling before completion of that
exact operation. It also waits for the replacement to become ready and the old UID
to disappear. Early success, missing evidence, and reversed event order fail rather
than skipping the drain check.

## Prerequisites

- Linux Docker host with at least 4 CPUs, 6 GiB free RAM, and 10 GiB free disk
- Host NFS client/server kernel support already loaded (`nfs` and `nfsd`); the script never runs `sudo` or loads modules
- Docker Engine 28 or newer (API 1.48 or newer), Kind `v0.33.0`, kubectl compatible with Kubernetes `v1.35.8` (CI pins `v1.35.8`), Helm `v4.3.0`, `curl`, `jq`, `git`, and Rust/Cargo
- Local images `kubarr-backend:<tag>`, `kubarr-frontend:<tag>`, and `kubarr-worker:<tag>`
- A charts checkout at `01e45468f233b03b4a7d70a320aec9075f77c213`

Build the CLI and current images, then run from the application repository:

```bash
cargo build --locked --manifest-path code/cli/Cargo.toml
docker build -f docker/Dockerfile.api -t kubarr-backend:acceptance-local .
docker build -f docker/Dockerfile.frontend -t kubarr-frontend:acceptance-local .
docker build -f docker/Dockerfile.worker -t kubarr-worker:acceptance-local .

KUBARR_ACCEPTANCE_DISPOSABLE=1 \
KUBARR_ACCEPTANCE_IMAGE_TAG=acceptance-local \
KUBARR_ACCEPTANCE_CHARTS_DIR="$PWD/../kubarr-charts" \
tests/acceptance/app-lifecycle.sh
```

For the real frontend scenario, install the pinned frontend dependencies and
Chromium first, then add `KUBARR_ACCEPTANCE_FRONTEND=1`. The harness supplies the
gateway URL, disposable bootstrap-admin credentials, run ID, chart versions, and a
private mode-0600 result path to these serial Playwright projects:
`real-settings-vpn`, `real-app-install`, `real-app-upgrade`, `real-app-restart`, and
`real-app-uninstall`. Playwright starts no development server and must not mock
routes. Each app project writes its operation ID to the shared JSON result as
`install_id`, `update_id`, `restart_id`, or `delete_id`; the shell harness then
performs the existing API and Kubernetes validation.

The charts path defaults to the sibling `../kubarr-charts`, but the commit is always verified. The harness uses `git archive` at that revision, so dirty or unpushed files cannot affect generated artifacts. CI supplies an explicit checkout path.

Keep the harness unchanged during execution, or run an immutable snapshot with
explicit `KUBARR_ACCEPTANCE_APP_ROOT`, `KUBARR_ACCEPTANCE_CLI`, and
`KUBARR_ACCEPTANCE_CHARTS_DIR` paths. Bash can
fail when its script is edited mid-run. Local snapshots, private work directories,
and logs can live beneath `.acceptance-runs/`, which is excluded from Git and
Docker build contexts and survives `/tmp` cleanup.

The runtime images must include support for `KUBARR_CHARTS_SOURCE_DIR` and `KUBARR_CHARTS_PLAIN_HTTP`. The source variable is present on the first backend and worker pods. Before the ConfigMap is mounted, their initial sync fails against the absent local directory rather than attempting GitHub; the harness then mounts the projected source and rolls both Deployments before the explicit sync.

Optional controls include `KUBARR_ACCEPTANCE_KIND_IMAGE`, `KUBARR_ACCEPTANCE_GATEWAY_PORT`, `KUBARR_ACCEPTANCE_INSTALL_TIMEOUT` (default `5m`), `KUBARR_ACCEPTANCE_APP_TIMEOUT` (default `10m`), `KUBARR_ACCEPTANCE_WORK_DIR`, and `KUBARR_ACCEPTANCE_FRONTEND`. The API scenario's live drain check has no skip flag. The update must be observed running within 60 seconds; operation polling is bounded at 11 minutes and the worker rollout at 12 minutes, with readiness and old-UID checks bounded separately. The harness also verifies that the installed worker Deployment is a singleton using the `Recreate` strategy and a 660-second termination grace period.

The harness pre-pulls every pinned bootstrap/Sonarr image for the Docker host's native `linux/amd64` or `linux/arm64` platform, including `busybox:1.37.0` used by the acceptance chart's init container. The frontend scenario also preloads `ghcr.io/onedr0p/exportarr:v2.3.0` because UI installs use the chart defaults; the API scenario explicitly disables that exporter. It exports each dependency and local Kubarr image with `docker image save --platform` before loading the archive into Kind; that option requires Docker API 1.48 and the harness intentionally has no less-safe fallback. The disposable admin password and Sonarr-generated API key exist only in process-local variables/files under the mode-0700 work directory. Failure output excludes raw workload logs, Secrets, and ConfigMaps. The exit trap removes only the exact cluster, registry container, port-forward, and work directory created by that invocation.

The application persistence check changes Sonarr's instance name through the real host-config API and verifies it after upgrade and restart. Sonarr 4 requires an instance name to start or end with `Sonarr`, so the test uses `Kubarr Acceptance Sonarr`. The chart's `External` authentication method and `DisabledForLocalAddresses` requirement are represented by the API as `external` and `disabledForLocalAddresses`; a full object returned by `GET /api/v3/config/host` remains valid for `PUT` with those values. Sonarr API failures report only the HTTP method/path/status and structured validation fields, never the host-config response or authentication material.

Run offline structural checks with:

```bash
tests/acceptance/test-helpers.sh
tests/acceptance/test-sonarr-api.sh
```

The chart-fixture helper tests also require Helm, PyYAML 6.0.3, and the chart
checkout. CI explicitly supplies those prerequisites. These helper checks are not
a substitute for the live acceptance run.
