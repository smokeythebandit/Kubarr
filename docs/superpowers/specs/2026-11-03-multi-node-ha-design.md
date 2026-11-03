# Multi-node high availability — design

Date: 2026-11-03
Status: awaiting user review

## Intent

Kubarr runs on clusters of any size, from a single k3s VM to multi-node Talos.
Today every system component runs as one replica regardless of how many nodes
exist, so losing any node that hosts the gateway, backend, or database takes
the dashboard (and app routing) down even when other nodes are healthy.

Goal: on any install, Kubarr detects how many nodes it can schedule on and,
when there are two or more, runs the components where it makes sense in a
highly-available shape — automatically, at install time and whenever nodes are
added or removed later. The user asked for "most things that make sense to run
HA, run them HA"; the per-component choices below are the builder's
interpretation of that, confirmed in conversation.

## Goals

- Auto-detect schedulable node count on every install (not just the author's
  cluster).
- On 2+ nodes, run stateless system components with multiple replicas spread
  across nodes, with PodDisruptionBudgets so drains/upgrades never take all
  replicas down.
- Replace the single-instance PostgreSQL chart with CloudNativePG (CNPG) on
  every install: 1 instance on a single node, `min(nodes, 3)` instances on
  multi-node, automatic failover.
- PostgreSQL replicas use node-local disk (a local-path StorageClass), not NFS.
- Singleton workers (kubarr-worker loops, backend scheduler) get leader
  election so a standby takes over quickly when the leader's node dies.
- Autoscale: when a node joins, scale up without user action; when a node is
  deleted from the cluster, scale down.
- Nightly `pg_dump` of the database to NFS on every install.

## Non-goals

- Surviving loss of the NFS server. NFS is external to the cluster and remains
  a single point of failure; HA here covers losing a Kubernetes node.
- Surviving loss of etcd quorum. On the author's cluster, gertrude hosts 2 of 3
  control-plane nodes; losing it loses the cluster. Out of scope.
- HA for media apps (Sonarr, Radarr, Jellyfin, etc.). They use SQLite/local
  state and cannot run two copies.
- HA for monitoring storage (VictoriaMetrics, VictoriaLogs, Grafana). Single
  writer storage; a brief metrics/logs gap during node loss is acceptable.
- Horizontal autoscaling by load (HPA). "Autoscale" in this doc means scaling
  by node count only.
- New UI. A read-only HA status indicator is optional (see "Left to builder").
- Migrating existing installs. Kubarr is still in development with no
  external users; the author's cluster will be wiped and reinstalled. The
  old `system/postgresql` chart is deleted, not migrated from.

## Anti-goals (would be a failure even if it technically works)

- Two PostgreSQL pods writing the same data directory (e.g. sharing one NFS
  path). Must never happen.
- Scaling down when a node goes NotReady. NotReady is exactly the failure HA
  exists to survive; only node deletion triggers scale-down.
- Replicas "spread" onto the same node, giving the appearance of HA without
  the substance. Spread must be enforced across nodes (hostname topology).
- A single-node install getting worse: extra pods, extra RAM, or failing
  installs because a PDB or anti-affinity rule can't be satisfied.
- Duplicate background work (two workers both reconciling, two schedulers
  firing the same job) as a side effect of adding replicas.

## Current state (mapped 2026-11-03)

- **Initial install:** `kubarr-cli/src/bootstrap.rs:49-103` — managed-nfs,
  observability charts, DB secret (`db_credentials.rs:102`), postgresql
  (`database.rs:10-75`), then backend/frontend/openresty/worker
  (`install.rs:14-147`), each `helm upgrade --install … -n default`. No replica
  values set anywhere.
- **Later upgrades:** backend queues `OP_UPDATE`; worker executes it
  (`app_manager.rs:1371-1377`) with `--reset-then-reuse-values`
  (`deployment.rs:374-383`). No periodic reconciler for system charts exists.
  Worker's `reconcile_states` loop (`app_manager.rs:891-909`) already holds a
  kube client and runs helm on system releases.
- **cert-manager** is a catalog system component installed via the backend
  (`catalog.rs:96-107`). **cloudflared and kubarr-dns-webhook are not
  installed by Kubarr** (only referenced by e2e tests).
- **Monitoring:** CLI installs fluent-bit, victorialogs, victoriametrics,
  node-exporter, flow-exporter, grafana (`observability.rs`). kube-state-metrics
  and vmalert are catalog system charts.
- **Backend multi-replica hazards:**
  - Scheduler (`scheduler.rs:87-111`: session_cleanup hourly, audit_retention
    daily, ChartSyncTask) — no cross-replica lock.
  - JWT keys in memory, loaded-or-generated into `system_setting`
    (`security.rs:68-98`) — two cold replicas can both generate a key pair.
  - Notification providers cached in memory, reloaded only on the replica
    that handled the change (`notification/mod.rs:132-137`,
    `endpoints/notifications.rs:385`).
  - Network websocket: per-replica 1s broadcaster + caches
    (`network_broadcaster.rs:74`, `state.rs:29,128`) — correct per replica,
    just duplicated load.
  - `Migrator::up` runs on every connect in backend and worker
    (`application/database.rs:30`) — concurrent migrations possible.
  - Sessions are in the DB (fine). No rate limiting.
- **openresty:** `lua_shared_dict kubarr_lookups` per-pod cache with TTL —
  fine with multiple replicas.
- **Worker:** loops in `runtime.rs:46-131` (chart sync, pod_watch, stale-op
  recovery, operation poll/reconcile/audit outbox, DomainReconciler). Ops are
  claimed with a conditional UPDATE, but chart hard-codes `replicas: 1` +
  `strategy: Recreate` and `app_manager.rs:964-965` assumes one worker. No
  Lease, advisory lock or SKIP LOCKED.
- **Postgres:** release `postgresql` (ns `default`), StatefulSet `kubarr-db`
  in `kubarr-database`, service `kubarr-db-rw.kubarr-database.svc:5432`, secret
  `kubarr-db-app` key `uri` → `KUBARR_DATABASE_URL`. These names match
  CNPG conventions for a Cluster named `kubarr-db`.
  Data on NFS subdir `system/postgresql`. NetworkPolicy allows only
  backend/worker. CLI runs SQL via `kubectl exec … psql`
  (`db_bootstrap.rs:137-160`). No backup code anywhere.
- **Storage:** CLI installs no StorageClass; single-node k3s brings its own
  `local-path`. No CNI installed (Cilium detected/upgraded only).
- **HA primitives:** no PDB, topologySpreadConstraints or anti-affinity
  anywhere; only pass-through `affinity: {}`. `kubarr-common/_storage.tpl:352`
  fails if replicas ≠ 1 for charts with private config — must be relaxed or
  confirmed not to apply to HA'd charts.

## Node detection

- Count nodes that are: `Ready`, not cordoned (`spec.unschedulable` false),
  and without a `NoSchedule`/`NoExecute` taint Kubarr pods don't tolerate.
  Control-plane nodes count if they are schedulable (Talos/k3s setups often
  are — the author's 3 nodes are all control-plane and untainted).
- For scale-down decisions, count nodes that *exist* (any status) and meet the
  taint/cordon rules — so a NotReady node still counts and replicas are kept.
- Desired replica tiers:
  - 1 node: today's shape exactly.
  - 2 nodes: 2 replicas for stateless components, 2 CNPG instances.
  - 3+ nodes: 3 replicas for gateway/backend/frontend (see table), 3 CNPG
    instances. Cap at 3.

## Per-component behavior

| Component | 1 node | 2+ nodes |
|---|---|---|
| openresty (gateway) | 1 | `min(nodes, 3)`, spread, PDB maxUnavailable 1 |
| kubarr-backend (API) | 1 | `min(nodes, 3)`, spread, PDB |
| kubarr-frontend | 1 | 2, spread, PDB |
| kubarr-worker | 1 | 2, leader election (Lease); only leader runs loops |
| backend scheduler jobs | in-process | moved into worker (runs under worker leader) |
| cloudflared | 2 (today) | 2, add spread — chart-only; only matters if a user installs it |
| cert-manager (controller, webhook, cainjector) | 1 | 2, chart-native leader election, spread |
| kubarr-dns-webhook | 1 | 2, spread — chart-only, same caveat |
| PostgreSQL | CNPG, 1 instance | CNPG, `min(nodes, 3)` instances, local disk |
| fluent-bit, node-exporter | DaemonSet | unchanged (already per-node) |
| VictoriaMetrics/Logs, Grafana, vmalert, alertmanager | 1 | 1 (non-goal) |
| managed-nfs | 1 | 1 |
| media apps | 1 | 1 (non-goal) |

Spread: `topologySpreadConstraints` on `kubernetes.io/hostname` with
`whenUnsatisfiable: DoNotSchedule` for HA components when replicas ≤ nodes,
so two replicas never land on one node. Add as a shared helper in
`kubarr-common`. PDBs rendered only when replicas > 1 (a PDB with 1 replica
blocks node drains on single-node installs).

## PostgreSQL via CloudNativePG

- Kubarr installs the CNPG operator (pinned version, renovate-tracked) as a
  system chart, plus a chart rendering a CNPG `Cluster` named `kubarr-db` in
  namespace `kubarr-database`. This replaces `kubarr-charts/system/postgresql`,
  which is deleted.
- Naming: a Cluster named `kubarr-db` produces Service `kubarr-db-rw` and
  Secret `kubarr-db-app` — the names backend/worker already use. Clients need
  no change. The CLI's own `kubarr-db-app` secret creation
  (`db_credentials.rs`) must be reconciled with CNPG's generated secret
  (either CNPG creates it and the CLI copies it to `kubarr-backend` /
  `kubarr-worker`, or the CLI pre-creates it and the Cluster references it via
  `bootstrap.initdb.secret`) — builder's call, but exactly one owner.
- NetworkPolicy keeps allowing only backend/worker, plus CNPG operator and
  instance-to-instance replication traffic, plus the backup job.
- Storage: node-local disk. Use the cluster's default StorageClass if it is
  not NFS-backed (local-path, Longhorn, Ceph etc. all acceptable). If the only
  classes are NFS-backed or none exist, Kubarr installs Rancher
  `local-path-provisioner` with Talos-compatible config (data path under
  `/var`, namespace labelled for privileged pod security). Postgres data no
  longer lives on NFS.
- Instances: 1 on single node; `min(nodes, 3)` otherwise. CNPG handles
  replica creation, streaming replication, and failover. Pod anti-affinity:
  required across hostnames.
- Sizing: author's DB is ~25 MB; nodes have ~50 GiB free. Default volume
  small (builder's call, e.g. 5 Gi), expandable.

### Nightly backup

- A CronJob runs nightly `pg_dump` (custom format) against the `-rw`
  service and writes to the NFS share under `system/postgresql-backups/`,
  timestamped. Retention: keep last 7 (builder may make it configurable).
- Accepted loss window: up to one day of writes if a single-node install's
  disk dies. Multi-node installs are additionally covered by replicas.
- Restore path must exist and be tested: documented command (CLI subcommand
  or documented `pg_restore` steps) that restores the latest dump into a fresh
  `kubarr-db` cluster.
- Job failure surfaces somewhere visible (pod failure is enough for now;
  alert via vmalert if cheap).

## Autoscaling on node changes

- A reconciler watches Nodes. On change, recompute desired replicas per tier
  and apply (helm upgrade of affected system charts with new values, or patch
  replicas directly — builder's call, but helm state must not drift and later
  undo it).
- Scale up when the schedulable count rises. Scale down only when a Node
  object is deleted (or becomes permanently cordoned/tainted), never on
  NotReady.
- Debounce (e.g. a node must be Ready for ~2 minutes before triggering
  scale-up) to avoid flapping during reboots.
- CNPG scale-down removes a replica, never the primary first; rely on CNPG's
  own instance management.
- The reconciler is itself a singleton (runs under the worker's leader
  election).

## Constraints

- Must keep working on single-node k3s (the hosted-model VM path) with no
  behavior change beyond PostgreSQL moving to CNPG.
- Talos nodes in the author's cluster are untainted control-plane nodes with
  60 GiB disks; Proxmox `ssd_storage` backing cp-1/cp-2 is 93% full
  (133 GB free) — keep DB volumes small.
- Chart versions are pinned in `kubarr-cli/src/charts.rs`; new charts
  (CNPG operator, local-path) follow the same pinning + renovate pattern.
- Backend/worker/frontend run `:latest` with `imagePullPolicy: Always` today;
  replica changes must not break rolling restarts.

## Decided by the user

- Feature applies to every install, with automatic node-count detection.
- "Most things that make sense to run HA, run them HA."
- PostgreSQL moves to CloudNativePG.
- PostgreSQL replicas on node-local disk, not NFS.
- Automatic scaling when nodes are added later.
- Nightly `pg_dump` to NFS; up to one day of loss acceptable on single node.
- No migration: still in development, author's cluster will be wiped.

## Builder's calls (made in conversation, user may override)

- CNPG on every install, including single node (one code path).
- Replica cap of 3; tiers as in the table.
- Scale down only on node deletion, never NotReady.
- Install local-path-provisioner only when no suitable StorageClass exists.
- Monitoring storage, media apps, managed-nfs stay single replica.
- CNPG Cluster keeps the name `kubarr-db` (fresh installs only, so no clash).
- Use existing non-NFS default StorageClass; else install local-path with
  Talos-compatible config.
- Backup retention 7 nightly dumps.

## Backend and worker changes required for replicas

- Worker: 2 replicas, `strategy: RollingUpdate`, leader election via a
  Kubernetes Lease. Only the leader runs any loop (chart sync, pod_watch,
  stale-op recovery, operation poll, reconcile, audit outbox,
  DomainReconciler, node reconciler, moved scheduler jobs). Standby just
  waits for the lease. Leader must stop all loops if it loses the lease.
- Backend scheduler jobs (session_cleanup, audit_retention, ChartSyncTask)
  move into the worker. Backend API becomes stateless apart from caches.
- JWT key bootstrap: generate-and-insert must be race-safe (insert-if-absent
  then re-read, so all replicas end up with the stored key).
- Notification provider cache: invalidate across replicas (DB version
  counter checked per send, or Postgres LISTEN/NOTIFY — builder's call).
- Migrations: wrap `Migrator::up` in a Postgres advisory lock.
- Network broadcaster duplication across replicas is acceptable.

## Left to the builder

- Node reconciler lives in the worker (it already runs helm on system
  releases); exact mechanism for applying replica values (helm values
  overlay vs direct patch) is builder's call, provided later
  `--reset-then-reuse-values` upgrades don't undo it.
- Lease implementation (kube-rs or hand-rolled on coordination.k8s.io).
- Optional read-only "HA: n nodes, m replicas" indicator in the dashboard.
- Test strategy: chart unit tests for rendering per tier (1/2/3+ nodes),
  Rust tests for node counting and debounce, one e2e on a multi-node cluster
  (author's Talos cluster) including killing a node and verifying dashboard +
  DB stay up.
