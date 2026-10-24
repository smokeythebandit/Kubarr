# Storage

Kubarr separates shared NFS data from private application configuration claims.
The installer configures the shared-storage backend; app installation records
explicit mount choices for the worker to deploy.

## Shared Media Claim

App charts generally use:

```yaml
storage:
  media:
    existingClaim: media-data
```

The `kubarr-common` helpers create a namespace-local PV/PVC pair backed by the
shared NFS export unless `storage.media.create` is `false`. Separate namespaces
have separate PVC objects pointing to that export, allowing download clients,
media managers, and servers to share files.

The shared claim defaults to ReadWriteMany access, an empty storage class,
`Retain` reclaim policy, and a requested size of `1Ti`. Consult the selected
chart and common helpers for exact rendered settings.

## NFS Backend

Choose managed NFS or external NFS in the bootstrap wizard. The managed service
is `kubarr-managed-nfs.kubarr-storage.svc.cluster.local`, exporting `/` to clients
on port 2049. Its backing PVC stays on the bootstrap StorageClass.

For an external server, chart values include:

```yaml
storage:
  media:
    existingClaim: media-data
    nfs:
      server: 192.168.1.120
      path: /mnt/hdd_storage
      size: 1Ti
```

The export must be accessible from the Kubernetes nodes with the permissions
required by the application UIDs/GIDs. The installer's external-NFS check
verifies a write/read/remove cycle and cleanup from one Ready Linux node.
See the [Bootstrap Guide](https://github.com/smokeythebandit/kubarr-cli/blob/main/docs/bootstrap.md)
for the check's scope.

## Explicit Shared Mounts

Every shared-data mount declares its name, export-relative source, container
target, and access mode. For example, a media manager can mount:

```yaml
storage:
  mounts:
    - name: library
      source: media
      target: /media
      mode: read-write
    - name: downloads
      source: downloads/qbittorrent
      target: /downloads/qbittorrent
      mode: read-write
  media:
    existingClaim: media-data
```

This is an illustrative mount list, not the complete defaults for every app.
Helm replaces lists: supplying `storage.mounts` replaces that chart's default
mount list. Multi-workload charts use
`storage.workloads.<component>.mounts` where defined. Sources and targets are
owned by each chart's `values.yaml`; there is no implicit role/layout mapping.

## Private App Configuration

The 11 charts whose `/config` previously lived on shared NFS now use private
configuration claims. A typical configuration is:

```yaml
storage:
  config:
    create: true
    existingClaim: ""
    size: 5Gi
    storageClassName: ""
    migrationConfirmed: false
```

An empty `storageClassName` uses the cluster's default StorageClass. Managed NFS
does not provide that default private-config StorageClass. Select an appropriate
class or an existing claim for the app. Configuration paths and claim behavior
vary by chart; consult its values before overriding them.

Immich PostgreSQL and Frigate SQLite require local/block storage. Grafana SQLite
uses its dedicated PVC, and Fluent Bit positions remain node-local.

## Kubarr API and Worker

The API and worker mount the shared export explicitly for storage browsing and
lifecycle work:

```yaml
storage:
  mounts:
    - name: storage-browser
      source: .
      target: /data
      mode: read-write
  media:
    existingClaim: media-data
```

The export-root mount is intentional for these platform components. Their
containers also have writable temporary storage for their read-only root
filesystems.

## PostgreSQL

The platform PostgreSQL chart stores Kubarr state in the `kubarr-database`
namespace. Its current shared mount declares source `system/postgresql` and
target `/var/lib/postgresql/data`. This platform database is separate from
application-specific databases, whose storage requirements may differ.

## Upgrading Existing Storage

Read [STORAGE-MIGRATION.md](https://github.com/smokeythebandit/kubarr-charts/blob/main/STORAGE-MIGRATION.md)
before upgrading to declarative mounts and private configuration claims.

Stop writes, back up, migrate, and verify existing configuration data before
setting `storage.config.migrationConfirmed=true`. The chart does not copy that
data. Legacy role/layout/path values fail rendering, including values carried
forward from an older Helm release. The migration guide covers retained claims,
directories, and the `storage.media.create=false` handoff.
