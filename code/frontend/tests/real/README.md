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
| `real-settings-profiles` | 2 | Manual DNS profile and disabled staging certificate profile create/edit/delete, and manual domain inventory CRUD. These only store configuration; no provider traffic, DNS changes or certificate issuance are asserted. |
| `real-settings-accounts` | 3 | Real registration rejected when closed; pending users denied login until approval, rejection, single-use invite redemption via the real API, user creation/deletion, disposable role audit permission enforcement and attributed audit events. The one-use code is never placed in a browser URL or native Playwright failure snapshot. |
| `real-app-install` | 1 | Install Sonarr through its catalog detail panel; wait for worker completion and healthy workloads; require the default metrics exporter with zero restarts |
| `real-app-upgrade` | 1 | Refresh the catalog and upgrade through the UI; verify chart B and stable workload health |
| `real-app-restart` | 1 | Use the permission-gated Restart control and verify the operation and workload health |
| `real-app-uninstall` | 1 | Uninstall through the UI; verify removal and that Install becomes available again |

The original 14-test lane passed locally with exit status 0 and successful resource
cleanup on 2026-09-22. The record is at
`tests/acceptance/validated-frontend-run-2026-09-22.md` in the application root.
The expanded 19-test lane and A/B application lifecycle passed in the owned
`settings-sol-thirteenth-20260923` disposable run on 2026-09-23; the harness
removed its cluster and registry. The previous attempted runs were stopped at
new-test failures and cleaned up, not counted as passes.
After moving one-use invite redemption out of browser URLs and native failure
snapshots, all 19 tests passed again in the owned
`settings-privacy-verify-260923` run, including Sonarr install, upgrade,
restart and uninstall. Its scoped cluster, registry and work directory were
removed. This invitation journey uses the real API; the strictly mocked
browser lane separately checks the invite-creation modal.

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
- Traces, video, and automatic screenshots are disabled because configuration
  forms handle credentials. The fixture attaches an explicit screenshot only
  from a form-free Domains inventory, and attaches sanitized counts/path
  evidence on failure; no response bodies, credentials, queries, or network traces.
- JSON per phase and the static test ledger are generated for CI artifacts; see
  `tests/report/README.md` for local use and inventory semantics.

`pnpm test:acceptance` / `pnpm test:real` expose the config, but the seven projects
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
catalog application's behavior, all non-admin permissions, OAuth/2FA,
external notification delivery, or network-policy enforcement. The separate API
acceptance scenario retains the in-flight worker shutdown/drain check.
