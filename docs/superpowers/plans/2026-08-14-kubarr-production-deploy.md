# Kubarr Production Deployment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kubarr running in production on a new Proxmox VM, exposed through OPNsense nginx, with test users able to log into the dashboard and stream media.

**Architecture:** Provision a dedicated VM on Proxmox (`pve`, 192.168.1.2), deploy Kubernetes and the published Kubarr Helm charts with Proxmox snapshot rollback, mount the existing media library from the host's NFS export, then wire external access and users.

**Tech Stack:** k3s, Helm charts (kubarr-charts), Proxmox VE API, NFS, OPNsense nginx.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-08-14-kubarr-production-deploy-design.md`
- Proxmox API: `https://192.168.1.2:8006/api2/json`, token id `root@pam!claude` (secret provided in-session — never write it to any file or commit).
- New VM: id **104**, name **kubarr-prod**, 8 cores, 16384 MB RAM, 100 GB disk on `ssd_storage`, bridge `vmbr0`, QEMU guest agent on, onboot=1, Ubuntu 24.04 LTS.
- Existing VMs 100–103 must not be touched.
- NFS exports on pve (`/mnt/ssd_storage`, `/mnt/hdd_storage`) currently allow only 192.168.1.3/.4 — the new VM's IP must be added to both.
- Existing media library on `/mnt/hdd_storage` is mounted read-write by the new stack — never reorganize or delete library content.

---

### Task 1: Verify published images and charts are current

**Interfaces:**
- Consumes: `.github/workflows/publish.yml` (images backend, worker, frontend, dns-webhook to ghcr).
- Produces: confirmation that the selected chart versions and their application image tags exist in the registry.

- [ ] **Step 1: Select chart/image refs**

Record the chart references, versions, and image tags from the deployment's chart metadata and values.

- [ ] **Step 2: Check the registry**

For each ref: `helm show chart <oci-ref> --version <v>` (or `docker manifest inspect` for images).
Expected: all resolve. If any are missing, publish them through the appropriate repository's CI/CD pipeline and wait for completion before Task 4.

### Task 2: Provision VM 104 (kubarr-prod)

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

### Task 3: NFS export for the new VM

**Interfaces:**
- Produces: `$KUBARR_IP` allowed on both exports of 192.168.1.2; verified mountable from VM 104.

- [ ] **Step 1: Add the IP on pve**

On pve shell, edit `/etc/exports`: append `,$KUBARR_IP(rw,no_subtree_check)` (match the exact option string already used for 192.168.1.3/.4 — read the file first and copy its option set) to both `/mnt/ssd_storage` and `/mnt/hdd_storage` lines, then `exportfs -ra`.

- [ ] **Step 2: Verify from VM 104**

```bash
ssh kubarr@$KUBARR_IP 'sudo apt-get install -y nfs-common && showmount -e 192.168.1.2 && sudo mount -t nfs 192.168.1.2:/mnt/hdd_storage /mnt && ls /mnt | head && sudo umount /mnt'
```
Expected: both exports listed for `$KUBARR_IP`, mount succeeds, existing media directories visible.

### Task 4: Deploy and verify the application

**Interfaces:**
- Consumes: published charts and images; VM 104 with snapshot `pre-install`.
- Produces: single-node k3s cluster on VM 104 with Kubarr + media stack installed; snapshot `known-good`.

- [ ] **Step 1: Deploy the published charts**

Provision the single-node Kubernetes cluster and deploy the selected chart versions with the external NFS server at 192.168.1.2, config on `/mnt/ssd_storage`, and media on `/mnt/hdd_storage`.
**On any failure:** capture the error, roll back to the `pre-install` snapshot, correct the deployment configuration, and repeat the validation.

- [ ] **Step 2: Verify the cluster**

```bash
ssh kubarr@$KUBARR_IP 'sudo k3s kubectl get nodes && sudo k3s kubectl get pods -A | grep -v Running | grep -v Completed || true'
curl -s -o /dev/null -w '%{http_code}\n' http://$KUBARR_IP:30081/   # backend NodePort
```
Expected: node Ready, no crashlooping pods, HTTP answer from Kubarr.

- [ ] **Step 3: Log into the dashboard from LAN**

Open the configured gateway address, log in as admin, deploy Jellyfin from the catalog pointed at the NFS media path, and confirm the existing library is visible in it.

- [ ] **Step 4: Snapshot**

`POST /nodes/pve/qemu/104/snapshot` with `snapname=known-good`.

### Task 5: External exposure via OPNsense nginx

**Interfaces:**
- Consumes: cluster ingress/NodePorts on `$KUBARR_IP`.
- Produces: `kubarr.void-zero.com` (+ one vhost per user-facing media app, e.g. `media.void-zero.com` for the new Jellyfin — legacy `jellyfin` name stays on the old VM until decommission) proxying to the cluster, valid TLS, reachable from outside.

- [ ] **Step 1: Add vhosts (user drives OPNsense UI, mirroring the existing grafana/auth vhost pattern)**

Upstream = `$KUBARR_IP:<ingress-port>` per app; TLS terminated the same way as the existing vhosts (ACME on OPNsense). Add matching public DNS records for each new subdomain.

- [ ] **Step 2: Verify externally**

From outside the LAN (e.g. phone hotspot, or `curl --resolve` against the public IP 31.187.133.50): dashboard and media app load over HTTPS with valid certs.

### Task 6: Test users and smoke test

**Interfaces:**
- Consumes: exposed dashboard; Kubarr JWT/RBAC user management.
- Produces: tester accounts, verified end-to-end user journey, invitations sent.

- [ ] **Step 1: Create tester accounts** — non-admin role, one per tester, via the Kubarr dashboard admin UI.

- [ ] **Step 2: External smoke test as a tester** — from an outside network: log in, verify role limits (no admin pages), open logs/metrics of an app, stream a media file end-to-end via the media vhost.

- [ ] **Step 3: Invite users** — send URLs + credentials; point feedback at the repo's GitHub issues.

---

## Self-review notes

- Spec coverage: Phase 1 → Task 1, Phase 2 → Tasks 2–3, Phase 3 → Task 4, Phase 4 → Task 5, Phase 5 → Task 6. Rollback is provided by the snapshots in Tasks 2 and 4.
- OPNsense UI changes are explicitly user-driven.
- Chart/image references are recorded in Task 1 from the selected deployment artifacts.
