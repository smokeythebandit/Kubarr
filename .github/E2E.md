# Release E2E Deployment

`ci.yml` runs `fast-test.yml` on every PR and push before building images. After
the build, it calls `test.yml` for `v*` tags only. The latter contains live E2E and
diagnostic backend library coverage; coverage does not run the unvetted integration
targets. Release publishing requires successful live E2E and the separate
`acceptance.yml` real application-lifecycle job and the controlled
`vpn-acceptance.yml` WireGuard lab. Non-tag publishing
requires the checks, fast tests, and build, but does not run live E2E.

The E2E job builds the repository's CLI and uses `kubarr bootstrap` rather than
an umbrella chart. It installs managed NFS (5Gi on Kind's `standard` storage
class), PostgreSQL, Fluent Bit, VictoriaLogs, VictoriaMetrics, backend, frontend,
OpenResty, and worker. Managed NFS requires the hosted runner's `nfs` and `nfsd`
kernel modules. Helm releases live in `default`; workloads have their own
namespaces, as defined by the installer.

Tool versions are Helm 4.3.0, Kind 0.33.0, and Kubernetes/kubectl 1.35.8. The
Kind node image is digest-pinned. Helm 4.3 supports Kubernetes 1.34-1.37; keep
the node and kubectl versions aligned when upgrading Helm.

Kubarr explicitly uses `--server-side=false` and `--wait=legacy` to retain the
existing client-side apply and readiness behavior during the Helm major upgrade.
Rollback uses Helm 4's `--rollback-on-failure` flag. The installer requires Helm 4
before making cluster changes. Existing Helm 3 release records remain usable.

`CHARTS_REV` pins the validated bootstrap chart source. `CATALOG_REV` separately
pins runtime discovery to the upstream-aligned versions published in GHCR, rather
than requesting the obsolete pre-alignment versions. Local `kubarr-common` dependencies are built
in the categorized checkout before exposing flat symlinks to the CLI. When
updating the revision, also check the preloaded third-party image tags against
the rendered charts. Application artifacts and image names must match
`build.yml`: `kubarr-backend`, `kubarr-frontend`, and `kubarr-worker`.

Bootstrap seeds the disposable `admin` / `adminadmin` test account and validated
storage records after API migrations. `auth.setup.ts` only logs in; it does not
create an account. The API and worker then roll out with chart discovery pinned
to `CATALOG_REV`. Before Playwright starts, CI checks API health, the login page,
and the catalog entries used by the app tests through
`openresty/svc/kubarr-gateway` on port 8080.

The job uses its own kubeconfig, unique Kind cluster, and tracked port-forward
process. Cleanup must stay scoped to those resources, not other clusters or
Docker containers on the machine.

For checks without a cluster, use `actionlint .github/workflows/test.yml`, build
`code/cli` with `cargo build --locked`, and inspect `kubarr bootstrap --dry-run`
with the workflow's arguments/environment. In a disposable copy of the pinned
charts, run the dependency preparation and render the dry-run's Helm commands
with `helm template` (omit install-only `--server-side=false` and `--wait=legacy`).
Check image names, workload/service selectors, claims,
secrets, and gateway ports. `pnpm exec playwright test --list` in `code/frontend`
checks discovery without launching browsers. These checks do not prove that
NFS, image pulls, or browser tests work on a live runner.

The runtime catalog still downloads versioned charts from GHCR, and third-party
images use tags rather than digests. External service availability and mutable
registry tags are not eliminated by the source revision pin. TLS/DNS provisioning
is not bootstrapped here. The separate legacy `e2e-tests.yml` workflow is not
called by `ci.yml` and is not maintained by this repair.

## Coordinated Chart Update

The current pin includes OpenResty chart `0.2.5`, which fixes unknown frontend
routes returning HTTP 500 instead of the SPA's 404 page, worker chart `0.2.4`
with singleton replacement and shutdown grace, and monitoring startup fixes. Push the chart commit
before running this workflow, and publish its versioned application charts to
GHCR before the catalog readiness check can succeed. A source commit being
available does not establish that its chart packages have been published.
