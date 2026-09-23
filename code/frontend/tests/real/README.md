# Real frontend acceptance

These tests use Chromium against the **real gateway, API, database, worker, and
Kubernetes workloads**. They do not mock requests or run Vite. Every test signs in
through the UI with a fresh browser context. Mutations under test happen through
visible controls; GET requests validate persistence and operation results.

Run through the disposable-cluster harness from the application repository:

```sh
KUBARR_ACCEPTANCE_DISPOSABLE=1 \
KUBARR_ACCEPTANCE_FRONTEND=1 \
KUBARR_ACCEPTANCE_IMAGE_TAG=acceptance-local \
KUBARR_ACCEPTANCE_CHARTS_DIR="$PWD/../kubarr-charts" \
tests/acceptance/app-lifecycle.sh
```

Build the three runtime images and CLI and install Chromium first, as described
in `tests/acceptance/README.md`. Never target a personal or production deployment.
The harness supplies credentials, gateway URL, chart versions, a unique run ID,
and a private operation-result file. It switches chart fixtures between browser
phases and validates real pod replacement, Helm metadata, and NFS persistence.

## Scenarios

| Project | Tests | Coverage |
| --- | ---: | --- |
| `real-settings-vpn` | 10 | Registration/approval settings; dark/light themes; notification event enablement/severity; WireGuard/OpenVPN creation, edits, reload persistence, required-field validation, deletion cancellation/confirmation, and credential redaction |
| `real-app-install` | 1 | Install Sonarr through its catalog detail panel; wait for worker completion and healthy workloads; require the default metrics exporter with zero restarts |
| `real-app-upgrade` | 1 | Refresh the catalog and upgrade through the UI; verify chart B and stable workload health |
| `real-app-restart` | 1 | Use the permission-gated Restart control and verify the operation and workload health |
| `real-app-uninstall` | 1 | Uninstall through the UI; verify removal and that Install becomes available again |

The full 14-test lane passed locally with exit status 0 and successful resource
cleanup on 2026-09-22. The record is at
`tests/acceptance/validated-frontend-run-2026-09-22.md` in the application root.

## Isolation and reporting

- One worker, no retries, fail fast on the first failure.
- Explicit destructive-test opt-in and an explicit loopback gateway URL.
- Global settings are restored; VPN providers have run-specific names and are
  deleted through the UI. Failure cleanup discards the whole test cluster.
- Browser errors and unexpected HTTP failures fail tests. Only optional catalog
  icon GET 404s and their matching browser resource messages are exempted, because
  `AppIcon` intentionally renders a fallback. This rule has unit tests.
- Reports are separated by phase under `playwright-report/real/` and
  `test-results/real/`, so later phases do not overwrite earlier reports.
- Traces, video, and screenshots are disabled because configuration forms handle
  credentials. Only generated, disposable test credentials are used.

`pnpm test:acceptance` / `pnpm test:real` expose the config, but the five projects
must be orchestrated by the harness to perform the A/B catalog handoff correctly.
They are separate from `pnpm test:browser`, the fast mocked-API lane.

## Boundaries

VPN coverage is **configuration-only**. Tests do not assign a provider to an app,
click Test Connection, establish a tunnel, or demonstrate kill-switch/leak
protection. The separate `tests/acceptance/vpn/` lane now exercises a controlled
WireGuard server for routing, outage blocking, recovery, and removal. Credential edits currently
re-enter keys/passwords; metadata-only editing without re-entry is not covered.

The app upgrade uses the same Sonarr application image and different chart
versions. This lane does not establish binary migration compatibility, every
catalog application's behavior, non-admin browser permissions, OAuth/2FA,
external notification delivery, or network-policy enforcement. The separate API
acceptance scenario retains the in-flight worker shutdown/drain check.
