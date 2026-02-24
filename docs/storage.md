# Storage

Kubarr uses two distinct tiers of storage: fast local storage for application config and databases, and a shared NFS volume for media files.

---

## Media Storage (NFS)

All media apps — Sonarr, Radarr, qBittorrent, Jellyfin, and others — mount the same NFS export at `/data` inside their containers:

```
NFS server (e.g. 192.168.1.120:/mnt/hdd_storage)
  └── /data/
        ├── downloads/    ← torrent/usenet clients write here
        ├── movies/       ← Radarr manages this
        └── tv/           ← Sonarr manages this
```

Because every app sees the same `/data` directory, a download client can save a file and Sonarr or Radarr can immediately find and move it — no copying, no duplication.

### How Kubarr sets it up

You provide the NFS server address and export path once, during the setup wizard. From that point on, every time you install an app Kubarr automatically:

1. Creates a Kubernetes **PersistentVolume** pointing at your NFS server
2. Creates a **PersistentVolumeClaim** (`media-data`) in the app's namespace
3. Deploys the Helm chart configured to use that claim

Each app gets its own PVC, but they all point at the same NFS export — so they share the same physical storage.

### Setup

During the setup wizard, provide:

| Field | Example |
|-------|---------|
| NFS server | `192.168.1.120` |
| NFS path | `/mnt/hdd_storage` |

The directory structure inside the share is created automatically by the apps on first run.

### NFS server requirements

Your NFS server must allow read/write access from your Kubernetes nodes. A typical `/etc/exports` entry:

```
/mnt/hdd_storage  192.168.1.0/24(rw,sync,no_subtree_check,no_root_squash)
```

---

## Config Storage (per-app PVCs)

Each app stores its configuration — settings, databases, metadata — in its own dedicated PVC, separate from the shared media volume. These use `ReadWriteOnce` access and are provisioned by your cluster's default StorageClass (e.g. the Proxmox CSI driver on ZFS).

| App | Config PVC size | Extra PVCs |
|-----|----------------|------------|
| Radarr | 1 Gi | — |
| Sonarr | 1 Gi | — |
| qBittorrent | 1 Gi | — |
| Transmission | 1 Gi | — |
| Deluge | 1 Gi | — |
| SABnzbd | 1 Gi | — |
| ruTorrent | 1 Gi | — |
| Jackett | 1 Gi | — |
| Jellyseerr | 1 Gi | — |
| Jellyfin | 5 Gi | 10 Gi cache |
| Plex | 10 Gi | 20 Gi transcode |

Jellyfin and Plex get extra PVCs for image/thumbnail cache and active transcoding respectively — these are ephemeral working directories that benefit from fast local storage.

---

## PostgreSQL

Kubarr's own database is a PostgreSQL instance managed by CloudNativePG. It stores everything internal to Kubarr: users, roles, app settings, audit logs, and VPN configs.

It runs as a StatefulSet with a single `ReadWriteOnce` PVC:

| PVC | Size | Mount path |
|-----|------|------------|
| `kubarr-db-data-0` | 5 Gi | `/var/lib/postgresql/data` |

Like config PVCs, this is provisioned by the default StorageClass.

---

## Summary

| What | Storage type | Access | Provisioning |
|------|-------------|--------|--------------|
| Media files (`/data`) | NFS | ReadWriteMany | Manual — Kubarr creates PV/PVC at install time |
| App config (`/config`) | Default StorageClass | ReadWriteOnce | Dynamic — chart creates PVC on install |
| Jellyfin/Plex cache & transcode | Default StorageClass | ReadWriteOnce | Dynamic — chart creates PVC on install |
| Kubarr database | Default StorageClass | ReadWriteOnce | Dynamic — StatefulSet volumeClaimTemplate |
