# Kubarr Production Deployment — Design

**Date:** 2026-08-14
**Goal:** Get Kubarr into production on the home lab so external test users can log into the Kubarr dashboard and use the media apps.

## Context

- **Model:** Hosted. Benjamin runs the single production instance; test users get accounts. No public release artifacts required for this milestone.
- **Repos:**
  - `Kubarr` — api, worker, frontend, gateway, dns-webhook, CLI installer (Rust). Uncommitted TUI installer work in `code/cli` (~700 lines, `tui.rs` + `install_events.rs`); builds clean, no TODO markers, no automated tests.
  - `kubarr-charts` — Helm charts for the media stack (media-server, media-manager, indexer, download-client, monitoring, system). Clean, up to date.
- **Infrastructure (inventoried 2026-08-14):**
  - Proxmox VE 9.2.10, single node `pve` at 192.168.1.2: 32 cores, 62.7 GB RAM (~19 GB unallocated), storage pools `ssd_storage` (ZFS, 581 GB free) and `hdd_storage` (ZFS, 14.7 TB, 13.4 TB used — existing media library).
  - Same host exports NFS: `/mnt/ssd_storage` and `/mnt/hdd_storage`, currently restricted to 192.168.1.3 and 192.168.1.4.
  - Existing VMs: `ubuntu` (100), `k8s-tst` (101, old test cluster — untouched by this plan), `development` (102), `gameserver` (103, stopped).
  - External access path: nginx reverse proxy on the OPNsense firewall.
  - Proxmox API token `root@pam!claude` available for automation (secret held out-of-band; never committed).

## Decisions

1. **Production runs in a new dedicated VM** on `pve` (existing `k8s-tst` stays untouched).
   - Spec: 8 cores, 16 GB RAM, 100 GB disk on `ssd_storage`, virtio NIC on `vmbr0`, QEMU guest agent enabled, onboot=1.
   - Guest OS: current Ubuntu LTS server (matches what the installer targets).
   - Static IP via OPNsense DHCP reservation; that IP gets added to both NFS exports on the Proxmox host.
2. **Bootstrap via the Kubarr CLI installer**, including the new TUI. The uncommitted TUI work is finished first and is the installer used for production — installing production is also the validation run for the installer.
3. **Storage:** media apps mount the existing library from `hdd_storage` over NFS (read-write, preserving the current library); app config/state uses NFS on `ssd_storage` per the charts' managed-NFS pattern.
4. **Exposure:** cluster ingress on the VM's static IP; OPNsense nginx adds vhosts (Kubarr dashboard, Jellyfin/media apps, request app) proxying to the ingress. TLS terminates wherever the existing nginx setup terminates it for other services (ACME on OPNsense, or pass-through to cert-manager in-cluster) — decided during setup to match existing config.
5. **Users:** Kubarr JWT/RBAC accounts per tester, non-admin role. Testers use both the Kubarr dashboard and the media apps.

## Plan of record

### Phase 1 — Finish the TUI installer
- Definition of done: one complete, successful installer run in a VM — wizard flow, bootstrap, all install phases rendered through the TUI event stream, no panics, no broken terminal state.
- Fix whatever that run exposes. No test-suite build-out in this milestone; the end-to-end run is the verification.
- Commit on branch `feat/tui-installer`, merge to `main` once the run passes.
- Ensure CI-published images and charts are current before the production install (`publish.yml` — verify it covers all images the installer pulls).

### Phase 2 — Provision the production VM
- Create the VM per spec above (Proxmox API token available for automation).
- Add DHCP reservation on OPNsense; add the VM's IP to both NFS exports on `pve`.
- Snapshot the clean OS state ("pre-install").

### Phase 3 — Install
- Run the installer against the VM. On failure: roll back to the pre-install snapshot, fix, repeat until a clean run.
- The surviving install *is* production — no separate rehearsal environment (snapshot rollback covers the rehearsal role).
- Point media apps at the existing library on `hdd_storage`; verify library content is visible before proceeding.
- Snapshot again post-install ("known-good").

### Phase 4 — Expose
- OPNsense nginx vhosts for the Kubarr dashboard and each user-facing media app (exact list = what the installer deployed), proxying to cluster ingress.
- TLS per decision 4. Verify externally from outside the LAN.

### Phase 5 — Users and smoke test
- Create tester accounts (non-admin RBAC role).
- Smoke test from an external network: log in to the dashboard, deploy/manage an app, view logs/metrics, stream media through the media server.
- Invite test users.

## Success criteria

- An outside user on their own device can log into the Kubarr dashboard, operate within their role, and stream media.
- Rollback exists: Proxmox "known-good" snapshot post-setup.
- Installer work is merged to `main` and produced the running cluster.

## Risks / constraints

- **RAM headroom:** ~19 GB unallocated; the new VM takes 16 GB. Acceptable now; starting `gameserver` (6 GB) later would overcommit — revisit then.
- **Installer untested end-to-end:** mitigated by snapshot-rollback install loop in Phase 3.
- **NFS single point:** media and config storage live on the Proxmox host itself; host reboot takes storage down with the cluster. Accepted for a test deployment.
- **Existing media library mounted read-write** by apps under test — snapshot covers the VM, not the library. ZFS snapshots on `hdd_storage` are the mitigation if wanted (out of scope here).
