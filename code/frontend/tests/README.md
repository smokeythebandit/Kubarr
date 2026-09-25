# Frontend Test Lanes

## Deterministic Browser Tests

Run from `code/frontend` with the repository's supported Node 26 and pnpm:

```sh
pnpm exec playwright install --with-deps chromium
pnpm test:typecheck
pnpm test
pnpm test:browser
```

CI integration command: `pnpm test:browser`. Do not start an API, cluster, or
separate frontend server for this job. `playwright.browser.config.ts` starts and
stops local Vite at `http://127.0.0.1:4174`, requires that port to be free, and
never reuses an existing server. Its Vite config removes the API/auth proxies;
`VITE_API_URL` is forced to `/api`. `BASE_URL` and live auth files are not used.
Browser binaries/dependencies must be installed before the job. Failure traces
and screenshots go to `test-results/browser`.

`tests/browser/fixtures.ts` installs context-wide routes before navigation.
Every test gets fresh browser storage, an explicitly mocked administrator,
fresh route handlers and local mock state. Method, path, and query string must
match exactly. Unregistered API requests, external HTTP requests, non-Vite
WebSockets, and uncaught browser errors fail the test, even when the application
catches a request failure. Service workers are blocked. Mutations are allowed
only by the test that registers them; none reaches a backend. Import `test` and
`expect` from this fixture in every deterministic spec, not from Playwright.
Do not use `request`/`route.fetch` to contact real services in this lane.

The 44 Chromium tests cover behavioral scenarios (including strict mocked
General registration/approval persistence, account administration and manual DNS/certificate profile CRUD):

| Replacement | Coverage |
| --- | --- |
| `browser/07-storage.spec.ts` (10) | Folder/parent/breadcrumb/thumbnail navigation, scoped creation and validation/cancel/error handling, rename/delete confirmation, exact editor save and failed-save retention, viewer read-only controls, lazy statistics and refresh, changed inventory, offline instructions |
| `browser/14-vpn.spec.ts` (9) | Empty/disabled states, navigation, required fields/cancel, WireGuard and OpenVPN creation payloads, rejected creation, seeded edit/delete confirmation, connection success/failure, app assignment/removal, retry/refresh |
| `browser/15-notifications.spec.ts` (8) | Empty/seeded inbox, outside-click dismissal, preferences link, individual/all read and delete, channel configuration/enablement, supported channels, event enablement/severity persistence, test destination validation and delivery success/failure |
| `browser/17-domains.spec.ts` (7) | Current Domains navigation/inventory, required and wildcard validation, cancel, create/edit/delete confirmation, rejected save, path/subdomain/exact-host app URLs |
| `browser/18-settings-access.spec.ts` (1) | Registration and approval toggle payloads and reload persistence |
| `browser/19-settings-profiles.spec.ts` (2) | Manual DNS profile CRUD and staging certificate profile create/delete with strict request matching |
| `browser/20-registration.spec.ts` (2) | Public registration disabled response and invite payload through the gateway-compatible login route |
| `browser/21-settings-admin.spec.ts` (5) | Pending approval/rejection, invite create/delete, role permission matrix, user CRUD/role assignment and attributed audit filtering |

The old top-level `07-storage`, `14-vpn`, `15-notifications`, and `17-cloudflare` specs were
replaced rather than left as destructive shared-state tests. Cloudflare token
wizard/deployment/status scenarios describe a removed UI; they are not skipped
or represented as tested features. Current Domains inventory and app URL flows
replace that suite. DNS-provider and certificate-profile management are separate
panes. Their new real configuration-only journeys have not been run in a cluster yet.

## Live Tests

### Real disposable-cluster frontend acceptance

`playwright.real.config.ts` and `tests/real/` now provide a separate, real-backend
lane: 19 settings, VPN-configuration, profile, accounts and app-lifecycle scenarios.
All 19 passed in a fresh-image disposable run on 2026-09-23 with scoped cleanup.
It is driven by
`KUBARR_ACCEPTANCE_FRONTEND=1 tests/acceptance/app-lifecycle.sh` from the application
root with the other disposable-run prerequisites. See `tests/real/README.md` for
coverage, safety, and limitations. It does not use the mocked browser fixtures.

### Legacy live suite

`playwright.config.ts` still owns the live `auth` and `chromium` projects and
explicitly ignores `tests/browser` and `tests/real`. The nonexistent bootstrap setup dependency
was removed. Discovery is safe and does not execute login or contact a target:

```sh
pnpm exec playwright test --config playwright.config.ts --list
pnpm test:browser --list
```

The live settings navigation now expects Domains, not Cloudflare Tunnel. OAuth
login and callback-error groups use empty storage; admin configuration and linked
accounts keep authenticated storage. Genuine live scenarios otherwise remain in
place. Do not run the full live suite on a user's cluster: legacy tests still
mutate shared data and are not an isolation-safe CI lane.

## Remaining Gaps

- Storage browser tests use mock inventories and do not establish NFS persistence
  or server-side path/permission enforcement. A real filesystem smoke test still
  needs unique resources and cleanup in a disposable environment.
- Other legacy suites (including OAuth provider configuration and 2FA) retain
  environment-dependent assertions/skips. The OAuth storage fix does not claim
  provider enablement, external redirects, or callback backend correctness.
- VPN editing currently requires credentials even though the form advertises
  `(unchanged)`. The seeded edit test explicitly replaces credentials; editing
  without credentials remains a product gap, not a silently skipped scenario.
- Mocked responses test frontend behavior, not API contracts, actual VPN
  connectivity, delivery, DNS, certificates, or deployment. This lane currently
  runs desktop Chromium, not a mobile or cross-browser matrix.
- `test:typecheck` checks all Playwright tests/configs and imported frontend
  types. It keeps strict type checking but permits unused locals already present
  in legacy specs; it is not a typecheck of Vitest source tests.

Local verification can invoke installed tools directly if pnpm rejects an older
Node runtime. Node 22.23.1 successfully ran Vite 8, Playwright, TypeScript, and
Vitest here without changing engine requirements or dependencies:

```sh
node node_modules/vitest/vitest.mjs run
node node_modules/typescript/bin/tsc --noEmit
node node_modules/typescript/bin/tsc --project tsconfig.tests.json
node node_modules/@playwright/test/cli.js test --config playwright.browser.config.ts
node node_modules/@playwright/test/cli.js test --config playwright.config.ts --list
```
