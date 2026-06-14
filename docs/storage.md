# Storage

Kubarr's Helm charts use one shared ReadWriteMany media claim as the storage contract between the dashboard, worker, and installed apps. The claim is named `media-data` by default and is backed by NFS.

---

## Shared Media Claim

Most charts declare the same storage value:

```yaml
storage:
  media:
    existingClaim: media-data
```

The `kubarr-common` chart helper renders a PersistentVolume and PersistentVolumeClaim for each chart namespace unless `storage.media.create` is set to `false`. The generated claim:

| Field | Default |
|-------|---------|
| Claim name | `media-data` |
| Access mode | `ReadWriteMany` |
| Storage class | `""` |
| Reclaim policy | `Retain` |
| Requested size | `1Ti` |

Each app namespace gets its own PVC object, but those PVCs point to the same NFS export. That lets download clients, media managers, and media servers share files without copying them between volumes.

---

## NFS Backend

The shared claim can point at either Kubarr's managed NFS service or an external NFS server.

By default, the storage helper uses the managed NFS service:

```yaml
kubarr-managed-nfs.kubarr-storage.svc.cluster.local:/
```

The managed NFS chart defaults are:

| Value | Default |
|-------|---------|
| Namespace | `kubarr-storage` |
| Service port | `2049` |
| Export path | `/exports` |
| Backing PVC | `managed-nfs-data` |
| Backing PVC size | `1Ti` |

For external NFS, set the app chart storage values to your server and export path:

```yaml
storage:
  media:
    existingClaim: media-data
    nfs:
      server: 192.168.1.120
      path: /mnt/hdd_storage
      size: 1Ti
```

Your NFS server must allow read/write access from the Kubernetes nodes. A typical `/etc/exports` entry is:

```text
/mnt/hdd_storage  192.168.1.0/24(rw,sync,no_subtree_check,no_root_squash)
```

---

## Directory Layout

The charts use subpaths on the shared claim to keep app config, downloads, media, cache, and transcode data separate:

```text
media-data
├── config/
│   ├── jellyfin/
│   ├── plex/
│   ├── radarr/
│   ├── sonarr/
│   └── qbittorrent/
├── downloads/
│   ├── deluge/
│   ├── qbittorrent/
│   ├── rutorrent/
│   ├── sabnzbd/
│   └── transmission/
├── media/
├── cache/
│   └── jellyfin/
├── transcode/
│   └── plex/
└── system/
    └── postgresql/
```

The exact paths are owned by each chart's `values.yaml`, not by the application image.

---

## App Mounts

Download clients mount their config and download directories from app-specific subpaths:

| App | Config mount | Config subpath | Downloads mount | Downloads subpath |
|-----|--------------|----------------|-----------------|-------------------|
| Deluge | `/config` | `config/deluge` | `/downloads` | `downloads/deluge` |
| qBittorrent | `/config` | `config/qbittorrent` | `/downloads` | `downloads/qbittorrent` |
| ruTorrent | `/config` | `config/rutorrent` | `/downloads` | `downloads/rutorrent` |
| SABnzbd | `/config` | `config/sabnzbd` | `/downloads` | `downloads/sabnzbd` |
| Transmission | `/config` | `config/transmission` | `/downloads` | `downloads/transmission` |

Media managers mount shared media and downloads paths:

| App | Config mount | Config subpath | Media mount | Media subpath | Downloads mount | Downloads subpath |
|-----|--------------|----------------|-------------|---------------|-----------------|-------------------|
| Radarr | `/config` | `config/radarr` | `/media` | `media` | `/downloads` | `downloads` |
| Sonarr | `/config` | `config/sonarr` | `/media` | `media` | `/downloads` | `downloads` |

Media servers mount shared media read-only and use separate config/work directories:

| App | Config mount | Config subpath | Media mount | Media subpath | Extra mount |
|-----|--------------|----------------|-------------|---------------|-------------|
| Jellyfin | `/config` | `config/jellyfin` | `/data` | `media` | `/cache` -> `cache/jellyfin` |
| Plex | `/config` | `config/plex` | `/data` | `media` | `/transcode` -> `transcode/plex` |

Jellyseerr mounts `/app/config` from `config/jellyseerr` and `/data` from `media`. Jackett mounts `/config` from `config/jackett`.

---

## Kubarr API And Worker

The Kubarr API and worker also mount the shared media claim so the file browser and app lifecycle code see the same storage as the apps:

```yaml
storage:
  media:
    existingClaim: media-data
  mountPath: /data
```

Both deployments set `KUBARR_STORAGE_PATH` to the configured mount path and mount the claim there. They also mount `/tmp` as an `emptyDir` because the pods run with a read-only root filesystem.

---

## PostgreSQL

Kubarr's PostgreSQL chart runs a single-replica StatefulSet named `kubarr-db`. The database stores Kubarr users, roles, settings, app state, audit logs, sessions, and other internal data.

Current chart storage mounts the shared media claim at `/var/lib/postgresql/data` using the `system/postgresql` subpath:

| Field | Value |
|-------|-------|
| Workload | StatefulSet `kubarr-db` |
| Claim | `media-data` |
| Mount path | `/var/lib/postgresql/data` |
| Subpath | `system/postgresql` |
| `PGDATA` | `/var/lib/postgresql/data/pgdata` |

The chart still has `persistence.size` and `persistence.storageClassName` values, but the current StatefulSet template does not create a separate database PVC from those values.

---

## Summary

| What | Storage type | Access | Provisioning |
|------|--------------|--------|--------------|
| Shared media claim | NFS PV/PVC | ReadWriteMany | `kubarr-common.storage.mediaClaim` |
| App config | NFS subpath | ReadWriteMany | App chart values |
| Downloads | NFS subpath | ReadWriteMany | Download-client chart values |
| Media library | NFS subpath | ReadWriteMany | Media-manager/server chart values |
| Cache/transcode | NFS subpath | ReadWriteMany | Jellyfin/Plex chart values |
| Kubarr file browser | NFS PVC mounted at `/data` | ReadWriteMany | `system/kubarr` chart |
| Kubarr database | NFS subpath `system/postgresql` | ReadWriteMany claim, single writer workload | `system/postgresql` chart |
