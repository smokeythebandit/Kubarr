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

The next priorities are isolated PostgreSQL migration/claim tests, interrupted
worker recovery, isolated live storage/2FA scenarios, and real gateway
and NetworkPolicy enforcement checks. Fast mocks cannot establish cluster admission,
VPN connectivity, provider delivery, DNS/TLS, or network isolation.
