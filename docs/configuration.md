# Configuration Reference

Kubarr is installed as separate Helm releases. Configure initial dashboard
access, storage, and the administrator through the [CLI wizard](https://github.com/smokeythebandit/kubarr-cli/blob/main/docs/bootstrap.md).
Each chart's `values.yaml` is the reference for that component; there is no
umbrella chart with shared `backend:` and `frontend:` sections.

## Environment Variables

These defaults come from `code/api/src/application/config/` and apply to a
standalone process. Helm charts can override them.

| Variable | Purpose | Process default |
|----------|---------|-----------------|
| `KUBARR_API_HOST` | API bind address | `0.0.0.0` |
| `KUBARR_API_PORT` | API HTTP port | `8000` |
| `KUBARR_DATABASE_URL` | SeaORM database URL | Falls back to `DATABASE_URL`, then `postgres://kubarr:kubarr@localhost:5432/kubarr` |
| `KUBARR_IN_CLUSTER` | Use in-cluster Kubernetes access | `false`; platform charts set `true` |
| `KUBARR_KUBECONFIG_PATH` | Explicit kubeconfig path | Unset |
| `KUBARR_DEFAULT_NAMESPACE` | Default Kubernetes namespace | `media` |
| `KUBARR_LOG_LEVEL` | Log filter level | `info` |
| `KUBARR_OAUTH2_ENABLED` | OAuth2 configuration flag | `false` |
| `KUBARR_OAUTH2_ISSUER_URL` | OAuth2 issuer URL | `http://kubarr:8000/auth` |
| `KUBARR_CHARTS_DIR` | Local chart cache | `/tmp/charts` |
| `KUBARR_CHARTS_SOURCE_DIR` | Optional local chart source directory | Unset |
| `KUBARR_CHARTS_REPO` | GitHub chart repository | `smokeythebandit/kubarr-charts` |
| `KUBARR_CHARTS_GIT_REF` | Chart source revision | `main` |
| `KUBARR_CHARTS_REGISTRY` | OCI chart registry | `oci://ghcr.io/smokeythebandit/kubarr-charts` |
| `KUBARR_CHARTS_SYNC_INTERVAL` | Chart refresh interval, seconds | `3600` |
| `KUBARR_CHARTS_PLAIN_HTTP` | Explicit insecure registry transport opt-in | `false` |

In-cluster startup first attempts to load database credentials from the
`kubarr-database` namespace; the configured database URL is the fallback.
The API and worker must connect to the same database.

### Worker Intervals

| Variable | Default, seconds |
|----------|------------------|
| `KUBARR_WORKER_POLL_INTERVAL_SECONDS` | `5` |
| `KUBARR_WORKER_RECONCILE_INTERVAL_SECONDS` | `30` |
| `KUBARR_DOMAIN_RECONCILE_INTERVAL_SECONDS` | `60` |

### Local Frontend Proxy

`KUBARR_DEV_API_URL` configures Vite's development proxy for `/api` and `/auth`.
It defaults to `http://127.0.0.1:8000`, matching the API's default port:

```sh
KUBARR_DEV_API_URL=http://127.0.0.1:8080 pnpm dev
```

This is a development-server setting, not a production API URL. Production
routing is handled by the gateway.

## Helm Chart Values

The platform charts live in [kubarr-charts/system](https://github.com/smokeythebandit/kubarr-charts/tree/main/system):

| Chart | Default workload namespace | Role |
|-------|----------------------------|------|
| `kubarr-backend` | `kubarr-backend` | API |
| `kubarr-worker` | `kubarr-worker` | Lifecycle worker |
| `kubarr-frontend` | `kubarr-frontend` | Dashboard assets |
| `openresty` | `openresty` | Public gateway |
| `postgresql` | `kubarr-database` | Application database |
| `managed-nfs` | `kubarr-storage` | Optional managed shared storage |

Helm release namespaces can differ from workload namespaces. Inspect
`helm list -A` before changing an installed release. Use the installed chart's
version and values as the starting point for upgrades; the CLI pins compatible
bootstrap chart revisions in `src/charts.rs`.

### API Image and Resources

For the `kubarr-backend` chart, values are top-level:

```yaml
image:
  repository: ghcr.io/smokeythebandit/kubarr-backend
  tag: "0.3.1"
  pullPolicy: IfNotPresent
service:
  type: ClusterIP
  port: 8000
  targetPort: 8000
resources:
  requests:
    cpu: 100m
    memory: 256Mi
  limits:
    cpu: 500m
    memory: 512Mi
```

Use matching API, worker, and frontend image versions. For local images,
override the repository, tag, and pull policy and load the images into your
development cluster.

The API and worker charts expose an `env` list. Helm replaces lists rather than
merging entries by name, so preserve the component's existing environment entries
when supplying an override. Environment variables backed by Secrets use normal
Kubernetes `valueFrom.secretKeyRef` entries in that list.

## Authentication Configuration

The installer creates the initial administrator. The API authenticates session
cookies containing signed JWT session identifiers and loads session/user
permissions from the database. Signing keys are initialized through the API's
security service; `KUBARR_JWT_SECRET` is not a supported configuration variable.

Manage users, roles, registration, and authentication settings through the
dashboard's supported settings and account controls. See the [User Guide](user-guide.md)
and [API](api.md) documentation.

## Storage Configuration

Shared storage uses explicit `storage.mounts` entries; multi-workload charts can
use `storage.workloads.<component>.mounts`. For example:

```yaml
storage:
  mounts:
    - name: library
      source: media
      target: /media
      mode: read-write
  media:
    existingClaim: media-data
```

Charts supporting private configuration claims expose `storage.config`, including
`size`, `storageClassName`, and `existingClaim`. These settings are chart-specific.
See [Storage](storage.md) and the [migration guide](https://github.com/smokeythebandit/kubarr-charts/blob/main/STORAGE-MIGRATION.md)
before upgrading older releases. Legacy `storage.mountPath`, role, and layout
settings are not the current shared-mount contract.

## Network Policy Configuration

App charts define their own NetworkPolicies and allowed communication. Their
`ingressFrom` and `egressTo` namespace selectors must match the deployment
topology. A namespace boundary alone does not enforce network isolation;
NetworkPolicies require a supporting CNI. See [Networking](networking.md).

Dashboard access is configured on the OpenResty gateway: NodePort, LoadBalancer,
or ClusterIP with port-forwarding. See [Quick Start](quick-start.md) for access
commands.

## Security Configuration

The API chart defaults to a non-root UID 1000, dropped capabilities,
`RuntimeDefault` seccomp, disabled privilege escalation, and a read-only root
filesystem. Writable storage and temporary directories are mounted separately.
Application charts have their own requirements; do not assume these defaults
apply to every managed workload.

Kubernetes permissions are defined by each chart's RBAC values and templates.
The API currently has broad cluster-management permissions, including RBAC
mutation. Review the actual rendered roles when changing deployment privileges.

## Scheduling

Platform charts expose `nodeSelector`, `tolerations`, `affinity`,
`podAnnotations`, and `imagePullSecrets`. Check the selected chart's values for
supported fields and defaults. For example:

```yaml
nodeSelector:
  kubernetes.io/hostname: worker-1
imagePullSecrets:
  - name: registry-credentials
```
