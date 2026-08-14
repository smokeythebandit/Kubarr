# Kubarr Production Deployment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kubarr running in production on a new Proxmox VM, exposed through OPNsense nginx, with test users able to log into the dashboard and stream media.

**Architecture:** Finish and merge the uncommitted TUI installer, provision a dedicated VM on Proxmox (`pve`, 192.168.1.2), bootstrap single-node k3s + Kubarr with the CLI installer (the production install doubles as the installer's end-to-end validation, with Proxmox snapshot rollback as the retry mechanism), mount the existing media library from the host's NFS export, then wire external access and users.

**Tech Stack:** Rust CLI (ratatui), k3s, Helm charts (kubarr-charts), Proxmox VE API, NFS, OPNsense nginx.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-08-14-kubarr-production-deploy-design.md`
- Proxmox API: `https://192.168.1.2:8006/api2/json`, token id `root@pam!claude` (secret provided in-session — never write it to any file or commit).
- New VM: id **104**, name **kubarr-prod**, 8 cores, 16384 MB RAM, 100 GB disk on `ssd_storage`, bridge `vmbr0`, QEMU guest agent on, onboot=1, Ubuntu 24.04 LTS.
- Existing VMs 100–103 must not be touched.
- NFS exports on pve (`/mnt/ssd_storage`, `/mnt/hdd_storage`) currently allow only 192.168.1.3/.4 — the new VM's IP must be added to both.
- Existing media library on `/mnt/hdd_storage` is mounted read-write by the new stack — never reorganize or delete library content.
- TUI work is committed on branch `feat/tui-installer`, merged to `main` only after the end-to-end install succeeds.
- Commit messages end with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task 1: Commit the TUI installer work on a branch

**Files:**
- Modify (already dirty in worktree): `code/cli/src/*.rs`, `code/cli/Cargo.toml`, `code/cli/Cargo.lock`
- Create (already exist untracked): `code/cli/src/tui.rs`, `code/cli/src/install_events.rs`

**Interfaces:**
- Produces: branch `feat/tui-installer` containing the full TUI wizard; `kubarr bootstrap` with no args on a TTY runs `tui::run_bootstrap_wizard()`, with args runs the flag path (`code/cli/src/bootstrap.rs:28-36`).

- [ ] **Step 1: Build and lint gate**

Run: `cd code/cli && cargo build && cargo clippy -- -D warnings`
Expected: both succeed. If clippy fails, fix warnings before committing (small mechanical fixes only; anything behavioral gets its own commit).

- [ ] **Step 2: Non-TUI regression check (dry run)**

Run: `cargo run -- bootstrap --cluster-mode single-node --storage-mode external-nfs --nfs-server 192.168.1.2 --nfs-path /mnt/hdd_storage --admin-username admin --admin-email bmartens@pm.me --admin-password placeholder123 --dry-run --skip-cluster-check`
Expected: prints planned commands, exits 0. The flag path must not have regressed.

- [ ] **Step 3: Manual TUI smoke (user drives)**

Run in a real terminal: `cargo run -- bootstrap` — walk the wizard to the summary screen, then cancel (do not install on the dev machine). Screens render, navigation works, cancel restores the terminal.

- [ ] **Step 4: Commit on branch**

```bash
git checkout -b feat/tui-installer
git add code/cli
git commit -m "feat(cli): add ratatui install wizard with live install event stream

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

### Task 2: Verify published images and charts are current

**Interfaces:**
- Consumes: `.github/workflows/publish.yml` (images backend, worker, frontend, dns-webhook to ghcr).
- Produces: confirmation that the chart refs baked into the CLI (`chart_ref(...)` in `code/cli/src/util.rs`) and the image tags they pull exist in the registry at deployable versions.

- [ ] **Step 1: Extract chart/image refs from the CLI**

Run: `grep -rn "CHART_REF\|oci://" code/cli/src/util.rs code/cli/src/types.rs`
Record each ref and default version.

- [ ] **Step 2: Check the registry**

For each ref: `helm show chart <oci-ref> --version <v>` (or `docker manifest inspect` for images).
Expected: all resolve. If any missing → run the publish workflow (`gh workflow run publish.yml`) from latest `main` and wait for completion before Task 5.

### Task 3: Provision VM 104 (kubarr-prod)

**Interfaces:**
- Consumes: Proxmox API token (in-session).
- Produces: VM 104 running Ubuntu 24.04, reachable over SSH at a static IP (call it `$KUBARR_IP` everywhere below), snapshot `pre-install` taken.

- [ ] **Step 1: Pick a free IP**

Ping-scan a candidate (e.g. 192.168.1.5): `ping -c1 -W2 192.168.1.5` must fail, and the OPNsense DHCP static-mapping list must not contain it (user confirms in OPNsense UI). Record as `$KUBARR_IP`.

- [ ] **Step 2: Create the VM**

Preferred path — SSH to pve (`ssh root@192.168.1.2`; if no key auth, user adds the session key or runs these on the pve shell):

```bash
cd /var/lib/vz/template # any scratch dir on pve
wget -N https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img
qm create 104 --name kubarr-prod --cores 8 --memory 16384 --net0 virtio,bridge=vmbr0,firewall=1 --agent 1 --onboot 1 --ostype l26
qm importdisk 104 noble-server-cloudimg-amd64.img ssd_storage
qm set 104 --scsihw virtio-scsi-single --scsi0 ssd_storage:vm-104-disk-0,iothread=1
qm disk resize 104 scsi0 100G
qm set 104 --ide2 ssd_storage:cloudinit --boot order=scsi0
qm set 104 --ciuser kubarr --sshkeys ~/.ssh/authorized_keys --ipconfig0 ip=$KUBARR_IP/24,gw=192.168.1.1
qm start 104
```

- [ ] **Step 3: Verify boot and SSH**

Run: `until ssh -o ConnectTimeout=3 kubarr@$KUBARR_IP true; do sleep 5; done && ssh kubarr@$KUBARR_IP 'lsb_release -ds; nproc; free -h'`
Expected: Ubuntu 24.04, 8 CPUs, ~16 GB.

- [ ] **Step 4: Snapshot**

Proxmox API: `POST /nodes/pve/qemu/104/snapshot` with `snapname=pre-install`. Verify listed via `GET /nodes/pve/qemu/104/snapshot`.

### Task 4: NFS export for the new VM

**Interfaces:**
- Produces: `$KUBARR_IP` allowed on both exports of 192.168.1.2; verified mountable from VM 104.

- [ ] **Step 1: Add the IP on pve**

On pve shell, edit `/etc/exports`: append `,$KUBARR_IP(rw,no_subtree_check)` (match the exact option string already used for 192.168.1.3/.4 — read the file first and copy its option set) to both `/mnt/ssd_storage` and `/mnt/hdd_storage` lines, then `exportfs -ra`.

- [ ] **Step 2: Verify from VM 104**

```bash
ssh kubarr@$KUBARR_IP 'sudo apt-get install -y nfs-common && showmount -e 192.168.1.2 && sudo mount -t nfs 192.168.1.2:/mnt/hdd_storage /mnt && ls /mnt | head && sudo umount /mnt'
```
Expected: both exports listed for `$KUBARR_IP`, mount succeeds, existing media directories visible.

### Task 5: Production install via TUI installer

**Interfaces:**
- Consumes: `feat/tui-installer` branch build; VM 104 with snapshot `pre-install`.
- Produces: single-node k3s cluster on VM 104 with Kubarr + media stack installed; snapshot `known-good`.

- [ ] **Step 1: Ship the CLI to the VM**

```bash
cd code/cli && cargo build --release
scp target/release/kubarr kubarr@$KUBARR_IP:
```

- [ ] **Step 2: Run the TUI install (user drives, this is the validation run)**

`ssh -t kubarr@$KUBARR_IP ./kubarr bootstrap` — wizard choices: single-node cluster, external-nfs storage (server 192.168.1.2, config on `/mnt/ssd_storage`, media on `/mnt/hdd_storage`), admin account, Grafana dashboards on.
Expected: wizard completes, install phases stream in the TUI, exit clean.
**On any failure:** capture the error (screenshot/log), roll back — `POST /nodes/pve/qemu/104/snapshot/pre-install/rollback`, wait, `POST /nodes/pve/qemu/104/status/start` — fix the bug in `code/cli`, commit to `feat/tui-installer`, repeat this task. Iterate until one clean run.

- [ ] **Step 3: Verify the cluster**

```bash
ssh kubarr@$KUBARR_IP 'sudo k3s kubectl get nodes && sudo k3s kubectl get pods -A | grep -v Running | grep -v Completed || true'
curl -s -o /dev/null -w '%{http_code}\n' http://$KUBARR_IP:30081/   # backend NodePort
```
Expected: node Ready, no crashlooping pods, HTTP answer from Kubarr.

- [ ] **Step 4: Log into the dashboard from LAN**

Open `http://$KUBARR_IP:<frontend-port>` (port from install summary), log in as admin, deploy Jellyfin from the catalog pointed at the NFS media path, confirm the existing library is visible in it.

- [ ] **Step 5: Snapshot**

`POST /nodes/pve/qemu/104/snapshot` with `snapname=known-good`.

- [ ] **Step 6: Merge the installer**

```bash
git checkout main && git merge --no-ff feat/tui-installer -m "feat(cli): TUI install wizard, validated by production install

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push origin main
```

### Task 6: External exposure via OPNsense nginx

**Interfaces:**
- Consumes: cluster ingress/NodePorts on `$KUBARR_IP`.
- Produces: `kubarr.void-zero.com` (+ one vhost per user-facing media app, e.g. `media.void-zero.com` for the new Jellyfin — legacy `jellyfin` name stays on the old VM until decommission) proxying to the cluster, valid TLS, reachable from outside.

- [ ] **Step 1: Add vhosts (user drives OPNsense UI, mirroring the existing grafana/auth vhost pattern)**

Upstream = `$KUBARR_IP:<ingress-port>` per app; TLS terminated the same way as the existing vhosts (ACME on OPNsense). Add matching public DNS records for each new subdomain.

- [ ] **Step 2: Verify externally**

From outside the LAN (e.g. phone hotspot, or `curl --resolve` against the public IP 31.187.133.50): dashboard and media app load over HTTPS with valid certs.

### Task 7: Test users and smoke test

**Interfaces:**
- Consumes: exposed dashboard; Kubarr JWT/RBAC user management.
- Produces: tester accounts, verified end-to-end user journey, invitations sent.

- [ ] **Step 1: Create tester accounts** — non-admin role, one per tester, via the Kubarr dashboard admin UI.

- [ ] **Step 2: External smoke test as a tester** — from an outside network: log in, verify role limits (no admin pages), open logs/metrics of an app, stream a media file end-to-end via the media vhost.

- [ ] **Step 3: Invite users** — send URLs + credentials; point feedback at the repo's GitHub issues.

---

## Self-review notes

- Spec coverage: Phase 1 → Tasks 1–2, Phase 2 → Tasks 3–4, Phase 3 → Task 5, Phase 4 → Task 6, Phase 5 → Task 7. Success criteria covered by Task 5 step 3–4, Task 6 step 2, Task 7 step 2; rollback by snapshots (Tasks 3/5).
- Interactive steps (TUI runs, OPNsense UI) are explicitly user-driven; everything else is executable from this session.
- Chart/image refs intentionally resolved in Task 2 rather than hardcoded here — they live in `code/cli/src/util.rs` and may change with the branch.
