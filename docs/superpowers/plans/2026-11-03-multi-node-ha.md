# Multi-node HA Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kubarr detects schedulable node count and runs system components highly-available on 2+ nodes, with PostgreSQL on CloudNativePG, node-local storage, nightly pg_dump to NFS, and automatic rescale on node changes.

**Architecture:** One Helm value, `kubarr.ha.nodeCount`, drives every HA chart through shared `kubarr-common` helpers (replicas, spread, PDB). The CLI sets it at install; a leader-elected worker loop recomputes it from Nodes and re-applies it with `helm upgrade --reuse-values` at the installed chart version. Backend becomes replica-safe (no scheduler, race-free JWT bootstrap, notification cache freshness, migration lock); worker becomes 2 replicas behind a Kubernetes Lease.

**Tech Stack:** Rust (axum, sea-orm 2, kube 4.2, k8s-openapi 0.28), Helm charts (python unittest render tests in `kubarr-charts/tests`), CloudNativePG operator, Rancher local-path-provisioner.

**Spec:** `Kubarr/docs/superpowers/specs/2026-11-03-multi-node-ha-design.md`

**Repos / branches:** create `feat/multi-node-ha` in `Kubarr/` (from `release/v0.3.0`), `kubarr-charts/` and `kubarr-cli/` (from `main`). Do not touch the uncommitted `kubarr-cli` changes in `.github/workflows/ci.yml` and `src/registry.rs` — leave them unstaged.

## Global Constraints

- Value name everywhere: `kubarr.ha.nodeCount` (int, default 1). Absent or ≤1 ⇒ today's single-replica shape exactly: no PDB, no spread constraints, no extra pods.
- Replica tiers: `replicas = nodeCount<=1 ? 1 : min(nodeCount, max)` where max = 3 for openresty, kubarr-backend, postgres instances; 2 for kubarr-frontend, kubarr-worker, cert-manager (controller/webhook/cainjector), kubarr-dns-webhook. cloudflared keeps `replicaCount: 2` always.
- Spread: `topologySpreadConstraints` on `kubernetes.io/hostname`, `maxSkew: 1`, `whenUnsatisfiable: DoNotSchedule`, only when replicas > 1.
- PDB: `maxUnavailable: 1`, only when replicas > 1.
- Never scale down on NotReady; only when a Node is deleted or permanently cordoned/tainted.
- Scale-up debounce: a node counts only after its Ready condition has been True for ≥ 120 s.
- CNPG Cluster name `kubarr-db`, namespace `kubarr-database`, release `postgresql` (chart `system/postgresql` is rewritten in place, keeping the release name the CLI and backend already use). Clients keep `kubarr-db-rw.kubarr-database.svc:5432` and secret `kubarr-db-app` key `uri`.
- Postgres storage: cluster default StorageClass if not NFS-backed, else install local-path-provisioner (Talos-compatible: path under `/var/local-path-provisioner`, namespace labelled `pod-security.kubernetes.io/enforce=privileged`). Default volume 5Gi.
- Backup: nightly CronJob `pg_dump -Fc` to NFS `system/postgresql-backups/kubarr-<UTC timestamp>.dump`, keep last 7.
- No migration of existing installs. The author's cluster gets wiped.
- Lease: `coordination.k8s.io/v1` Lease `kubarr-worker-leader` in the worker namespace, duration 15 s, renew every 5 s.
- Postgres migration advisory lock key: `0x6B75_6261_7272` (i64, "kubarr").

## Review Focus

1. Single-node install: renders and behaves identically to today (no PDB that blocks drains, no pending second replica). → chart render test per HA chart with nodeCount unset and =1 (Task 6), CLI test that 1 node passes `nodeCount=1` (Task 9).
2. Node goes NotReady (the actual HA failure): replica count must not drop. → `desired_node_count` test in Task 5.
3. Worker leader dies without releasing its Lease (node crash): standby takes over after lease expiry, not never. → `decide` test with expired lease in Task 1.
4. Worker rolling update: new pod must not run loops until old pod has drained and released. → `decide` test with live foreign lease + release-on-shutdown test in Task 1.
5. Two backend replicas cold-starting on an empty DB: both end with the same JWT key pair. → Task 3 test with pre-existing rows.

---

### Task 1: Worker leader election (Kubarr)

**Files:**
- Create: `Kubarr/code/api/src/services/leader.rs`
- Modify: `Kubarr/code/api/src/services/mod.rs` (export), `Kubarr/code/api/src/application/runtime.rs` (`run_worker`)

**Interfaces:**
- Produces:
  - `pub enum LeaseAction { Acquire, Renew, Wait }`
  - `pub fn decide(spec: Option<&LeaseSpec>, me: &str, now: DateTime<Utc>, duration: Duration) -> LeaseAction` — Acquire if no lease / no holder / holder's `renewTime + leaseDurationSeconds < now`; Renew if holder == me; else Wait.
  - `pub struct LeaderElector { client: kube::Client, namespace: String, name: String, identity: String, duration: Duration, renew_every: Duration }`
  - `pub async fn LeaderElector::acquire(&self, cancel: &CancellationToken) -> Result<LeaderGuard>` — loops until holder, using `resourceVersion` optimistic replace / create; returns `Err` if cancelled first.
  - `pub struct LeaderGuard { pub lost: CancellationToken }` — background renew task; cancels `lost` if a renew has not succeeded within `duration`.
  - `pub async fn LeaderGuard::release(self)` — sets `holderIdentity` to None if still ours.
- Identity: env `POD_NAME`, fallback hostname. Namespace: env `POD_NAMESPACE`, fallback `kubarr-worker`.

- [ ] **Step 1: Write failing unit tests in `leader.rs`**: `decide_acquires_missing_lease`, `decide_acquires_expired_foreign_lease` (renewTime now-20s, duration 15s ⇒ Acquire), `decide_waits_on_live_foreign_lease` (now-5s ⇒ Wait), `decide_renews_own_lease`, `decide_acquires_released_lease` (holder None ⇒ Acquire).
- [ ] **Step 2:** `cargo test -p kubarr leader::` → FAIL (not defined).
- [ ] **Step 3: Implement** `decide`, `LeaderElector`, `LeaderGuard` per interfaces.
- [ ] **Step 4: Wire into `run_worker`:** after `init_kubernetes()` and before `init_database`/chart sync/any loop, acquire leadership when a kube client exists (no cluster ⇒ run unguarded, as today for local dev). Any lost-lease ⇒ `cancellation.cancel()` then exit with error so the pod restarts as a standby. On normal shutdown, after `worker_tasks.wait()`, call `guard.release()`.
- [ ] **Step 5:** `cargo test -p kubarr leader:: && cargo clippy --all-targets -- -D warnings` → PASS.
- [ ] **Step 6: Commit** `feat(worker): elect a single active worker with a Lease`.

### Task 2: Move scheduler jobs to the worker + migration advisory lock (Kubarr)

**Files:**
- Modify: `Kubarr/code/api/src/services/scheduler.rs`, `Kubarr/code/api/src/application/runtime.rs`, `Kubarr/code/api/src/application/database.rs`
- Test: `Kubarr/code/api/tests/migration_lock_postgres_tests.rs` (new)

**Interfaces:**
- `pub fn start_scheduler(db: Arc<DatabaseConnection>, chart_sync: Arc<ChartSyncService>, cancel: CancellationToken)` — tasks stop when cancelled. Called from `run_worker` after leadership; removed from `init_services`.
- `connect_with_url` on Postgres: open a separate 1-connection pool, `SELECT pg_advisory_lock(0x6B7562617272)`, run `Migrator::up` on the main pool, `pg_advisory_unlock`, close. SQLite path unchanged.

- [ ] **Step 1: Failing test** `concurrent_connects_both_migrate_cleanly` in the new test file, gated on `MIGRATION_LOCK_TEST_POSTGRES_URL` (skip with early return when unset, same pattern as `pod_cache_postgres_tests.rs`): two `connect_with_url` in `tokio::join!` on a fresh DB both return Ok.
- [ ] **Step 2:** run against a disposable local postgres (`podman run --rm -d -p 55432:5432 -e POSTGRES_PASSWORD=t postgres:16-alpine`) → expect FAIL or flake without lock (if it passes by luck, keep test; lock is still required).
- [ ] **Step 3: Implement** lock + scheduler move.
- [ ] **Step 4:** `cargo test` (full) + gated test with URL set → PASS. Confirm `grep -n start_scheduler src/application/runtime.rs` shows only the worker call.
- [ ] **Step 5: Commit** `fix(backend): run periodic jobs only in the elected worker and serialize migrations`.

### Task 3: Race-free JWT key bootstrap (Kubarr)

**Files:** Modify `Kubarr/code/api/src/services/security.rs`.

**Interfaces:** `async fn persist_or_load_jwt_keys(db: &DatabaseConnection, generated: (String, String)) -> Result<(String, String)>` — in one transaction: insert both rows with `ON CONFLICT (key) DO UPDATE ... WHERE system_setting.value = ''` (empty rows still get filled), then re-read both rows and return the stored values. `init_jwt_keys` uses the returned pair.

- [ ] **Step 1: Failing test** `persist_returns_existing_keys_not_generated`: pre-insert non-empty `jwt_private_key`/`jwt_public_key` = ("P1","Q1"); call with ("P2","Q2"); assert returns ("P1","Q1") and DB unchanged. Keep `init_jwt_keys_updates_existing_empty_key_rows` passing.
- [ ] **Step 2:** run → FAIL. **Step 3:** implement. **Step 4:** `cargo test security` → PASS.
- [ ] **Step 5: Commit** `fix(auth): let concurrent backends converge on one JWT key pair`.

### Task 4: Notification provider freshness across replicas (Kubarr)

**Files:** Modify `Kubarr/code/api/src/services/notification/mod.rs`. Test in `Kubarr/code/api/tests/notification_service_tests.rs`.

**Interfaces:**
- `init_providers` first resets all three providers to `None` (fixes disabled channels lingering).
- `pub async fn refresh_if_changed(&self) -> Result<()>` — fingerprint = `(count(*), max(updated_at))` over `notification_channel`; reload via `init_providers` when it differs from the cached fingerprint (`RwLock<Option<(i64, Option<DateTime<Utc>>)>>`). Called at the start of every public send path.

- [ ] **Step 1: Failing test** `second_instance_sees_channel_change`: two `NotificationService` on the same sqlite file DB; enable telegram via DB write; instance B `refresh_if_changed()` ⇒ telegram provider present; disable ⇒ absent.
- [ ] **Step 2–4:** fail → implement → `cargo test notification` pass.
- [ ] **Step 5: Commit** `fix(notifications): reload providers when another replica changes them`.

### Task 5: Node topology + HA rescale loop (Kubarr)

**Files:**
- Create: `Kubarr/code/api/src/services/ha.rs`
- Modify: `Kubarr/code/api/src/services/mod.rs`, `Kubarr/code/api/src/services/app_manager.rs` (shared helm lock), `Kubarr/code/api/src/application/runtime.rs`

**Interfaces:**
- `pub const HA_NODE_COUNT_SETTING: &str = "ha_node_count";`
- `pub const HA_RELEASES: &[&str] = &["openresty", "kubarr-backend", "kubarr-frontend", "kubarr-worker", "postgresql", "cert-manager", "kubarr-dns-webhook", "cloudflared"];` — only those currently installed are touched.
- `pub fn schedulable_node_count(nodes: &[Node], now: DateTime<Utc>) -> u32` — Ready=True for ≥120 s, `spec.unschedulable != true`, no taint with effect NoSchedule/NoExecute.
- `pub fn retained_node_count(nodes: &[Node]) -> u32` — any Ready status, same cordon/taint rules.
- `pub fn desired_node_count(applied: u32, schedulable: u32, retained: u32) -> u32` — `schedulable > applied ⇒ schedulable`; `retained < applied ⇒ max(retained, 1)`; else `applied`. Result ≥ 1.
- `pub async fn reconcile_ha(manager: &AppManager) -> Result<Option<u32>>` — reads setting `ha_node_count` (absent ⇒ applied = 1; the CLI writes it at install, Task 9), computes desired, and if changed: for each installed HA release, under `AppManager`'s helm lock, `helm upgrade <release> <chart_ref> -n default --version <installed> --reuse-values --set kubarr.ha.nodeCount=<n>` (no `--wait`; for `kubarr-worker` mirror the existing `wait=false` self-update rule). Writes the setting only after all succeed. Returns new count when changed.
- `pub fn extra_ha_values(release: &str, n: u32) -> Vec<String>` — for `cert-manager` only: `cert-manager.replicaCount`, `cert-manager.webhook.replicaCount`, `cert-manager.cainjector.replicaCount` = min(n,2) when n>1 (else 1), plus `cert-manager.podDisruptionBudget.enabled=true` and hostname topology spread when n>1; empty for every other release. Test `cert_manager_gets_subchart_replica_values`.
- `AppManager` gains `helm_lock: tokio::sync::Mutex<()>` held by `execute_operation` and `reconcile_ha`.
- Worker runs `reconcile_ha` every 30 s under leadership.

- [ ] **Step 1: Failing unit tests**: `counts_ready_untainted_nodes`, `skips_cordoned_and_noschedule_tainted`, `debounces_recently_ready` (Ready 60 s ago ⇒ not counted), `notready_node_is_retained` (retained counts it, schedulable doesn't), `desired_scales_up`, `desired_holds_on_notready` (applied 3, schedulable 2, retained 3 ⇒ 3), `desired_scales_down_on_deletion` (applied 3, retained 2 ⇒ 2), `desired_never_below_one`. Plus a helm-args test `ha_upgrade_args_pin_version_and_reuse_values` for a pure `fn ha_upgrade_args(release, chart_ref, version, n) -> Vec<String>`.
- [ ] **Step 2–4:** fail → implement → `cargo test ha::` pass; `cargo clippy` clean.
- [ ] **Step 5: Commit** `feat(worker): rescale HA system releases when nodes join or leave`.

### Task 6: kubarr-common HA helpers + HA wiring in system charts (kubarr-charts)

**Files:**
- Create: `kubarr-charts/system/kubarr-common/templates/_ha.tpl`, `kubarr-charts/tests/test_ha_rendering.py`
- Modify: `system/openresty`, `system/kubarr-backend`, `system/kubarr-frontend`, `system/kubarr-worker`, `system/kubarr-dns-webhook`, `system/cloudflared` (values + deployment + new `pdb.yaml`), `system/cert-manager/values.yaml` + template if needed, Chart.yaml version bumps; `system/kubarr-worker/templates/rbac.yaml` (or new) and deployment env.

**Interfaces (templates):**
- `kubarr-common.ha.nodeCount` (root) → int from `.Values.kubarr.ha.nodeCount`, default 1.
- `kubarr-common.ha.replicas` (dict "root" . "max" N) → int per Global Constraints.
- `kubarr-common.ha.spread` `(dict "root" . "replicas" R "selector" <labels dict>)` → `topologySpreadConstraints:` block or empty.
- `kubarr-common.ha.pdb` (dict "root" . "replicas" R "name" … "namespace" … "selector" …) → PDB or empty.
- Charts replace <code v-pre>replicas: {{ .Values.replicaCount }}</code> with the helper; keep `replicaCount` as an explicit override only if a chart already documents it (openresty/backend/frontend: helper wins unless `replicaCount` explicitly > 1 — builder may simply drop `replicaCount`).
- kubarr-worker: `strategy: RollingUpdate` (maxSurge 1, maxUnavailable 0), env `POD_NAME`/`POD_NAMESPACE` from downward API, Role for `leases` (get, create, update) in its namespace, ClusterRole add `nodes` (list, watch, get). Update the `app_manager.rs:964-965` comment in Task 5's PR, not here.
- cert-manager: upstream subchart values cannot call helpers; set `replicaCount`/`webhook.replicaCount`/`cainjector.replicaCount` via the reconciler instead: Task 5's `reconcile_ha` additionally passes `--set cert-manager.replicaCount=R,cert-manager.webhook.replicaCount=R,cert-manager.cainjector.replicaCount=R` and `podDisruptionBudget.enabled=true` when R>1 for release `cert-manager` (record this in Task 5 when implementing — the helper call there is `fn extra_ha_values(release, n) -> Vec<String>`). Add topology spread via upstream `topologySpreadConstraints` values in the same way.
- Check `kubarr-common.storage.validateConfig` replicas guard: none of the HA charts may pass `replicas` > 1 into it; if any does, fail the chart test rather than loosen the guard.

- [ ] **Step 1: Failing tests** in `test_ha_rendering.py` (helm template, same harness as existing tests): for each HA chart, `nodeCount` unset ⇒ replicas 1, no PDB, no `topologySpreadConstraints`; `nodeCount=2` ⇒ replicas 2, PDB maxUnavailable 1, spread on hostname; `nodeCount=5` ⇒ capped (3 or 2 per chart). Worker: RollingUpdate + lease Role present.
- [ ] **Step 2:** `cd kubarr-charts && python -m pytest tests/test_ha_rendering.py` → FAIL.
- [ ] **Step 3:** implement helpers + chart wiring; bump chart patch versions.
- [ ] **Step 4:** run new test + full `python -m pytest tests -q --ignore=tests/e2e` → PASS.
- [ ] **Step 5: Commit** `feat(charts): scale system components across nodes`.

### Task 7: CloudNativePG operator chart + postgres chart as CNPG Cluster + nightly backup (kubarr-charts)

**Files:**
- Create: `kubarr-charts/system/cloudnative-pg/` (wrapper chart, upstream `cloudnative-pg` dependency from `https://cloudnative-pg.github.io/charts`, pinned latest stable, `kubarr.io/system: "true"`, hidden).
- Rewrite: `kubarr-charts/system/postgresql/` — templates: `cluster.yaml` (CNPG `Cluster` `kubarr-db`), `backup-cronjob.yaml`, `networkpolicy.yaml`, `namespace.yaml`; delete `statefulset.yaml`, `service.yaml`, `secret.yaml`, `vpa.yaml` (CNPG owns services). Bump major chart version.
- Update: `kubarr-charts/tests/chart_artifacts.py` DATABASE_CHARTS and any storage-contract tests that assumed the NFS data mount.
- Test: `kubarr-charts/tests/test_postgres_cnpg.py`

**Values contract:**
- `kubarr.ha.nodeCount` → `spec.instances` via `kubarr-common.ha.replicas` max 3.
- `database.existingSecret` (required, fails if empty) → `spec.bootstrap.initdb.secret.name`; DB/owner `kubarr`.
- `persistence.size` (5Gi), `persistence.storageClassName` ("" ⇒ omit, cluster default).
- `spec.affinity.podAntiAffinityType: required`, `topologyKey: kubernetes.io/hostname`.
- `backup.enabled` (true), `backup.schedule` ("17 3 * * *"), `backup.retain` (7), `backup.image` (postgres 16 alpine, same major as `imageName` of the Cluster). CronJob mounts NFS via existing `kubarr-common.storage` media claim helpers at source `system/postgresql-backups`, connects to `kubarr-db-rw` with `uri` from the existing secret, writes `kubarr-$(date -u +%Y%m%dT%H%M%SZ).dump` via `pg_dump -Fc`, writes to a temp name then renames, then deletes all but newest 7 `kubarr-*.dump`.
- NetworkPolicy ingress on 5432: kubarr-backend, kubarr-worker namespaces, the backup job pods, CNPG instance pods (replication, 5432 and 8000 status), and the operator namespace (8000).

- [ ] **Step 1: Failing tests**: render with nodeCount unset ⇒ `instances: 1`; nodeCount 3 ⇒ 3, anti-affinity required; empty `existingSecret` ⇒ render fails; CronJob present with schedule, retention command keeps 7, mounts `system/postgresql-backups`; no StatefulSet rendered.
- [ ] **Step 2–4:** fail → implement → full chart test suite passes (`helm dependency build system/cloudnative-pg` first).
- [ ] **Step 5: Commit** `feat(postgresql): run the database on CloudNativePG with nightly dumps to NFS`.

### Task 8: local-path-provisioner chart (kubarr-charts)

**Files:** Create `kubarr-charts/system/local-path-provisioner/` — vendored upstream manifests (rancher/local-path-provisioner, pinned latest stable image) templated: namespace `local-path-storage` with label `pod-security.kubernetes.io/enforce: privileged`, ConfigMap path `/var/local-path-provisioner`, StorageClass `local-path` (`WaitForFirstConsumer`, reclaim `Delete`), `storageClass.default` value (false by default — CLI decides). Test `tests/test_local_path.py`: path under `/var`, namespace label, StorageClass binding mode.

- [ ] Steps: failing test → implement → pass → commit `feat(storage): ship a Talos-safe local-path provisioner`.

### Task 9: CLI install flow (kubarr-cli)

**Files:**
- Create: `kubarr-cli/src/topology.rs` (node count), `kubarr-cli/src/db_restore.rs`
- Modify: `src/bootstrap.rs` (order), `src/database.rs`, `src/db_bootstrap.rs`, `src/db_credentials.rs`, `src/install.rs`, `src/observability.rs` only if it installs HA charts (it doesn't — leave), `src/charts.rs` (pins for `cloudnative-pg`, `local-path-provisioner`, bumped `postgresql` and HA charts), `src/main.rs` (subcommand), `src/types.rs`.

**Interfaces:**
- `pub fn schedulable_node_count_from_json(nodes_json: &str) -> Result<u32, String>` — same rules as Task 5 minus debounce (install time); `pub fn detect_node_count(dry_run: bool) -> Result<u32, String>` via `kubectl get nodes -o json`; dry-run ⇒ 1.
- `pub fn needs_local_path(storage_classes_json: &str) -> bool` — true when no default StorageClass, or the default's provisioner is NFS-backed (`kubarr.dev/nfs-subdir`, `nfs.csi.k8s.io`, `cluster.local/nfs-subdir-external-provisioner`, or contains `nfs`).
- Every HA release install (openresty, kubarr-backend, kubarr-frontend, kubarr-worker, postgresql) gets `--set kubarr.ha.nodeCount=<n>`.
- Bootstrap order: managed-nfs → [local-path if needed, then set it default only if there was no default] → cloudnative-pg operator (wait for its deployment + CRD) → observability → DB secret → postgresql (wait: `kubectl wait --for=condition=Ready cluster.postgresql.cnpg.io/kubarr-db -n kubarr-database --timeout=10m`) → backend/frontend/openresty/worker.
- DB secret: CLI stays owner of `kubarr-db-app`; type `kubernetes.io/basic-auth` with keys `username` (`kubarr`), `password`, `uri`. `uri` host stays `kubarr-db-rw.kubarr-database.svc`.
- `db_bootstrap.rs` exec target: primary pod via `kubectl get pod -n kubarr-database -l cnpg.io/cluster=kubarr-db,cnpg.io/instanceRole=primary -o name`; psql as `postgres` superuser locally (`psql -U postgres -d kubarr`) — replace every `statefulset/kubarr-db`. `db_credentials.rs` `read_cluster_state` detects an existing DB via the CNPG Cluster object instead of the StatefulSet.
- After install, write `ha_node_count=<n>` into `system_setting` in the same SQL bootstrap step so the worker's first reconcile is a no-op.
- `kubarr db restore [--file <name>]`: runs a one-off pod mounting the NFS backups path, `pg_restore --clean --if-exists -d <uri>` from newest (or named) dump; scales backend/worker to 0 first and back after.

- [ ] **Step 1: Failing tests**: `topology` (fixture JSON: 3 Ready nodes, one cordoned, one NoSchedule-tainted ⇒ 1; …), `needs_local_path` (no default ⇒ true; default `rancher.io/local-path` ⇒ false; default `kubarr.dev/nfs-subdir` ⇒ true), bootstrap plan test (existing `every_remote_bootstrap_release_uses_an_explicit_chart_revision` extended: `cloudnative-pg` precedes `postgresql`, HA releases carry `kubarr.ha.nodeCount`), db_credentials manifest test asserts basic-auth type + `username`, db_restore args test.
- [ ] **Step 2–4:** fail → implement → `cargo test && cargo clippy --all-targets -- -D warnings` pass.
- [ ] **Step 5: Commit** `feat(bootstrap): install HA-aware system components on CloudNativePG`.

### Task 10: End-to-end on the Talos cluster

Author's cluster may be wiped (authorized). Kubeconfig `~/kubarr-cluster/kubeconfig`, talosconfig `~/kubarr-cluster/talosconfig`, 3 nodes 192.168.1.20-22.

- [ ] **Step 1:** build/publish: push branches, let CI publish images/charts, or use CLI local chart overrides + locally built images pushed to the registry CI uses. Record which.
- [ ] **Step 2:** uninstall all Kubarr releases + namespaces, wipe NFS `/mnt/hdd_storage/system/postgresql` (old data; confirm path first), reinstall with CLI.
- [ ] **Step 3: Verify:** `kubectl get deploy -A` shows openresty/backend 3, frontend/worker 2; `kubectl get cluster -n kubarr-database` 3 instances Ready on 3 distinct nodes; PDBs present; one worker holds Lease; `system_setting.ha_node_count=3`.
- [ ] **Step 4: Failure test:** `talosctl -n 192.168.1.22 shutdown` (bertha's node). Within 60 s dashboard login + app list work via https://void-zero.com; replicas unchanged (no scale-down); if the DB primary was on cp-3, CNPG promotes a replica. Bring node back (`qm start 112` on bertha).
- [ ] **Step 5:** trigger backup job manually (`kubectl create job --from=cronjob/... `), confirm dump on NFS; run `kubarr db restore` against it on the live cluster; dashboard still works.
- [ ] **Step 6:** record results in the spec's status line; commit.
