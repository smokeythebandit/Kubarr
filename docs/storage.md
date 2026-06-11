# Storage

Kubarr routes application storage through NFS so apps see the same durable storage layout regardless of which Kubernetes node runs them.

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

## Config Storage (NFS subpaths)

Each app stores its configuration — settings, databases, metadata — on the same NFS-backed `media-data` claim. The app Helm charts own the app-specific mount paths and subpaths so the backend only supplies the shared NFS claim contract.

Charts define the app-specific directories, subpaths, and any extra working paths such as cache or transcode directories.

---

## PostgreSQL

Kubarr's own database is a PostgreSQL instance managed by CloudNativePG. It stores everything internal to Kubarr: users, roles, app settings, audit logs, and VPN configs.

It runs as a StatefulSet with a single `ReadWriteOnce` PVC:

| PVC | Size | Mount path |
|-----|------|------------|
| `kubarr-db-data-0` | 5 Gi | `/var/lib/postgresql/data` |

This database PVC is separate from the app storage contract.

---

## Summary

| What | Storage type | Access | Provisioning |
|------|-------------|--------|--------------|
| Media files (`/data`) | NFS | ReadWriteMany | Manual — Kubarr creates PV/PVC at install time |
| App config (`/config`) | NFS subpath | ReadWriteMany | Chart configures app-specific path from `media-data` |
| Jellyfin/Plex cache & transcode | NFS subpath | ReadWriteMany | Chart configures app-specific path from `media-data` |
| Kubarr database | Default StorageClass | ReadWriteOnce | Dynamic — StatefulSet volumeClaimTemplate |
