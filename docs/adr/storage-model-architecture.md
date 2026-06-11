# ADR: Storage Model Architecture for Kubarr

**Status:** Proposed
**Date:** 2026-01-29
**Authors:** Auto-Claude
**Deciders:** Kubarr maintainers

## Context

Kubarr is a Kubernetes-native homelab management platform designed for single-pod deployment serving 1-5 users. The backend is built with Rust/Axum and currently relies on three storage layers: a PostgreSQL relational database, in-memory caches, and file-system volumes. This ADR documents the current architecture, identifies issues, and evaluates alternative storage models.

### Decision Drivers

- **Operational simplicity** - Homelab operators, not SREs, manage this system
- **Resource efficiency** - Minimize memory and CPU overhead on constrained hardware
- **Data durability** - Protect against data loss without complex backup infrastructure
- **Single-pod architecture** - No horizontal scaling; one backend pod serves all traffic
- **Migration feasibility** - Any change must handle 22 existing entity models and 22 migrations

---

## Current Architecture

### 1. PostgreSQL Database (CloudNativePG 1.28)

**Setup:** PostgreSQL is managed by the CloudNativePG (CNPG) operator v1.28 running in Kubernetes. The backend connects via SeaORM 1.1 with both `sqlx-postgres` and `sqlx-sqlite` feature flags compiled in `Cargo.toml`.

**Connection Management** (`code/backend/src/application/database.rs`):

```rust
let mut opts = ConnectOptions::new(database_url);
opts.max_connections(10)
    .min_connections(1)
    .connect_timeout(Duration::from_secs(30))
    .idle_timeout(Duration::from_secs(600))
    .sqlx_logging(false);
```

- **Pool size:** Hardcoded at 10 max / 1 min connections - not configurable via environment variables
- **Timeouts:** 30s connect, 600s idle
- **SQL logging:** Disabled (no query-level observability)
- **Retry logic:** None on initial connection failure; `try_connect()` provides graceful degradation with a 5s timeout during bootstrap
- **Migrations:** Run automatically on connect via `Migrator::up(&db, None)`

**Entity Models** (`code/backend/src/models/`): 22 entities covering:

| Category | Models |
|----------|--------|
| Auth & Users | `user`, `session`, `role`, `user_role`, `role_permission`, `role_app_permission`, `invite`, `pending_2fa_challenge`, `user_preferences` |
| OAuth | `oauth_account`, `oauth_provider` |
| Notifications | `notification_channel`, `notification_event`, `notification_log`, `user_notification`, `user_notification_pref` |
| System | `system_setting`, `server_config`, `bootstrap_status`, `audit_log` |
| VPN | `vpn_provider`, `app_vpn_config` |

All models use standard SeaORM patterns: `DeriveEntityModel` with `Serialize`/`Deserialize`, `i64` primary keys (PostgreSQL `bigint`), and `DateTimeUtc` timestamps.

**Migrations** (`code/backend/src/migrations/`): 22 ordered migrations (including a seed-defaults migration) covering schema creation from `m20260127_000001` through `m20260130_000002`.

**CNPG RBAC** (`charts/kubarr/values.yaml`):

```yaml
- apiGroups: ["postgresql.cnpg.io"]
  resources: ["clusters", "backups", "scheduledbackups", "poolers"]
  verbs: ["get", "list", "watch", "create", "update", "delete", "patch"]
```

The backend has full CRUD access to CNPG resources, enabling programmatic database cluster management from within the application.

### 2. In-Memory Caching (`code/backend/src/application/state.rs`)

Two cache implementations exist in `AppState`:

#### EndpointCache

Caches Kubernetes service endpoint lookups to avoid K8s API calls on every proxy request.

```rust
pub struct EndpointCache {
    cache: Arc<RwLock<HashMap<String, CachedEndpoint>>>,
    ttl: Duration, // 60 seconds
}
```

- **TTL:** 60 seconds, checked on read only (lazy expiration)
- **Eviction:** None - expired entries remain in memory until overwritten by a new `set()` call
- **Size limit:** None - unbounded growth potential
- **Persistence:** Lost on pod restart

#### NetworkMetricsCache

Stores cumulative network counters and computes sliding-window rate averages for dashboard display.

```rust
pub struct NetworkMetricsCache {
    cache: Arc<RwLock<HashMap<String, CachedNetworkMetrics>>>,
    max_age: Duration, // 5 minutes
}
```

- **Staleness:** 5-minute max age, checked on read
- **Rate calculation:** Sliding window of 5 samples with EMA smoothing
- **Eviction:** None - stale entries remain in the HashMap
- **Size limit:** None - one entry per namespace, practical bound is cluster size

#### Additional Shared State

`AppState` also holds several `Arc<RwLock<T>>` wrappers:

- `SharedDbConn` - Optional database connection (deferred until PostgreSQL is available)
- `SharedK8sClient` - Kubernetes client
- `SharedCatalog` - Application catalog
- `NetworkMetricsBroadcast` / `BootstrapBroadcast` - Tokio broadcast channels for WebSocket clients

### 3. File-System Storage (`code/backend/src/endpoints/storage.rs`)

File browsing and download endpoints serve media from a configurable storage path.

- **Configuration:** Storage root from `system_settings` table or `KUBARR_STORAGE_PATH` env var (default: `/data`)
- **Volume type:** hostPath volumes in Kubernetes (no CSI, no PVC abstraction)
- **Security:** Path traversal prevention via canonicalization and base-path containment checks
- **Protected folders:** `downloads`, `media` directories cannot be deleted
- **File operations:** Browse, get info, create directory, delete (empty dirs/files only), stream download
- **Disk stats:** Uses `df -B1` system command for storage usage reporting
- **Permissions:** Role-based (StorageView, StorageWrite, StorageDelete, StorageDownload)

### 4. Session & Authentication Layer (`code/backend/src/middleware/auth.rs`)

Every authenticated request performs:

1. Extract JWT session token from cookies (supports indexed multi-session: `kubarr_session_0`, `kubarr_session_1`, ...)
2. Decode and validate JWT claims
3. **Database lookup:** `Session::find_by_id(&claims.sid).one(&db)` - hits PostgreSQL on every request
4. Validate session is not revoked and not expired
5. **Database lookup:** `User::find_by_id(session.user_id)` with active/approved filters
6. **Fire-and-forget update:** `session.last_accessed_at` written asynchronously
7. **Database lookups:** Fetch all user roles, role permissions, and app permissions (3+ queries)

**No caching exists at the auth layer.** Every authenticated request triggers 4+ database queries. For a homelab with 1-5 users, this is functionally acceptable but architecturally wasteful.

### 5. Key-Value Configuration (`code/backend/src/models/system_setting.rs`)

System settings use a string-keyed key-value pattern:

```rust
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    pub value: String,
    pub description: Option<String>,
    pub updated_at: DateTimeUtc,
}
```

Used for dynamic configuration (storage path, OAuth2 settings, JWT keys) without requiring schema migrations.

---

## Identified Issues

### Performance Issues

1. **Audit stats loads all logs into memory** - `get_audit_stats()` in `audit.rs` (line 281) executes `audit_log::Entity::find().all(db).await?` to compute top actions by counting in application code. This fetches every audit log row into memory instead of using a SQL `GROUP BY` aggregation. As the audit log grows, this becomes an unbounded memory allocation that will eventually cause OOM or severe latency spikes.

   ```rust
   // Current: loads ALL logs into memory to count actions
   let all_logs = audit_log::Entity::find().all(db).await?;
   let mut action_counts: HashMap<String, u64> = HashMap::new();
   for log in all_logs {
       *action_counts.entry(log.action.clone()).or_insert(0) += 1;
   }
   ```

2. **No auth-layer caching** - 4+ database queries per authenticated request for session, user, and permissions lookups. Every request hits PostgreSQL for session validation, user fetch, roles, and permissions - no in-memory caching of auth context.

3. **Hardcoded connection pool** - Pool size is hardcoded at 10 max / 1 min connections in `database.rs` and not configurable via environment variables. For a homelab with 1-5 users, 10 connections is excessive and wastes PostgreSQL memory (~10MB per connection). Conversely, if the application's concurrency needs change, a code change and rebuild is required.

4. **No automatic cache eviction** - Both `EndpointCache` and `NetworkMetricsCache` use lazy expiration (staleness checked on read only). Expired entries are never removed from the underlying `HashMap`. There is no background eviction task, no `remove()` on stale reads, and no periodic cleanup. Over time, the cache accumulates dead entries that consume memory without providing value.

5. **Unbounded cache growth** - Neither cache implementation has a maximum size limit. While `NetworkMetricsCache` is practically bounded by the number of Kubernetes namespaces, `EndpointCache` has no such natural bound. There is no LRU eviction, no max-entries cap, and no memory pressure monitoring.

6. **Audit log growth** - No automatic retention policy. `clear_old_logs()` exists but must be called manually via the admin API. Without scheduled cleanup, the audit_log table grows indefinitely, compounding the `get_audit_stats` memory issue above.

### Architectural Limitations

1. **CNPG operator overhead** - Running the CloudNativePG operator adds significant resource consumption and operational complexity for what is fundamentally a single-pod, single-user embedded database workload. The operator itself runs as a separate deployment with its own CPU/memory allocation, watches CRDs, and manages failover logic that is unnecessary when there is only one database pod. For a homelab, this is a disproportionate amount of infrastructure for the problem being solved.

2. **Cache data lost on pod restart** - All cached data (endpoint lookups, network metrics history, rate calculations) is lost when the backend pod restarts. This causes a cold-start penalty: the first requests after restart must re-populate caches via K8s API calls and metric collection. Network rate calculations need multiple samples before producing accurate sliding-window averages, meaning the dashboard shows incomplete data for several polling intervals after restart.

3. **No connection retry logic** - If PostgreSQL becomes temporarily unavailable after the initial connection is established (e.g., during a CNPG switchover, brief network partition, or pod reschedule), there is no reconnection logic. The `try_connect()` function provides graceful degradation during bootstrap but does not handle mid-operation disconnects. SeaORM's underlying connection pool (sqlx) does handle some reconnection, but the application has no explicit retry-with-backoff strategy for transient failures.

4. **hostPath coupling** - File storage is tied to the node's filesystem with no portability across nodes. If the pod is rescheduled to a different node, storage is lost. No PVC abstraction or CSI driver integration exists.

5. **SQLite already compiled** - Both `sqlx-postgres` and `sqlx-sqlite` feature flags are enabled in `Cargo.toml`, but only PostgreSQL is used at runtime. This means the binary already includes SQLite support, making a potential migration to SQLite a smaller lift than it might otherwise be.

### Operational Complexity

1. **CNPG dependency** - Requires installing and managing the CNPG operator in the cluster before Kubarr can be deployed. This adds a prerequisite step that complicates initial setup for homelab users.
2. **Backup configuration** - CNPG backups require separate configuration (S3-compatible storage, scheduled backups, retention policies). Without this, there is no automated backup - a risk for homelab users who may not configure it.
3. **Resource footprint** - CNPG operator pod + PostgreSQL pod(s) consume memory (~256MB+ for PostgreSQL, ~128MB for the operator) and CPU beyond what the application itself needs. On constrained homelab hardware (e.g., Raspberry Pi, mini PC), this overhead is significant.
4. **Port forwarding fragility** - Development workflow requires manual port-forward restart after every deployment, adding friction to the development cycle.

---

## Option A: Optimized PostgreSQL

**Summary:** Keep the current PostgreSQL/CloudNativePG stack but address all identified performance issues, tune CNPG configuration, and improve operational reliability. This is the lowest-risk path that delivers meaningful improvements without architectural changes.

### Quick Wins (Application-Level)

These improvements can be implemented independently with minimal risk and no schema changes.

#### 1. Environment-Configurable Connection Pool

The current hardcoded pool in `database.rs` prevents tuning without a code change and rebuild:

```rust
// Current: hardcoded values
opts.max_connections(10)
    .min_connections(1)

// Proposed: env-configurable with sensible defaults
let max_conns = std::env::var("KUBARR_DB_MAX_CONNECTIONS")
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(5); // Reduced default: 5 is sufficient for 1-5 users

let min_conns = std::env::var("KUBARR_DB_MIN_CONNECTIONS")
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(1);

opts.max_connections(max_conns)
    .min_connections(min_conns)
```

**Impact:** Reduces default PostgreSQL memory usage by ~50MB (5 connections × ~10MB each vs 10) and allows operators to tune without rebuilding. Environment variables can be set via `values.yaml` or ConfigMap.

#### 2. Audit Stats GROUP BY Query

Replace the unbounded in-memory aggregation with a SQL-level `GROUP BY`:

```rust
// Current: loads ALL audit logs into memory
let all_logs = audit_log::Entity::find().all(db).await?;
let mut action_counts: HashMap<String, u64> = HashMap::new();
for log in all_logs {
    *action_counts.entry(log.action.clone()).or_insert(0) += 1;
}

// Proposed: SQL GROUP BY - constant memory regardless of log size
use sea_orm::{FromQueryResult, QuerySelect};

#[derive(Debug, FromQueryResult)]
struct ActionCount {
    action: String,
    count: i64,
}

let action_counts = audit_log::Entity::find()
    .select_only()
    .column(audit_log::Column::Action)
    .column_as(audit_log::Column::Id.count(), "count")
    .group_by(audit_log::Column::Action)
    .into_model::<ActionCount>()
    .all(db)
    .await?;
```

**Impact:** Eliminates the most critical memory issue. With 100K audit logs, the current approach allocates ~50MB+ for deserialized models; the GROUP BY approach returns only the aggregated counts (a few KB regardless of table size). This prevents OOM risks as the audit log grows.

#### 3. Background Cache Cleanup

Add a `tokio::spawn` interval task to evict expired entries from both caches:

```rust
// In application startup, after AppState is created:
let cache_clone = app_state.endpoint_cache.clone();
let metrics_clone = app_state.network_metrics_cache.clone();

tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(300)); // Every 5 minutes
    loop {
        interval.tick().await;
        cache_clone.evict_expired().await;
        metrics_clone.evict_stale().await;
        tracing::debug!("Cache cleanup completed");
    }
});
```

This requires adding `evict_expired()` / `evict_stale()` methods to `EndpointCache` and `NetworkMetricsCache` that iterate the HashMap and remove entries older than the TTL/max_age.

**Impact:** Prevents unbounded memory growth from dead cache entries. For `EndpointCache`, this bounds memory to active endpoints only. For `NetworkMetricsCache`, stale namespace entries from deleted namespaces are cleaned up.

#### 4. Scheduled Audit Log Retention

Automate the existing `clear_old_logs()` function via a background task instead of requiring manual admin API calls:

```rust
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(86400)); // Daily
    loop {
        interval.tick().await;
        let retention_days = std::env::var("KUBARR_AUDIT_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(90); // Default: 90 days
        if let Some(db) = db_conn.read().await.as_ref() {
            match clear_old_logs(db, retention_days).await {
                Ok(count) => tracing::info!("Audit cleanup: removed {} old entries", count),
                Err(e) => tracing::warn!("Audit cleanup failed: {}", e),
            }
        }
    }
});
```

**Impact:** Prevents indefinite table growth. With 90-day retention and typical homelab activity (~100-500 events/day), the audit_log table stays under 50K rows (~5MB), keeping the GROUP BY query fast and PostgreSQL VACUUM efficient.

### CNPG Tuning (Infrastructure-Level)

These improvements optimize the CloudNativePG cluster configuration for homelab workloads.

#### 1. Separate WAL Volume

PostgreSQL Write-Ahead Logging (WAL) benefits significantly from dedicated I/O:

```yaml
# CNPG Cluster spec
spec:
  storage:
    size: 5Gi
    storageClass: local-path
  walStorage:
    size: 2Gi
    storageClass: local-path
```

**Impact:** Separating WAL from data storage can improve write IOPS by up to 2x on spinning disks and reduce I/O contention on SSDs. For homelab NVMe/SSD setups, the benefit is moderate but still measurable (~20-30% write throughput improvement) due to reduced fsync contention.

#### 2. Memory and CPU Allocation

Allocate ~75% of available resources to PostgreSQL for optimal buffer pool sizing:

```yaml
spec:
  resources:
    requests:
      memory: 256Mi
      cpu: 100m
    limits:
      memory: 512Mi
      cpu: 500m
  postgresql:
    parameters:
      shared_buffers: "128MB"        # ~25% of memory limit
      effective_cache_size: "384MB"  # ~75% of memory limit
      work_mem: "4MB"               # Per-operation sort memory
      maintenance_work_mem: "64MB"  # For VACUUM, CREATE INDEX
      wal_buffers: "4MB"
      max_connections: "20"         # Match app pool + headroom
```

**Impact:** Proper `shared_buffers` and `effective_cache_size` settings allow PostgreSQL to cache hot data (user sessions, roles, permissions) in memory, reducing disk reads for the auth-layer's 4+ queries per request. For Kubarr's 22-table schema with typical homelab data sizes (<100MB total), most of the working set fits in shared buffers.

#### 3. Dynamic PVC Provisioning

Use StorageClass with `allowVolumeExpansion: true` to enable online volume resizing:

```yaml
spec:
  storage:
    size: 5Gi
    storageClass: local-path  # Or longhorn, openebs-lvm for expansion support
    pvcTemplate:
      accessModes:
        - ReadWriteOnce
```

**Impact:** Eliminates the need to recreate PVCs when storage grows. Operators can resize with `kubectl patch` rather than performing backup/restore cycles. Note: `local-path` provisioner does not support expansion; production homelab setups should consider Longhorn or OpenEBS for this capability.

#### 4. Volume Snapshots for Backups

Leverage Kubernetes VolumeSnapshot API for consistent point-in-time backups:

```yaml
apiVersion: snapshot.storage.k8s.io/v1
kind: VolumeSnapshot
metadata:
  name: kubarr-db-snapshot
spec:
  volumeSnapshotClassName: csi-snapshotter
  source:
    persistentVolumeClaimName: kubarr-db-pvc
```

Alternatively, CNPG's built-in backup to S3-compatible storage (MinIO, Backblaze B2):

```yaml
spec:
  backup:
    barmanObjectStore:
      destinationPath: "s3://kubarr-backups/"
      endpointURL: "https://s3.us-west-000.backblazeb2.com"
      s3Credentials:
        accessKeyId:
          name: backup-creds
          key: ACCESS_KEY_ID
        secretAccessKey:
          name: backup-creds
          key: SECRET_ACCESS_KEY
    retentionPolicy: "30d"
```

**Impact:** Provides automated, consistent backups without manual intervention. CNPG's Barman integration handles WAL archiving and point-in-time recovery (PITR). Backblaze B2 free tier (10GB) is sufficient for typical homelab database sizes.

### High Availability Considerations

CNPG supports primary + replica configurations with automatic failover:

```yaml
spec:
  instances: 2  # Primary + 1 streaming replica
  enableSuperuserAccess: false
  primaryUpdateStrategy: unsupervised
```

**Assessment for homelab:** HA is generally **overkill** for a single-user homelab deployment. The added resource cost (2x PostgreSQL memory/CPU, WAL streaming overhead) outweighs the benefit when:
- Acceptable downtime is minutes, not seconds
- There is one operator who can manually intervene
- Pod restarts typically complete in 10-30 seconds

**When HA makes sense:**
- Multi-user homelab serving a household (3-5 concurrent users)
- Running on a multi-node cluster where node failure is a realistic scenario
- Kubarr manages critical infrastructure where downtime causes cascading issues

### Pros and Cons

| Aspect | Assessment |
|--------|------------|
| **Pros** | |
| Minimal migration effort | No schema changes, no data migration, no deployment model changes |
| Low risk | Each quick win can be deployed independently and rolled back |
| Preserves CNPG features | Automated failover, backup, WAL archiving remain available |
| Full SQL capabilities | All PostgreSQL features (LISTEN/NOTIFY, advanced indexing, JSON operators, full-text search) remain available |
| SeaORM compatibility | No ORM changes needed; existing migrations continue to work |
| Incremental improvement | Quick wins can be shipped immediately while CNPG tuning follows |
| **Cons** | |
| CNPG operator overhead remains | Operator pod (~128MB RAM) continues running for a single database instance |
| Operational complexity persists | Homelab users still need to install and manage the CNPG operator |
| Resource floor unchanged | PostgreSQL pod + operator pod baseline remains ~384MB+ RAM |
| Does not simplify deployment | Still requires CNPG CRDs, operator deployment, and cluster resource creation |
| Limited portability | CNPG is Kubernetes-specific; no path to running Kubarr outside K8s |

### Migration Effort

**Effort: Minimal (1-2 days of development)**

| Change | Effort | Risk |
|--------|--------|------|
| Configurable connection pool | ~1 hour | Very low - additive change, defaults preserved |
| Audit stats GROUP BY | ~2 hours | Low - query change with same output format |
| Background cache cleanup | ~2 hours | Low - additive background task |
| Scheduled audit retention | ~1 hour | Low - wraps existing `clear_old_logs()` |
| CNPG tuning (values.yaml) | ~2 hours | Low - configuration changes, rollback via Helm |
| HA setup (if desired) | ~4 hours | Medium - requires testing failover scenarios |

No data migration is required. No schema changes are needed. All changes are backward-compatible.

### Resource Impact

| Resource | Current | After Quick Wins | After CNPG Tuning |
|----------|---------|------------------|-------------------|
| PostgreSQL memory | ~256MB (default) | ~256MB (unchanged) | ~512MB (tuned buffers) |
| CNPG operator | ~128MB | ~128MB (unchanged) | ~128MB (unchanged) |
| DB connections | 10 max (hardcoded) | 5 max (configurable) | 5 max (configurable) |
| Connection memory | ~100MB (10 × 10MB) | ~50MB (5 × 10MB) | ~50MB (5 × 10MB) |
| Audit log table | Unbounded growth | Bounded by retention policy | Bounded by retention policy |
| Cache memory | Unbounded (lazy eviction) | Bounded (active eviction) | Bounded (active eviction) |
| **Total DB footprint** | **~484MB** | **~434MB** | **~690MB (with tuning)** |

Note: CNPG tuning increases PostgreSQL memory allocation intentionally to improve cache hit rates, which reduces disk I/O and improves query latency. The trade-off is worthwhile if the host has sufficient RAM (4GB+).

### Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| GROUP BY query returns different format | Low | Low | Unit test comparing old vs new output |
| Background task panics | Very low | Low | `tokio::spawn` isolates panics; add error logging |
| CNPG tuning causes instability | Low | Medium | Test in dev cluster first; Helm rollback available |
| Connection pool too small | Low | Low | Configurable via env var; adjust without rebuild |
| Audit retention deletes needed logs | Low | Medium | Default 90 days is conservative; env-configurable |

**Overall risk: Low.** All changes are incremental, independently deployable, and reversible. The quick wins address real issues with proven patterns. CNPG tuning follows well-documented PostgreSQL best practices.

---

## Option B: SQLite + Litestream

**Summary:** Replace PostgreSQL with an embedded SQLite database running inside the backend process, eliminating the CNPG operator and external database pod entirely. Continuous replication to S3-compatible storage (Backblaze B2) is handled by Litestream running as a sidecar container. The Kubernetes deployment model changes from a Deployment to a StatefulSet with `replicas: 1`.

### SeaORM Feature Flag Change

The Rust backend already compiles both PostgreSQL and SQLite support via SeaORM feature flags in `Cargo.toml`:

```toml
# Current Cargo.toml - both backends already compiled
sea-orm = { version = "1.1", features = ["sqlx-postgres", "sqlx-sqlite", "runtime-tokio-native-tls", "macros", "with-chrono"] }
sea-orm-migration = { version = "1.1", features = ["sqlx-postgres", "sqlx-sqlite", "runtime-tokio-native-tls"] }
```

For a SQLite-only deployment, the feature flags would change to remove the PostgreSQL dependency:

```toml
# Option B: SQLite-only (reduces binary size and compile time)
sea-orm = { version = "1.1", features = ["sqlx-sqlite", "runtime-tokio-native-tls", "macros", "with-chrono"] }
sea-orm-migration = { version = "1.1", features = ["sqlx-sqlite", "runtime-tokio-native-tls"] }
```

Alternatively, both can be kept (as today) and the database backend selected at runtime via the `DATABASE_URL` scheme:

```rust
// SQLite: DATABASE_URL=sqlite:///data/kubarr.db?mode=rwc
// PostgreSQL: DATABASE_URL=postgres://user:pass@host:5432/kubarr
```

SeaORM's `Database::connect()` automatically selects the correct driver based on the URL scheme. This means the same binary can support both backends, controlled entirely by configuration.

**Recommendation:** Keep both feature flags compiled (as today) to preserve the ability to use either backend. The binary size increase is negligible (~2MB), and it provides deployment flexibility.

### Data Type Compatibility

All 22 entity models use SeaORM-portable types that work identically across PostgreSQL and SQLite:

| SeaORM Type | PostgreSQL Mapping | SQLite Mapping | Compatible? |
|-------------|-------------------|----------------|-------------|
| `i64` (primary keys) | `BIGINT` | `INTEGER` (64-bit) | ✅ Yes |
| `String` | `VARCHAR`/`TEXT` | `TEXT` | ✅ Yes |
| `DateTimeUtc` (chrono) | `TIMESTAMPTZ` | `TEXT` (ISO 8601) | ✅ Yes |
| `bool` | `BOOLEAN` | `INTEGER` (0/1) | ✅ Yes |
| `Option<T>` | `NULLABLE` | `NULLABLE` | ✅ Yes |
| `Json` (serde_json) | `JSONB` | `TEXT` (JSON string) | ✅ Yes¹ |

¹ SQLite stores JSON as text. SeaORM handles serialization/deserialization transparently. However, PostgreSQL JSON operators (`->`, `->>`, `@>`, `#>`) are not available in SQLite. The current codebase uses `serde_json::Value` for JSON columns and deserializes in application code, so this is not an issue.

**Verified:** All 22 entity models in `code/backend/src/models/` use `i64` primary keys and `DateTimeUtc` timestamps. No PostgreSQL-specific column types (arrays, enums, custom types) are used. The SeaORM abstraction layer handles type mapping transparently.

### Connection Configuration Change

The `database.rs` connection setup changes minimally for SQLite:

```rust
// Current PostgreSQL connection
let mut opts = ConnectOptions::new("postgres://user:pass@host:5432/kubarr");
opts.max_connections(10)
    .min_connections(1)
    .connect_timeout(Duration::from_secs(30))
    .idle_timeout(Duration::from_secs(600))
    .sqlx_logging(false);

// SQLite connection (Option B)
let mut opts = ConnectOptions::new("sqlite:///data/kubarr.db?mode=rwc");
opts.max_connections(1)       // SQLite: single-writer, one connection for writes
    .min_connections(1)       // Keep connection alive
    .connect_timeout(Duration::from_secs(10))
    .idle_timeout(Duration::from_secs(600))
    .sqlx_logging(false);

// Critical: enable WAL mode for concurrent reads during writes
db.execute_unprepared("PRAGMA journal_mode=WAL").await?;
db.execute_unprepared("PRAGMA busy_timeout=5000").await?;
db.execute_unprepared("PRAGMA synchronous=NORMAL").await?;
db.execute_unprepared("PRAGMA foreign_keys=ON").await?;
```

**Key SQLite PRAGMAs:**
- `journal_mode=WAL` — Enables Write-Ahead Logging for concurrent read access during writes. Essential for Litestream replication.
- `busy_timeout=5000` — Wait up to 5 seconds for write lock instead of immediately returning SQLITE_BUSY.
- `synchronous=NORMAL` — Balanced durability/performance. WAL mode makes this safe; Litestream provides the durability guarantee via S3 replication.
- `foreign_keys=ON` — SQLite disables foreign key enforcement by default. Must be enabled per connection.

### StatefulSet Deployment

SQLite requires a persistent volume mounted into the pod. The Kubernetes deployment model changes from `Deployment` to `StatefulSet` to ensure stable storage identity:

```yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: {{ include "kubarr.fullname" . }}-backend
  namespace: {{ .Values.namespace.name }}
  labels:
    {{- include "kubarr.labels" . | nindent 4 }}
    app.kubernetes.io/component: backend
spec:
  serviceName: {{ include "kubarr.fullname" . }}-backend
  replicas: 1  # CRITICAL: SQLite requires single-writer; never scale beyond 1
  updateStrategy:
    type: Recreate  # Must stop old pod before starting new one (no rolling updates)
  selector:
    matchLabels:
      app.kubernetes.io/name: {{ include "kubarr.name" . }}-backend
      app.kubernetes.io/instance: {{ .Release.Name }}
  template:
    metadata:
      labels:
        app.kubernetes.io/name: {{ include "kubarr.name" . }}-backend
        app.kubernetes.io/instance: {{ .Release.Name }}
    spec:
      serviceAccountName: {{ include "kubarr.serviceAccountName" . }}
      # Init container: restore SQLite database from S3 via Litestream before backend starts
      initContainers:
        - name: litestream-restore
          image: litestream/litestream:0.3
          args: ["restore", "-if-db-not-exists", "/data/kubarr.db"]
          volumeMounts:
            - name: data
              mountPath: /data
          envFrom:
            - secretRef:
                name: {{ include "kubarr.fullname" . }}-litestream
      containers:
        # Backend API container
        - name: backend
          image: "{{ .Values.backend.image.repository }}:{{ .Values.backend.image.tag }}"
          ports:
            - name: http
              containerPort: {{ .Values.backend.service.targetPort }}
              protocol: TCP
          env:
            - name: DATABASE_URL
              value: "sqlite:///data/kubarr.db?mode=rwc"
            {{- toYaml .Values.backend.env | nindent 12 }}
          volumeMounts:
            - name: data
              mountPath: /data
          livenessProbe:
            {{- toYaml .Values.backend.livenessProbe | nindent 12 }}
          readinessProbe:
            {{- toYaml .Values.backend.readinessProbe | nindent 12 }}
          resources:
            {{- toYaml .Values.backend.resources | nindent 12 }}
        # Litestream sidecar: continuous replication to S3
        - name: litestream
          image: litestream/litestream:0.3
          args: ["replicate"]
          volumeMounts:
            - name: data
              mountPath: /data
            - name: litestream-config
              mountPath: /etc/litestream.yml
              subPath: litestream.yml
          envFrom:
            - secretRef:
                name: {{ include "kubarr.fullname" . }}-litestream
          resources:
            requests:
              memory: 32Mi
              cpu: 10m
            limits:
              memory: 64Mi
              cpu: 50m
      volumes:
        - name: litestream-config
          configMap:
            name: {{ include "kubarr.fullname" . }}-litestream
  volumeClaimTemplates:
    - metadata:
        name: data
      spec:
        accessModes: ["ReadWriteOnce"]
        storageClassName: {{ .Values.storage.storageClass | default "local-path" }}
        resources:
          requests:
            storage: {{ .Values.storage.size | default "1Gi" }}
```

**Key design decisions:**
- `replicas: 1` — SQLite's single-writer constraint means only one pod can write at a time. Attempting to scale beyond 1 would cause `SQLITE_BUSY` errors.
- `updateStrategy: Recreate` — The old pod must fully stop (releasing the SQLite file lock) before the new pod starts. Rolling updates would cause two pods to contend for the same database file.
- `volumeClaimTemplates` — StatefulSet provides stable PVC identity (`data-kubarr-backend-0`) that survives pod restarts and rescheduling.
- Init container runs `litestream restore` with `-if-db-not-exists` — Only restores from S3 on first deploy or after data loss. Subsequent restarts use the existing PVC data.

### Litestream Sidecar Pattern

Litestream continuously replicates SQLite's WAL (Write-Ahead Log) to S3-compatible object storage. This provides near-real-time backup without any application code changes.

**How it works:**

1. **Init container (restore):** Before the backend starts, Litestream downloads the latest database snapshot and WAL segments from S3, reconstructing the full SQLite database. The `-if-db-not-exists` flag skips restore if the database already exists on the PVC.

2. **Sidecar container (replicate):** Runs alongside the backend, watching the SQLite WAL file for changes. New WAL frames are uploaded to S3 within seconds of being written. Litestream performs periodic snapshots (default: every 24 hours) for faster future restores.

**Litestream configuration:**

```yaml
# ConfigMap: litestream.yml
dbs:
  - path: /data/kubarr.db
    replicas:
      - type: s3
        bucket: kubarr-backups
        path: db
        endpoint: https://s3.us-west-000.backblazeb2.com
        region: us-west-000
        retention: 168h          # Keep 7 days of WAL segments
        retention-check-interval: 1h
        snapshot-interval: 24h   # Full snapshot daily
        sync-interval: 1s       # Replicate WAL changes every second
        validation-interval: 12h # Verify replica integrity every 12 hours
```

**Recovery scenarios:**

| Scenario | Recovery Method | Downtime | Data Loss |
|----------|----------------|----------|-----------|
| Pod restart (same node) | PVC data intact; no restore needed | ~5-10 seconds (pod startup) | None |
| Pod reschedule (different node) | PVC may need re-provisioning; Litestream restores from S3 | ~30-60 seconds (restore + startup) | ≤1 second (last unreplicated WAL frame) |
| PVC data corruption | Litestream restores latest snapshot + WAL from S3 | ~30-60 seconds | ≤1 second |
| S3 bucket loss | PVC still has local data; manual backup needed | None (until PVC loss) | None (if PVC intact) |
| Complete disaster (PVC + S3) | Full data loss | N/A | All data |

### S3-Compatible Backup (Backblaze B2)

Backblaze B2 is the recommended S3-compatible storage for homelab use:

- **Free tier:** 10 GB storage, 1 GB/day download — more than sufficient for SQLite databases (typical Kubarr DB: <50MB)
- **S3-compatible API:** Litestream natively supports B2's S3 endpoint
- **Pricing beyond free tier:** $0.006/GB/month storage, $0.01/GB egress — negligible for database backups
- **Regions:** US West, US East, EU Central — low latency for most homelab locations

**Setup:**

```yaml
# Kubernetes Secret for Litestream S3 credentials
apiVersion: v1
kind: Secret
metadata:
  name: kubarr-litestream
type: Opaque
stringData:
  LITESTREAM_ACCESS_KEY_ID: "your-b2-application-key-id"
  LITESTREAM_SECRET_ACCESS_KEY: "your-b2-application-key"
```

**Alternative S3 backends:** MinIO (self-hosted), Wasabi ($6.99/TB/month), AWS S3, or any S3-compatible storage. Litestream is backend-agnostic.

**Offline/air-gapped fallback:** For environments without internet access, Litestream can replicate to a local MinIO instance or an NFS-mounted directory using the `file` replica type:

```yaml
replicas:
  - type: file
    path: /backup/kubarr-db
    retention: 168h
```

### Critical Constraints

#### 1. Single-Pod Only (Single-Writer Limitation)

SQLite supports only one concurrent writer. All write operations are serialized through a single write lock. This means:

- **No horizontal scaling** — Only one backend pod can exist. Attempting `replicas: 2` would cause `SQLITE_BUSY` errors on writes.
- **No read replicas** — Unlike PostgreSQL, there is no streaming replication for read scaling.
- **Write throughput** — SQLite in WAL mode handles ~50,000-100,000 simple writes/second on SSD. For Kubarr's homelab workload (1-5 users, <100 writes/minute), this is orders of magnitude more than sufficient.
- **Concurrent reads** — WAL mode allows unlimited concurrent readers even during writes. Read throughput is not a concern.

**Assessment for Kubarr:** The single-writer limitation is a non-issue for the homelab use case. Kubarr is already designed for single-pod deployment (`replicas: 1` in the current Deployment). The write workload (session updates, audit logs, configuration changes) is extremely light.

#### 2. Brief Downtime on Pod Reschedule

The `Recreate` update strategy means the old pod must fully terminate before the new pod starts. During this window:

- **Duration:** Typically 10-30 seconds (pod termination + new pod scheduling + Litestream restore if PVC is lost + app startup)
- **Impact:** API requests return 503 during the transition. WebSocket connections are dropped and must reconnect.
- **Mitigation:** Kubernetes `terminationGracePeriodSeconds` allows the backend to complete in-flight requests. Litestream's init container restore is fast for small databases (<1 second for <50MB).

**Assessment:** This downtime window is identical to the current behavior with PostgreSQL when the backend pod restarts. The CNPG database pod also has its own restart/failover time. In practice, SQLite may have *less* total downtime because there is no external database pod to restart or failover.

#### 3. No LISTEN/NOTIFY

PostgreSQL's `LISTEN/NOTIFY` mechanism provides real-time database-level event notifications. SQLite has no equivalent feature. This affects:

- **Current usage:** The Kubarr codebase does not currently use `LISTEN/NOTIFY`. WebSocket notifications are handled via Tokio broadcast channels in `AppState` (`NetworkMetricsBroadcast`, `BootstrapBroadcast`), not database triggers.
- **Future impact:** If real-time database change notifications were ever needed, they would need to be implemented via application-level event emission (e.g., publish to a Tokio channel after each write) rather than database triggers.

**Assessment:** No impact on current functionality. The application already uses in-memory broadcast channels for real-time events.

#### 4. No Advanced PostgreSQL Features

Migrating to SQLite means losing access to:

- **Full-text search (tsvector)** — Not currently used. If needed, SQLite's FTS5 extension provides similar functionality.
- **Array columns** — Not currently used in any of the 22 entity models.
- **JSONB operators** — Not used in queries; JSON is deserialized in application code via `serde_json`.
- **Advanced indexing (GIN, GiST, BRIN)** — Not currently used. SQLite supports standard B-tree indexes, which are sufficient for the current query patterns.
- **Window functions** — SQLite supports window functions (since 3.25.0), so this is not a limitation.
- **CTEs** — SQLite supports CTEs (since 3.8.3), so this is not a limitation.

**Assessment:** No current Kubarr functionality depends on PostgreSQL-specific features. All queries use standard SQL that is portable across both backends.

### Data Migration Approach

Migrating existing data from PostgreSQL to SQLite requires a one-time export/import process:

#### Step 1: Export from PostgreSQL

```bash
# Export each table as SQL INSERT statements (portable format)
pg_dump --data-only --inserts --no-owner --no-privileges \
  --exclude-table=sea_orm_migration \
  -d kubarr > kubarr_data.sql
```

Using `--inserts` instead of `COPY` ensures the SQL is SQLite-compatible. The `sea_orm_migration` table is excluded because SeaORM recreates it automatically.

#### Step 2: Create SQLite Database

```bash
# Run the backend with SQLite URL to trigger SeaORM migrations
DATABASE_URL=sqlite:///tmp/kubarr.db?mode=rwc cargo run
# Or use the migration CLI:
sea-orm-cli migrate up -d /tmp/kubarr.db
```

SeaORM's migration framework handles the SQLite schema creation. The same migration files work for both PostgreSQL and SQLite because they use SeaORM's database-agnostic DDL API.

#### Step 3: Import Data

```bash
# Clean up PostgreSQL-specific syntax from the dump
sed -e "s/true/1/g" -e "s/false/0/g" \
    -e "s/::.*//g" \
    -e "/^SET /d" -e "/^SELECT pg_/d" \
    kubarr_data.sql > kubarr_data_sqlite.sql

# Import into SQLite
sqlite3 /tmp/kubarr.db < kubarr_data_sqlite.sql
```

**Data type conversions during migration:**
- `boolean` → `0`/`1` (SQLite stores booleans as integers)
- `timestamptz` → ISO 8601 text (SeaORM handles this transparently via chrono)
- `bigint` → `INTEGER` (SQLite's INTEGER is 64-bit, matching PostgreSQL's BIGINT)
- PostgreSQL type casts (`::text`, `::integer`) → removed (SQLite doesn't use cast syntax in this form)

#### Step 4: Verify

```bash
# Count rows in each table to verify migration completeness
sqlite3 /tmp/kubarr.db "SELECT name, (SELECT COUNT(*) FROM [name]) FROM sqlite_master WHERE type='table';"
```

**Alternative approach:** A Rust migration tool could be written to read from PostgreSQL and write to SQLite using SeaORM, providing type-safe migration with automatic conversion. This is more robust but requires development effort.

#### Migration Effort Estimate

| Task | Effort | Risk |
|------|--------|------|
| SQLite PRAGMAs in `database.rs` | ~2 hours | Low - well-documented SQLite best practices |
| Migration compatibility testing | ~4 hours | Medium - verify all 22 migrations run on SQLite |
| StatefulSet Helm template | ~4 hours | Low - template conversion with Litestream sidecar |
| Litestream ConfigMap and Secret | ~2 hours | Low - standard Kubernetes resources |
| Data export/import tooling | ~4 hours | Medium - type conversion edge cases |
| Integration testing | ~8 hours | Medium - verify all 22 entities CRUD on SQLite |
| Remove CNPG dependency | ~2 hours | Low - delete CRDs, operator, and RBAC rules |
| Documentation | ~2 hours | Low - update README and deployment guide |
| **Total** | **~28 hours (3-4 days)** | **Medium overall** |

### Pros and Cons

| Aspect | Assessment |
|--------|------------|
| **Pros** | |
| Eliminates CNPG operator | No operator pod (~128MB RAM), no CRDs, no operator upgrades to manage |
| Eliminates PostgreSQL pod | No separate database pod (~256MB+ RAM), no connection pool overhead |
| Dramatically simpler deployment | One StatefulSet with a sidecar replaces Deployment + CNPG Cluster + operator |
| Lower resource footprint | SQLite is embedded in the backend process; total savings ~384MB+ RAM |
| Automated backup via Litestream | Continuous S3 replication with <1s RPO; simpler than CNPG Barman config |
| No network database latency | Queries execute in-process via file I/O, not over TCP. ~10x faster for simple queries |
| SQLite already compiled | Both `sqlx-postgres` and `sqlx-sqlite` feature flags are in `Cargo.toml`; no new dependencies |
| Ideal for homelab scale | SQLite handles millions of rows; Kubarr's 22 tables with <100K rows total is trivial |
| Fewer failure modes | No database connection failures, no network partitions, no connection pool exhaustion |
| **Cons** | |
| Single-writer limitation | Cannot scale beyond 1 replica; writes are serialized (but sufficient for homelab) |
| Recreate update strategy | Brief downtime (~10-30s) during pod updates; no zero-downtime rolling updates |
| No LISTEN/NOTIFY | Database-level event notifications unavailable (not currently used) |
| Migration effort required | ~3-4 days of development to convert, test, and validate data migration |
| S3 dependency for backup | Litestream requires S3-compatible storage; adds external dependency (but free via B2) |
| Less ecosystem tooling | No pgAdmin, no `psql`, limited debugging tools vs PostgreSQL's rich ecosystem |
| StatefulSet complexity | PVC management, volume provisioning, and storage class requirements |
| No advanced SQL features | No PostgreSQL-specific features (though none are currently used) |

### Resource Impact

| Resource | Current (PostgreSQL + CNPG) | Option B (SQLite + Litestream) | Savings |
|----------|---------------------------|-------------------------------|---------|
| CNPG operator pod | ~128MB RAM, ~50m CPU | Eliminated | 128MB RAM |
| Database pod | ~256MB RAM, ~100m CPU | Eliminated | 256MB RAM |
| Litestream sidecar | N/A | ~32-64MB RAM, ~10m CPU | -64MB RAM |
| Backend memory (DB) | ~10MB (connection pool) | ~5MB (SQLite in-process) | 5MB RAM |
| Database connections | 10 TCP connections | 1 file handle | Negligible |
| Storage volumes | CNPG PVC (5Gi) + WAL PVC (2Gi) | Single PVC (1Gi) | 6Gi disk |
| S3 backup storage | Optional (CNPG Barman) | Required (Litestream) | ~Same cost |
| **Total RAM savings** | | | **~325MB** |
| **Total pod count** | 3 (backend + PG + operator) | 1 (backend + sidecar) | 2 fewer pods |

### Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Migration data loss | Low | High | Test migration on copy; verify row counts; keep PostgreSQL running during transition |
| SQLite corruption | Very low | High | Litestream provides continuous backup; WAL mode is mature and well-tested |
| SeaORM migration incompatibility | Medium | Medium | Run all 22 migrations on SQLite in CI; fix any PostgreSQL-specific DDL |
| Litestream S3 outage | Low | Low | Local PVC has full data; S3 is only for backup, not runtime |
| Write contention under load | Very low | Low | SQLite handles >50K writes/sec; Kubarr does <100 writes/min |
| Future need for PostgreSQL features | Low | Medium | Both feature flags remain compiled; can switch back via `DATABASE_URL` |
| PVC data loss on node failure | Low | Medium | Litestream restores from S3 within seconds; RPO <1 second |
| StatefulSet operational complexity | Low | Low | Standard K8s pattern; well-documented; simpler than CNPG operator |

**Overall risk: Medium.** The migration itself carries moderate risk (data conversion, migration compatibility), but the runtime risk is lower than the current architecture (fewer moving parts, fewer failure modes). The ability to keep both feature flags and switch via `DATABASE_URL` provides a safe rollback path.

---

## Option C: Hybrid Approach (SQLite + Optional Caching Layer)

**Summary:** Use SQLite as the primary relational database (identical to Option B), but add an optional caching layer to address specific performance concerns — particularly auth-layer query overhead, session management, and potential future multi-replica scenarios. The caching layer can be either an external service (Redis/Valkey) or an embedded key-value store (redb, fjall), chosen based on deployment complexity tolerance and scaling ambitions.

This option recognizes that while SQLite eliminates the CNPG/PostgreSQL overhead, certain workloads (session lookups, permission caching, real-time notifications) benefit from a dedicated caching layer that survives beyond individual request lifetimes and offers native TTL expiration.

### Primary Storage: SQLite (Same as Option B)

The SQLite configuration, Litestream replication, StatefulSet deployment, data migration approach, and all critical constraints documented in Option B apply identically here. Option C extends Option B by layering a caching strategy on top.

**Refer to Option B for:**
- SeaORM feature flag configuration
- Data type compatibility (all 22 entities portable)
- SQLite PRAGMA settings (WAL mode, busy_timeout, synchronous, foreign_keys)
- StatefulSet with Litestream sidecar deployment
- S3-compatible backup via Backblaze B2
- Single-writer limitation and constraints
- Data migration approach from PostgreSQL

### When an External Caching Layer Is Justified

Not every deployment benefits from adding a caching layer. The decision depends on specific workload characteristics and operational requirements:

#### Justified Scenarios

| Scenario | Why Caching Helps |
|----------|-------------------|
| **Multi-replica sessions** | If Kubarr ever needs to scale beyond a single pod (e.g., separate API and worker pods), session state must be shared across processes. An in-memory cache per pod cannot serve this need. |
| **Cross-pod pub/sub** | WebSocket notifications currently use Tokio broadcast channels, which are process-local. If multiple pods need to broadcast events (e.g., "new download started"), a shared pub/sub mechanism (Redis/Valkey PUBLISH/SUBSCRIBE) enables cross-pod communication without polling. |
| **Persistent cache across restarts** | Current in-memory caches (`EndpointCache`, `NetworkMetricsCache`) are lost on pod restart. An external or embedded persistent cache preserves warm data across restarts, eliminating cold-start latency for auth lookups, endpoint resolution, and rate history. |
| **Auth-layer query reduction** | The current architecture performs 4+ database queries per authenticated request. A cache with TTL-based expiration can serve session/user/permission lookups from memory, reducing database load from O(requests) to O(cache_misses). |
| **Rate limiting** | If rate limiting is ever needed, atomic increment operations with TTL (native to Redis/Valkey) are the standard implementation pattern. SQLite cannot efficiently serve this use case. |

#### Not Justified Scenarios

| Scenario | Why Caching Is Overkill |
|----------|------------------------|
| **Single-pod homelab with 1-5 users** | SQLite queries for auth lookups complete in <1ms from disk (often from OS page cache). The 4+ queries per request are fast enough that the latency is imperceptible to users. Adding a cache adds complexity without measurable user-facing improvement. |
| **Low write volume** | Kubarr's write workload (<100 writes/minute) means cache invalidation is trivial — but it also means there's little to gain from caching because the database is never under pressure. |
| **Simple deployment priority** | Homelab operators who chose SQLite to eliminate CNPG complexity may not want to add Redis/Valkey complexity back. The operational burden of another service (even a small one) contradicts the simplicity motivation. |

### External Caching: Redis / Valkey

#### Overview

Redis is the de facto standard for application-level caching, session storage, and pub/sub messaging. Valkey is the open-source fork of Redis (created after Redis Ltd. changed its license to RSALv2 + SSPLv1 in March 2024) and is API-compatible.

**Recommendation:** Use Valkey over Redis for new deployments. Valkey is maintained by the Linux Foundation, is fully open-source (BSD-3-Clause license), and has broad industry backing (AWS, Google, Oracle, Ericsson). It is a drop-in replacement for Redis with identical API and protocol.

#### Rust Client: `fred` Crate (Preferred)

The `fred` crate is the recommended Redis/Valkey client for Rust async applications:

```toml
# Cargo.toml addition for Option C with external cache
fred = { version = "9", features = ["subscriber-client", "tokio-runtime"] }
```

**Why `fred` over alternatives:**

| Crate | Assessment |
|-------|-----------|
| `fred` (recommended) | Full async/await, connection pooling, cluster support, pub/sub, Lua scripting, pipeline batching, reconnect with backoff. Actively maintained, production-tested. |
| `redis-rs` | Older, widely used but lower-level. Less ergonomic async API. Adequate but `fred` is more modern. |
| `deadpool-redis` | Connection pool wrapper around `redis-rs`. Adds pooling but not pub/sub or advanced features. |

**Example: Auth-layer caching with `fred`:**

```rust
use fred::prelude::*;

/// Cache key format for session lookups
fn session_cache_key(session_id: &str) -> String {
    format!("session:{}", session_id)
}

/// Cached auth context: session + user + permissions
#[derive(Serialize, Deserialize)]
struct CachedAuthContext {
    session: SessionModel,
    user: UserModel,
    permissions: Vec<String>,
    app_permissions: Vec<AppPermission>,
}

/// Look up auth context: cache first, then database
async fn get_auth_context(
    client: &RedisClient,
    db: &DatabaseConnection,
    session_id: &str,
) -> Result<CachedAuthContext> {
    let cache_key = session_cache_key(session_id);

    // Try cache first
    if let Some(cached): Option<String> = client.get(&cache_key).await? {
        if let Ok(ctx) = serde_json::from_str::<CachedAuthContext>(&cached) {
            return Ok(ctx);
        }
    }

    // Cache miss: query database (4+ queries)
    let session = Session::find_by_id(session_id).one(db).await?;
    let user = User::find_by_id(session.user_id).one(db).await?;
    let permissions = fetch_permissions(db, user.id).await?;
    let app_permissions = fetch_app_permissions(db, user.id).await?;

    let ctx = CachedAuthContext { session, user, permissions, app_permissions };

    // Cache with TTL (e.g., 5 minutes)
    let json = serde_json::to_string(&ctx)?;
    client.set(&cache_key, json.as_str(), Some(Expiration::EX(300)), None, false).await?;

    Ok(ctx)
}
```

#### Native Redis/Valkey Features Relevant to Kubarr

| Feature | Use Case in Kubarr | Benefit |
|---------|-------------------|---------|
| **TTL expiration** | Session cache auto-expires after 5 minutes; no manual eviction needed | Eliminates the "no automatic cache eviction" issue identified in the current architecture |
| **Pub/Sub** | `PUBLISH kubarr:notifications <event>` enables cross-pod WebSocket event broadcasting | Future-proofs multi-pod notification delivery without polling |
| **Atomic operations** | `INCR`, `EXPIRE` for rate limiting counters | Enables rate limiting without database writes |
| **Key patterns** | `session:*`, `endpoint:*`, `auth:*` with `SCAN` for bulk operations | Clean cache invalidation on permission changes (e.g., `DEL session:*` after role update) |
| **Memory management** | `maxmemory-policy allkeys-lru` for automatic eviction under memory pressure | Bounded memory usage without application-level eviction logic |

#### Kubernetes Deployment (Valkey)

```yaml
# Minimal Valkey deployment for homelab
apiVersion: apps/v1
kind: Deployment
metadata:
  name: kubarr-valkey
  namespace: kubarr
spec:
  replicas: 1
  selector:
    matchLabels:
      app: kubarr-valkey
  template:
    spec:
      containers:
        - name: valkey
          image: valkey/valkey:8-alpine
          ports:
            - containerPort: 6379
          command: ["valkey-server"]
          args:
            - "--maxmemory"
            - "64mb"
            - "--maxmemory-policy"
            - "allkeys-lru"
            - "--save"
            - ""           # Disable RDB persistence (cache-only; SQLite is source of truth)
            - "--appendonly"
            - "no"         # Disable AOF persistence
          resources:
            requests:
              memory: 32Mi
              cpu: 10m
            limits:
              memory: 96Mi
              cpu: 100m
---
apiVersion: v1
kind: Service
metadata:
  name: kubarr-valkey
  namespace: kubarr
spec:
  selector:
    app: kubarr-valkey
  ports:
    - port: 6379
      targetPort: 6379
```

**Key decisions:**
- **No persistence** — Valkey is a cache, not a database. SQLite is the source of truth. Disabling RDB and AOF saves disk I/O and simplifies operation.
- **64MB memory limit** — Sufficient for caching sessions, endpoints, and permissions for 1-5 users. LRU eviction handles memory pressure automatically.
- **Alpine image** — Minimal footprint (~30MB container image vs ~130MB full image).

### Embedded Key-Value Alternatives

For deployments where adding an external service (Valkey) is undesirable, an embedded Rust key-value store can provide persistent caching within the backend process itself.

#### redb (Recommended)

**Repository:** [cberner/redb](https://github.com/cberner/redb)
**License:** MIT / Apache-2.0

```toml
redb = "2"
```

**Characteristics:**
- **ACID transactions** — Full transactional guarantees with crash safety
- **Stable API** — Reached 1.0 in 2023; now at 2.x with a mature, well-documented API
- **B+ tree storage engine** — Optimized for read-heavy workloads (ideal for cache lookups)
- **Zero dependencies** — Pure Rust, no C bindings, no system library requirements
- **Memory-mapped I/O** — Leverages OS page cache for fast reads without explicit caching logic
- **Concurrent readers** — Multiple threads can read simultaneously (similar to SQLite WAL mode)
- **Single-writer** — One write transaction at a time (same constraint as SQLite)

**Example: Auth context caching with redb:**

```rust
use redb::{Database, ReadableTable, TableDefinition};
use std::time::{SystemTime, UNIX_EPOCH};

const AUTH_CACHE: TableDefinition<&str, &[u8]> = TableDefinition::new("auth_cache");
const AUTH_EXPIRY: TableDefinition<&str, u64> = TableDefinition::new("auth_expiry");

fn get_cached_auth(db: &Database, session_id: &str) -> Option<CachedAuthContext> {
    let read_txn = db.begin_read().ok()?;
    let expiry_table = read_txn.open_table(AUTH_EXPIRY).ok()?;
    let cache_table = read_txn.open_table(AUTH_CACHE).ok()?;

    // Check expiry
    let expiry = expiry_table.get(session_id).ok()??;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if now > expiry.value() {
        return None; // Expired
    }

    // Read cached value
    let value = cache_table.get(session_id).ok()??;
    serde_json::from_slice(value.value()).ok()
}
```

**Assessment for Kubarr:** redb is the best embedded KV option. Its ACID guarantees, stable API, and read-optimized B+ tree engine align perfectly with Kubarr's cache use case (frequent reads, infrequent writes, crash safety desired). The pure-Rust implementation eliminates cross-compilation concerns for ARM homelab devices.

#### fjall

**Repository:** [fjall-rs/fjall](https://github.com/fjall-rs/fjall)
**License:** MIT / Apache-2.0

```toml
fjall = "2"
```

**Characteristics:**
- **LSM-tree storage engine** — Optimized for write-heavy workloads with background compaction
- **Key-value partitions** — Supports multiple "partitions" (column families) within one database
- **Configurable compaction** — Leveled, tiered, or FIFO compaction strategies
- **Bloom filters** — Reduces unnecessary disk reads for missing keys
- **Cross-partition transactions** — Atomic writes across multiple partitions
- **Active development** — Rapidly evolving API; version 2.x

**Assessment for Kubarr:** fjall is better suited for write-heavy workloads (logging, time-series) than for read-heavy cache lookups. Its LSM-tree engine has higher read amplification than redb's B+ tree for point lookups. The API is less stable than redb. **Use fjall only if write throughput is a bottleneck** — which it is not for Kubarr's homelab workload.

#### sled — NOT RECOMMENDED

**Repository:** [spacejam/sled](https://github.com/spacejam/sled)

**⚠️ Do not use sled.** Despite its popularity (4K+ GitHub stars), sled has critical issues:

- **Perpetual beta** — The README explicitly states: "sled is still beta quality." It has been in beta since 2018 with no 1.0 release timeline.
- **Data loss bugs** — Multiple open issues report data corruption and loss under concurrent access.
- **Uncertain maintenance** — Development has slowed significantly; the maintainer has discussed rewriting the engine entirely.
- **API instability** — Breaking changes between minor versions with no stability guarantees.

**redb was explicitly created as a stable, ACID-compliant alternative to sled.** Use redb instead.

#### Embedded KV Comparison

| Feature | redb | fjall | sled |
|---------|------|-------|------|
| Storage engine | B+ tree | LSM-tree | Lock-free B+ tree |
| ACID transactions | ✅ Full | ✅ Full | ⚠️ Partial |
| API stability | ✅ Stable (2.x) | ⚠️ Evolving (2.x) | ❌ Beta |
| Read performance | ✅ Excellent | ⚠️ Good (read amplification) | ✅ Good |
| Write performance | ✅ Good | ✅ Excellent | ✅ Good |
| Crash safety | ✅ Proven | ✅ Yes | ⚠️ Reported issues |
| Dependencies | Zero | Minimal | Minimal |
| Recommended? | **✅ Yes** | Conditional (write-heavy only) | **❌ No** |

### Is a Caching Layer Overkill for Single-Pod Homelab?

**Short answer: Yes, for most homelab deployments.**

The quantitative case against adding a caching layer for Kubarr's target deployment:

| Metric | Without Cache | With Cache | Improvement |
|--------|--------------|------------|-------------|
| Auth lookup latency | ~1-5ms (SQLite, likely from OS page cache) | ~0.1-0.5ms (in-memory) | 1-5ms saved per request |
| Auth queries per request | 4+ SQLite queries | 1 cache lookup (on hit) | 3 fewer queries |
| Total request latency | ~10-50ms (typical API) | ~5-45ms | Imperceptible to user |
| Cold start penalty | ~5-10 seconds (cache rebuilds) | None (persistent cache) | Moderate improvement |
| Memory overhead | None | +32-96MB (Valkey) or +5-20MB (redb) | Negative (more memory used) |

**For 1-5 users making <100 requests/minute**, the latency improvement is imperceptible. SQLite with WAL mode and OS page caching already serves read queries in <1ms for Kubarr's data sizes (<100MB). The 4+ auth queries complete in ~2-5ms total — well within acceptable latency for a homelab UI.

**When the calculus changes:**
- **>10 concurrent users** — Auth query volume becomes meaningful; cache reduces database load
- **Multi-pod deployment** — Shared session state requires external cache (in-memory is per-pod)
- **Sub-millisecond latency requirements** — If Kubarr becomes an API gateway or proxy with strict latency SLOs
- **Rate limiting** — Atomic increment with TTL is a cache-native operation

**Recommendation:** Start without a caching layer (pure Option B). Add redb as an embedded auth cache when/if auth latency becomes measurable. Add Valkey only if multi-pod deployment becomes a requirement.

### Pros and Cons

| Aspect | Assessment |
|--------|------------|
| **Pros** | |
| All Option B benefits | SQLite simplicity, eliminated CNPG, Litestream backup, lower resource footprint |
| Auth performance improvement | 4+ DB queries reduced to 1 cache lookup (cache hit) for session/user/permissions |
| Native TTL expiration | Redis/Valkey or redb-with-expiry replaces manual eviction logic in current `EndpointCache`/`NetworkMetricsCache` |
| Future-proof for multi-pod | External cache (Valkey) enables shared sessions and pub/sub if horizontal scaling is ever needed |
| Persistent cache survives restarts | Eliminates cold-start penalty for endpoint lookups and network rate history (with redb or Valkey with persistence) |
| Bounded memory by design | Valkey's `maxmemory-policy allkeys-lru` or redb's disk-backed storage prevents unbounded growth |
| **Cons** | |
| Added operational complexity | One more component to deploy, monitor, and debug (Valkey) or one more database file to manage (redb) |
| Marginal benefit for homelab | 1-5ms latency improvement is imperceptible for 1-5 users; effort-to-value ratio is poor |
| Cache invalidation complexity | Must invalidate cached auth context when permissions change, sessions are revoked, or users are deactivated |
| Two storage engines to understand | Developers must understand both SQLite and the cache layer; increases onboarding complexity |
| Valkey adds ~96MB RAM | Partially offsets the ~325MB saved by removing PostgreSQL/CNPG |
| redb adds write contention | Both SQLite and redb are single-writer; two single-writer stores on the same pod increase lock contention risk |

### Migration Effort

**Effort: Option B base + caching layer addition**

| Task | Effort | Risk |
|------|--------|------|
| All Option B tasks (SQLite migration) | ~28 hours (3-4 days) | Medium (see Option B) |
| **Additional for Valkey path:** | | |
| Add `fred` crate and connection setup | ~2 hours | Low - well-documented crate |
| Implement auth-layer cache (get/set/invalidate) | ~6 hours | Medium - cache invalidation logic |
| Add Valkey Kubernetes deployment | ~2 hours | Low - standard deployment |
| Cache invalidation on permission changes | ~4 hours | Medium - must cover all mutation paths |
| Integration testing (cache hit/miss/invalidation) | ~4 hours | Medium - edge cases in invalidation |
| **Valkey subtotal** | **~18 hours (2-3 days)** | **Medium** |
| **Additional for redb path:** | | |
| Add `redb` crate and database setup | ~2 hours | Low - simple API |
| Implement auth-layer cache with expiry | ~4 hours | Low - straightforward key-value ops |
| Background expiry cleanup task | ~2 hours | Low - periodic scan and delete |
| Integration testing | ~3 hours | Low - embedded, easier to test |
| **redb subtotal** | **~11 hours (1-2 days)** | **Low** |

**Total effort:**
- Option C with Valkey: ~46 hours (5-6 days)
- Option C with redb: ~39 hours (4-5 days)

### Resource Impact

| Resource | Option B (SQLite only) | Option C + Valkey | Option C + redb |
|----------|----------------------|-------------------|-----------------|
| Backend pod memory | ~50MB | ~50MB | ~55-70MB (+redb mmap) |
| Valkey pod | N/A | ~32-96MB | N/A |
| Litestream sidecar | ~32-64MB | ~32-64MB | ~32-64MB |
| Total RAM | ~82-114MB | ~114-210MB | ~87-134MB |
| Additional pods | 0 | 1 (Valkey) | 0 |
| Additional PVCs | 0 | 0 (cache-only, no persistence) | 0 (redb shares data PVC) |
| Additional Docker images | 0 | 1 (valkey:8-alpine, ~30MB) | 0 |

### Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| All Option B risks | (see Option B) | (see Option B) | (see Option B) |
| Cache-database inconsistency | Medium | Medium | Short TTL (5 min); explicit invalidation on writes; cache is non-authoritative |
| Valkey pod failure | Low | Low | Application falls back to direct SQLite queries; cache is optional |
| redb file corruption | Very low | Low | redb is ACID; cache can be rebuilt from SQLite (source of truth) |
| Cache invalidation bugs | Medium | Medium | Comprehensive test coverage; conservative TTL; manual cache-clear admin endpoint |
| Over-engineering for homelab | High | Low | Document that caching layer is optional; default deployment is Option B without cache |
| Valkey license changes | Very low | Low | Valkey is Linux Foundation governed; unlikely to change. Can also self-host Redis <7.4 (BSD licensed) |

**Overall risk: Medium-High.** The caching layer adds complexity and failure modes that are disproportionate to the benefit for a single-pod homelab. The primary risk is over-engineering: spending development effort on optimization that doesn't meaningfully improve the user experience. However, if multi-pod deployment or auth performance becomes a real requirement, the hybrid approach provides a clear upgrade path from Option B.

---

## Caching Strategy Evaluation

This section evaluates caching strategies independently from the primary database choice. Regardless of whether Kubarr uses PostgreSQL (Option A), SQLite (Option B), or a hybrid approach (Option C), the caching layer serves a distinct purpose: reducing repetitive lookups for transient, reconstructable data. The evaluation below compares three approaches and analyzes Kubarr's actual caching needs to determine the best fit for a homelab context.

### Kubarr's Actual Caching Needs

Before evaluating solutions, it is essential to understand what Kubarr actually caches and whether those workloads genuinely benefit from caching infrastructure.

#### EndpointCache — Kubernetes Service Discovery

The `EndpointCache` in `state.rs` caches the results of Kubernetes API calls that resolve app names to service endpoints (base URL and base path). This avoids hitting the K8s API server on every proxy request.

```rust
pub struct EndpointCache {
    cache: Arc<RwLock<HashMap<String, CachedEndpoint>>>,
    ttl: Duration, // 60 seconds
}
```

**Characteristics:**
- **Data is transient** — Endpoint mappings are derived from live Kubernetes service state. They change only when services are created, updated, or deleted — events that happen rarely in a homelab (minutes to hours apart, not seconds).
- **Data is fully reconstructable** — On cache miss or pod restart, a single K8s API call reconstructs the entry. There is no data loss risk; the source of truth is the Kubernetes API server.
- **Access pattern** — Read-heavy with infrequent writes. Most requests hit the same small set of endpoints (the user's installed apps).
- **Size** — Bounded by the number of apps managed by Kubarr. A typical homelab has 5-20 apps, each producing one cache entry (~200 bytes). Total: <5KB.
- **TTL** — 60 seconds. Short enough to pick up service changes quickly; long enough to avoid K8s API pressure.

**Current issues:**
1. Lazy expiration only — expired entries are never removed, just ignored on read
2. No size bound — theoretically unbounded, though practically limited by the number of apps
3. No background cleanup — stale entries from deleted apps accumulate indefinitely

#### NetworkMetricsCache — Rate Calculation State

The `NetworkMetricsCache` stores cumulative network counters and computes sliding-window rate averages for dashboard display.

```rust
pub struct NetworkMetricsCache {
    cache: Arc<RwLock<HashMap<String, CachedNetworkMetrics>>>,
    max_age: Duration, // 5 minutes
}
```

**Characteristics:**
- **Data is transient** — Rate calculations are derived from sequential counter snapshots. The cache holds recent samples to compute a sliding-window average.
- **Data is reconstructable** — After a pod restart, the first few polling intervals produce incomplete rate data (needs ≥2 samples to compute a delta), but the cache self-heals within 2-3 polling cycles (~20-30 seconds).
- **Access pattern** — Write-on-poll (every ~10 seconds per namespace), read-on-dashboard-request. Writes and reads are roughly balanced.
- **Size** — One entry per monitored namespace. A typical homelab has 3-10 namespaces. Each entry contains a sliding window of 5 `RateSample` structs (~320 bytes) plus metadata. Total: <5KB.
- **TTL** — 5-minute max age. Stale entries indicate a namespace has stopped being polled (possibly deleted).
- **Smoothing** — Exponential Moving Average (EMA) smoothing prevents abrupt rate jumps. This stateful computation benefits from cache continuity but recovers quickly from a cold start.

**Current issues:**
1. No background eviction — entries for deleted namespaces remain in memory
2. No size bound — one entry per namespace, but no cap
3. Cold-start gap — 2-3 polling intervals of incomplete rate data after restart

#### Key Insight: Both Caches Are Trivially Small and Fully Transient

Neither cache stores durable, authoritative data. Both are derived views of external state (Kubernetes API, cAdvisor metrics) that can be reconstructed from their sources within seconds. The total memory footprint of both caches combined is under 10KB for a typical homelab deployment.

This means the caching strategy should prioritize **simplicity and correctness** over advanced features like persistence, replication, or distributed consistency. The cold-start penalty (a few seconds of stale/missing data after pod restart) is acceptable for a homelab dashboard.

### Strategy 1: Improved In-Memory Caching

**Approach:** Enhance the existing `Arc<RwLock<HashMap>>` caches with background eviction and size bounds. No new dependencies, no new infrastructure, no architectural changes.

#### Proposed Improvements

**Background eviction via `tokio::spawn` interval task:**

```rust
// Add to application startup after AppState is created
let endpoint_cache = app_state.endpoint_cache.clone();
let metrics_cache = app_state.network_metrics_cache.clone();

tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(300)); // Every 5 minutes
    loop {
        interval.tick().await;
        let evicted_endpoints = endpoint_cache.evict_expired().await;
        let evicted_metrics = metrics_cache.evict_stale().await;
        if evicted_endpoints > 0 || evicted_metrics > 0 {
            tracing::debug!(
                "Cache cleanup: evicted {} endpoints, {} metrics entries",
                evicted_endpoints,
                evicted_metrics
            );
        }
    }
});
```

**Eviction methods on `EndpointCache`:**

```rust
impl EndpointCache {
    /// Remove all expired entries from the cache.
    /// Returns the number of entries evicted.
    pub async fn evict_expired(&self) -> usize {
        let mut cache = self.cache.write().await;
        let before = cache.len();
        cache.retain(|_, entry| entry.expires_at > Instant::now());
        before - cache.len()
    }
}
```

**Eviction methods on `NetworkMetricsCache`:**

```rust
impl NetworkMetricsCache {
    /// Remove all stale entries from the cache.
    /// Returns the number of entries evicted.
    pub async fn evict_stale(&self) -> usize {
        let mut cache = self.cache.write().await;
        let before = cache.len();
        let max_age = self.max_age;
        cache.retain(|_, entry| entry.timestamp.elapsed() < max_age);
        before - cache.len()
    }
}
```

**Max-size bounds:**

```rust
impl EndpointCache {
    const MAX_ENTRIES: usize = 100; // Far more than any homelab needs

    pub async fn set(&self, key: String, endpoint: CachedEndpoint) {
        let mut cache = self.cache.write().await;

        // Enforce max size: evict expired first, then oldest if still over limit
        if cache.len() >= Self::MAX_ENTRIES && !cache.contains_key(&key) {
            cache.retain(|_, entry| entry.expires_at > Instant::now());
            if cache.len() >= Self::MAX_ENTRIES {
                // Remove the entry closest to expiration
                if let Some(oldest_key) = cache.iter()
                    .min_by_key(|(_, v)| v.expires_at)
                    .map(|(k, _)| k.clone())
                {
                    cache.remove(&oldest_key);
                }
            }
        }

        cache.insert(key, endpoint);
    }
}
```

#### Assessment

| Criterion | Rating | Notes |
|-----------|--------|-------|
| Complexity | ✅ Minimal | ~50 lines of new code; no new dependencies |
| Operational overhead | ✅ None | No new pods, no new services, no configuration |
| Memory usage | ✅ Negligible | <10KB for both caches combined in typical deployments |
| Cold-start penalty | ⚠️ Unchanged | Caches are empty after pod restart; rebuilds within seconds |
| Eviction | ✅ Fixed | Background task cleans up every 5 minutes |
| Size bounds | ✅ Fixed | Max entries cap prevents unbounded growth |
| Persistence | ❌ None | Data lost on pod restart (acceptable for transient data) |
| Multi-pod support | ❌ None | Per-process cache; no sharing across pods |

### Strategy 2: External Redis/Valkey

**Approach:** Deploy a Redis or Valkey instance alongside the backend pod and use it as a shared cache for endpoint lookups, network metrics, and potentially auth-layer session data.

#### What It Provides

- **Native TTL expiration** — `SET key value EX 60` automatically expires entries without background tasks. Redis/Valkey handles eviction internally with configurable policies (`allkeys-lru`, `volatile-ttl`, etc.).
- **Persistence across restarts** — Even without RDB/AOF persistence, a separate Valkey pod survives backend pod restarts. If persistence is enabled, cache data survives Valkey pod restarts too.
- **Session sharing for multi-pod** — If Kubarr ever scales to multiple replicas, all pods can share session state via Valkey. In-memory caches cannot be shared across processes.
- **Pub/Sub for WebSockets** — Valkey's `PUBLISH`/`SUBSCRIBE` enables cross-pod event broadcasting. The current Tokio broadcast channels are process-local.
- **Atomic operations** — `INCR`, `DECR`, `GETSET` for rate limiting and counters without application-level locking.

#### What It Costs

| Cost | Details |
|------|---------|
| **Additional pod** | Valkey runs as a separate Deployment (~32-96MB RAM, ~10-100m CPU) |
| **Network latency** | Every cache operation is a network round-trip (~0.1-0.5ms within the same node). For <10KB of cache data, this overhead may exceed the savings from avoiding the original K8s API call or metric computation. |
| **Operational complexity** | Another service to deploy, monitor, upgrade, and debug. Valkey configuration (maxmemory, eviction policy, persistence settings) must be understood by homelab operators. |
| **New dependency** | `fred` crate (~2MB binary size increase); Valkey Docker image (~30MB); Kubernetes manifests for Deployment + Service. |
| **Connection management** | Must handle Valkey connection failures gracefully. If Valkey is unavailable, the backend must fall back to direct source lookups (K8s API, cAdvisor). |
| **Over-engineering risk** | For caching <10KB of transient, reconstructable data, an external key-value store is a disproportionate solution. |

#### Assessment

| Criterion | Rating | Notes |
|-----------|--------|-------|
| Complexity | ❌ Significant | New dependency, new pod, new configuration |
| Operational overhead | ❌ High | Another service for homelab operators to manage |
| Memory usage | ⚠️ Moderate | +32-96MB for Valkey pod; disproportionate for <10KB of cached data |
| Cold-start penalty | ✅ Eliminated | Cache survives backend pod restarts (if Valkey stays up) |
| Eviction | ✅ Native | Built-in TTL and LRU eviction policies |
| Size bounds | ✅ Native | `maxmemory` with configurable eviction policy |
| Persistence | ✅ Optional | RDB/AOF available if needed |
| Multi-pod support | ✅ Full | Shared state, pub/sub, atomic operations |

**Verdict:** Redis/Valkey is the right choice if Kubarr needs multi-pod deployment, shared sessions, or cross-pod pub/sub. For a single-pod homelab caching <10KB of transient data, it adds significant complexity for marginal benefit.

### Strategy 3: Embedded redb

**Approach:** Use the `redb` crate (pure-Rust B+ tree key-value store) to persist cache data to disk within the backend process. Cache entries survive pod restarts without external infrastructure.

#### What It Provides

- **ACID transactions** — Full crash safety with transactional reads and writes. Data is never in an inconsistent state, even after unexpected termination.
- **Persistent cache** — Cache data survives pod restarts. Endpoint lookups and network rate history are immediately available after restart, eliminating cold-start gaps.
- **No network overhead** — Embedded in the backend process; no TCP round-trips for cache operations. Read latency is comparable to in-memory access when data is in the OS page cache.
- **Zero external dependencies** — No new pods, no new services. Just a file on the existing PVC.

#### What It Costs

| Cost | Details |
|------|---------|
| **Single-process only** | redb, like SQLite, is single-writer. Cannot be shared across multiple pods. |
| **No native TTL** | Expiration must be implemented manually (expiry timestamp column + periodic cleanup), similar to the current in-memory approach. |
| **Disk I/O for cache ops** | Every cache write triggers a disk fsync (ACID guarantee). For a cache that writes on every metric poll (~every 10 seconds), this adds ~1ms of latency per write on SSD. |
| **Two embedded databases** | If using SQLite for the primary database, adding redb means two embedded storage engines with separate file management, backup considerations, and potential lock contention. |
| **New dependency** | `redb` crate (~1MB binary size increase). Minimal, but non-zero. |
| **Complexity vs. benefit** | Persisting <10KB of transient, reconstructable data to disk provides marginal benefit. The cold-start penalty (2-3 polling cycles, ~20-30 seconds) is acceptable for a homelab dashboard. |

#### Assessment

| Criterion | Rating | Notes |
|-----------|--------|-------|
| Complexity | ⚠️ Moderate | New dependency, manual TTL implementation, file management |
| Operational overhead | ✅ Low | No new pods; embedded in backend process |
| Memory usage | ✅ Low | ~5-20MB for memory-mapped file (mostly OS page cache) |
| Cold-start penalty | ✅ Eliminated | Cache data persisted to disk, available immediately |
| Eviction | ⚠️ Manual | Must implement TTL checking and background cleanup (same as in-memory) |
| Size bounds | ⚠️ Manual | Must implement max-entries logic (same as in-memory) |
| Persistence | ✅ Full | ACID-guaranteed crash safety |
| Multi-pod support | ❌ None | Single-process embedded database |

**Verdict:** redb is a well-designed embedded store, but for Kubarr's caching needs it solves a problem that barely exists. The cold-start penalty it eliminates (20-30 seconds of incomplete rate data) does not justify the added complexity of a second embedded database engine, manual TTL implementation, and disk I/O overhead on cache writes.

### Comparison Matrix

| Criterion | Improved In-Memory | External Redis/Valkey | Embedded redb |
|-----------|-------------------|----------------------|---------------|
| **New dependencies** | None | `fred` crate + Valkey pod | `redb` crate |
| **Implementation effort** | ~2-4 hours | ~18 hours | ~11 hours |
| **Lines of code** | ~50 | ~300+ | ~150+ |
| **Additional pods** | 0 | 1 | 0 |
| **Additional RAM** | 0 | +32-96MB | +5-20MB |
| **Persistence** | ❌ | ✅ | ✅ |
| **Multi-pod support** | ❌ | ✅ | ❌ |
| **Native TTL** | ❌ (manual) | ✅ | ❌ (manual) |
| **Operational complexity** | ✅ None added | ❌ Significant | ⚠️ Moderate |
| **Solves current issues** | ✅ Eviction + bounds | ✅ All + persistence | ✅ All + persistence |
| **Proportionate to need** | ✅ Yes | ❌ No (for homelab) | ⚠️ Borderline |

### Recommendation: Improved In-Memory Caching

**For Kubarr's homelab context, improved in-memory caching is the recommended strategy.** The rationale:

1. **The data is transient and reconstructable.** Both `EndpointCache` and `NetworkMetricsCache` derive their contents from external sources (Kubernetes API, cAdvisor) that are always available. There is no durable state to protect. Persisting cache entries across pod restarts saves at most 20-30 seconds of incomplete dashboard data — a negligible improvement for a homelab user.

2. **The data volume is trivially small.** Both caches combined hold <10KB in a typical deployment. Deploying a 32-96MB Valkey pod or adding a second embedded database engine to cache <10KB is a disproportionate response. The improved in-memory approach handles this with ~50 lines of code and zero new infrastructure.

3. **The current issues are minor and easily fixed.** The two real problems — lack of background eviction and lack of size bounds — are solved with a `tokio::spawn` interval task and a max-entries check in `set()`. These are ~50 lines of straightforward Rust code with no new dependencies.

4. **Operational simplicity is a core decision driver.** Kubarr targets homelab operators, not SREs. Every additional component (Valkey pod, redb file, new crate dependency) increases the surface area for debugging, monitoring, and maintenance. The in-memory approach adds zero operational burden.

5. **Multi-pod and persistence are not current requirements.** If Kubarr ever needs shared sessions or cross-pod pub/sub, the architecture can be extended to include Valkey at that point. The in-memory cache design does not preclude adding an external cache later — it simply avoids adding one prematurely.

6. **The cold-start penalty is acceptable.** After a pod restart, `EndpointCache` repopulates on the first request to each app (~1 K8s API call, <100ms). `NetworkMetricsCache` needs 2-3 polling intervals (~20-30 seconds) to produce accurate rate averages. For a homelab dashboard, this brief warm-up period is entirely acceptable.

**Implementation priority:** The improved in-memory caching changes (background eviction + max-size bounds) should be implemented as part of the "Quick Wins" regardless of which primary database option (A, B, or C) is chosen. They are orthogonal to the database decision and address real (if minor) issues in the current architecture.

---

## Recommendation

### Decision: Option B — SQLite + Litestream

**Kubarr should migrate from PostgreSQL/CloudNativePG to embedded SQLite with Litestream continuous replication.** This is the recommended storage architecture for the following reasons:

#### Rationale

1. **Operational simplicity is the primary driver.** Kubarr targets homelab operators who manage their own infrastructure. The current architecture requires installing the CloudNativePG operator, creating a CNPG Cluster custom resource, understanding CNPG backup configuration, and managing PostgreSQL pod lifecycle — all for a single-user application with <100MB of relational data. SQLite eliminates this entire layer. The database becomes a file inside the backend pod, managed by SeaORM the same way it manages PostgreSQL today.

2. **Resource savings are significant for constrained hardware.** Eliminating the CNPG operator pod (~128MB RAM) and PostgreSQL pod (~256MB RAM) saves ~325MB of RAM after accounting for the Litestream sidecar (~64MB). On a Raspberry Pi 4 (4GB RAM) or mini PC (8GB RAM), this is a meaningful reduction — roughly 8-10% of total system memory returned to other workloads.

3. **Fewer failure modes improve reliability.** The current architecture has three distinct failure points for database operations: the backend pod, the PostgreSQL pod, and the network path between them (including connection pool management, TCP timeouts, and CNPG failover logic). SQLite reduces this to one: the backend pod and its local file. There are no network partitions, no connection pool exhaustion, no CNPG operator bugs, and no PostgreSQL process crashes to handle.

4. **The migration is feasible and low-risk.** All 22 entity models use SeaORM-portable types (`i64` primary keys, `String`, `DateTimeUtc`, `bool`, `Option<T>`). No PostgreSQL-specific column types (arrays, enums, JSONB operators) are used in queries. Both `sqlx-postgres` and `sqlx-sqlite` feature flags are already compiled in `Cargo.toml`. SeaORM's `Database::connect()` selects the correct driver based on the URL scheme, meaning the same binary can run against either backend.

5. **Backup is simpler and more reliable.** CNPG backups require configuring Barman, S3 credentials, retention policies, and scheduled backups — all of which most homelab users never set up, leaving them with no backup at all. Litestream replicates every WAL frame to S3 within seconds, automatically, with a single ConfigMap and Secret. The free tier of Backblaze B2 (10GB) is more than sufficient for Kubarr's database size (<50MB).

6. **Single-pod constraint is a non-issue.** Kubarr is already designed for `replicas: 1`. The single-writer limitation of SQLite perfectly matches the single-pod deployment model. There is no current or planned need for horizontal scaling. If that requirement ever emerges, the dual feature flags allow switching back to PostgreSQL by changing `DATABASE_URL`.

### Comparison Matrix

| Criterion | Option A: Optimized PostgreSQL | Option B: SQLite + Litestream | Option C: Hybrid (SQLite + Cache) |
|-----------|-------------------------------|-------------------------------|-----------------------------------|
| **Operational complexity** | ⚠️ High — CNPG operator, CRDs, PG pod, backup config | ✅ Low — embedded DB, Litestream sidecar | ❌ Medium-High — SQLite + Valkey/redb |
| **Resource footprint** | ❌ ~484-690MB RAM (PG + operator + tuning) | ✅ ~82-114MB RAM (backend + Litestream) | ⚠️ ~114-210MB RAM (+ cache layer) |
| **Migration effort** | ✅ Minimal (1-2 days, no schema changes) | ⚠️ Moderate (3-4 days, deployment model change) | ❌ Significant (5-6 days, deployment + cache) |
| **Data durability** | ✅ CNPG WAL archiving, PITR, optional HA | ✅ Litestream S3 replication, <1s RPO | ✅ Same as Option B |
| **Resilience (pod restart)** | ⚠️ Depends on PG pod + CNPG failover | ✅ PVC data intact; Litestream restore if needed | ✅ Same as Option B + warm cache |
| **Scalability** | ✅ Can add read replicas via CNPG | ⚠️ Single-pod only (sufficient for homelab) | ⚠️ Single-pod + shared cache possible |
| **Query performance** | ✅ Full PostgreSQL optimizer | ✅ In-process, no network latency (~10x faster for simple queries) | ✅ In-process + cached hot paths |
| **Advanced SQL features** | ✅ Full PostgreSQL (LISTEN/NOTIFY, GIN, JSONB) | ⚠️ Standard SQL only (sufficient — no PG features used) | ⚠️ Same as Option B |
| **Backup simplicity** | ⚠️ Requires Barman/S3 config (often skipped) | ✅ Automatic via Litestream sidecar | ✅ Same as Option B |
| **Development complexity** | ✅ No changes to existing code patterns | ⚠️ SQLite PRAGMAs, StatefulSet, migration tooling | ❌ All Option B + cache invalidation logic |
| **Pod count** | ❌ 3 pods (backend + PG + operator) | ✅ 1 pod (backend + sidecar) | ⚠️ 1-2 pods (+ Valkey if external) |
| **SeaORM 2.0 compatibility** | ✅ Full | ✅ Full (Entity First Workflow supports SQLite) | ✅ Full |
| **Rollback path** | ✅ Already running | ✅ Change `DATABASE_URL` back to postgres:// | ✅ Same as Option B |

### Quick Wins — Implement Regardless of Chosen Option

These improvements address real issues in the current codebase and should be implemented immediately, independent of the database architecture decision. They are low-risk, independently deployable, and provide value whether Kubarr stays on PostgreSQL or migrates to SQLite.

#### Quick Win 1: Environment-Configurable Connection Pool

**Issue:** Connection pool size is hardcoded at 10 max / 1 min in `database.rs`, wasting ~50MB of PostgreSQL memory and requiring a code change to tune.

**Fix:** Read pool configuration from environment variables with sensible defaults.

```rust
let max_conns = std::env::var("KUBARR_DB_MAX_CONNECTIONS")
    .ok().and_then(|v| v.parse().ok()).unwrap_or(5);
let min_conns = std::env::var("KUBARR_DB_MIN_CONNECTIONS")
    .ok().and_then(|v| v.parse().ok()).unwrap_or(1);
```

**Effort:** ~1 hour | **Risk:** Very low | **Impact:** Configurable without rebuild; reduced default memory usage

#### Quick Win 2: Audit Stats GROUP BY Query

**Issue:** `get_audit_stats()` loads every audit log row into memory to count actions. This is an unbounded memory allocation that will eventually cause OOM as the audit log grows.

**Fix:** Replace with a SQL `GROUP BY` aggregation that returns only the counted results.

```rust
let action_counts = audit_log::Entity::find()
    .select_only()
    .column(audit_log::Column::Action)
    .column_as(audit_log::Column::Id.count(), "count")
    .group_by(audit_log::Column::Action)
    .into_model::<ActionCount>()
    .all(db).await?;
```

**Effort:** ~2 hours | **Risk:** Low | **Impact:** Eliminates critical OOM risk; constant memory regardless of log size

#### Quick Win 3: Background Cache Eviction

**Issue:** Both `EndpointCache` and `NetworkMetricsCache` use lazy expiration only. Expired entries are never removed from the HashMap, causing unbounded memory growth from dead entries.

**Fix:** Add a `tokio::spawn` interval task that periodically iterates both caches and removes expired/stale entries.

```rust
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(300));
    loop {
        interval.tick().await;
        endpoint_cache.evict_expired().await;
        metrics_cache.evict_stale().await;
    }
});
```

**Effort:** ~2 hours | **Risk:** Low | **Impact:** Prevents unbounded cache memory growth

#### Quick Win 4: Cache Size Limits

**Issue:** Neither cache has a maximum size limit. `EndpointCache` has no natural bound on entry count.

**Fix:** Add a `MAX_ENTRIES` constant and enforce it in `set()` by evicting expired entries first, then the oldest entry if still over the limit.

**Effort:** ~1 hour | **Risk:** Very low | **Impact:** Hard upper bound on cache memory consumption

#### Quick Win 5: Scheduled Audit Log Retention

**Issue:** No automatic retention policy. The `clear_old_logs()` function exists but requires manual admin API calls. Without scheduled cleanup, the audit_log table grows indefinitely.

**Fix:** Wrap `clear_old_logs()` in a `tokio::spawn` daily task with a configurable retention period (default: 90 days via `KUBARR_AUDIT_RETENTION_DAYS`).

**Effort:** ~1 hour | **Risk:** Low | **Impact:** Bounded audit log table size (~50K rows with 90-day retention)

#### Quick Wins Summary

| Quick Win | Effort | Risk | Blocks on DB Decision? |
|-----------|--------|------|----------------------|
| Configurable connection pool | ~1 hour | Very low | No |
| Audit stats GROUP BY | ~2 hours | Low | No |
| Background cache eviction | ~2 hours | Low | No |
| Cache size limits | ~1 hour | Very low | No |
| Scheduled audit log retention | ~1 hour | Low | No |
| **Total** | **~7 hours** | **Low** | **No — implement now** |

### Migration Roadmap

The migration from PostgreSQL to SQLite is organized into four phases. Each phase is independently deployable and includes a rollback path.

#### Phase 0: Quick Wins (Week 1)

Implement all five quick wins listed above. These improvements benefit the current PostgreSQL setup and are prerequisites for a healthy migration baseline.

**Deliverables:**
- Environment-configurable connection pool in `database.rs`
- SQL GROUP BY for audit stats in `audit.rs`
- Background eviction task for both caches in application startup
- Max-entries bounds on `EndpointCache`
- Scheduled audit log retention task

**Rollback:** Each change is independently revertible. No schema changes.

#### Phase 1: SQLite Compatibility (Week 2)

Verify and fix all 22 SeaORM migrations for SQLite compatibility. Add SQLite PRAGMA configuration to `database.rs`. Run integration tests against SQLite.

**Deliverables:**
- SQLite PRAGMA configuration (`journal_mode=WAL`, `busy_timeout=5000`, `synchronous=NORMAL`, `foreign_keys=ON`)
- All 22 migrations verified on SQLite (fix any PostgreSQL-specific DDL)
- Integration test suite passing against both PostgreSQL and SQLite
- `DATABASE_URL` scheme detection for automatic backend selection

**Rollback:** No production changes yet. All work is in CI/development.

#### Phase 2: Kubernetes Deployment Change (Week 3)

Convert the backend Deployment to a StatefulSet with Litestream sidecar. Create ConfigMap for Litestream configuration and Secret for S3 credentials.

**Deliverables:**
- StatefulSet Helm template (replaces Deployment) with `replicas: 1` and `updateStrategy: Recreate`
- Litestream init container (restore) and sidecar container (replicate)
- Litestream ConfigMap and Secret for Backblaze B2
- VolumeClaimTemplate for SQLite data PVC
- Updated `values.yaml` with SQLite-specific configuration
- Health check endpoints verified with SQLite backend

**Rollback:** Keep the old Deployment template. Switch back by changing `DATABASE_URL` to postgres:// and deploying the Deployment instead of StatefulSet.

#### Phase 3: Data Migration and Cutover (Week 4)

Export data from PostgreSQL, import into SQLite, verify row counts, and cut over to the SQLite deployment.

**Deliverables:**
- Data export script (`pg_dump --data-only --inserts`)
- Data import script with PostgreSQL→SQLite type conversions
- Row count verification across all 22 tables
- Litestream initial backup to S3 confirmed
- CNPG operator and PostgreSQL resources removed from cluster
- CNPG RBAC rules removed from `values.yaml`

**Rollback:** PostgreSQL data remains intact during migration. If issues are found, switch `DATABASE_URL` back and redeploy with the Deployment template. CNPG resources can be recreated from Helm.

#### Migration Timeline Summary

```
Week 1: Quick Wins (no deployment model change)
  ├── Configurable pool, GROUP BY, cache eviction, size limits, audit retention
  └── All changes benefit current PostgreSQL setup

Week 2: SQLite Compatibility (development/CI only)
  ├── Migration compatibility testing
  ├── PRAGMA configuration
  └── Dual-backend integration tests

Week 3: Kubernetes Deployment (staging/dev cluster)
  ├── StatefulSet + Litestream sidecar
  ├── S3 backup configuration
  └── Health check verification

Week 4: Data Migration and Cutover (production)
  ├── Export PostgreSQL → Import SQLite
  ├── Verify data integrity
  ├── Cut over to SQLite deployment
  └── Remove CNPG resources
```

**Total estimated effort:** ~35 hours (Quick Wins: ~7h + SQLite migration: ~28h)

### SeaORM 2.0 Compatibility Note

SeaORM 2.0 (currently in release candidate) introduces the **Entity First Workflow**, which allows defining entities in Rust and automatically generating or synchronizing the database schema — reversing the traditional migration-first approach.

**Impact on this recommendation:**

- **Option B (SQLite) is fully compatible with SeaORM 2.0.** The Entity First Workflow supports SQLite as a backend. Entity definitions using `DeriveEntityModel` will work identically on both PostgreSQL and SQLite.
- **Existing migrations continue to work.** SeaORM 2.0 does not deprecate the migration-first workflow. The 22 existing migrations will continue to function. The Entity First Workflow is an *additional* option, not a replacement.
- **Dual feature flags remain valid.** Keeping both `sqlx-postgres` and `sqlx-sqlite` compiled ensures compatibility with SeaORM 2.0's auto-schema-sync feature, which can target either backend.
- **No blocking conflicts.** The SQLite migration recommended here does not introduce any patterns or dependencies that conflict with a future SeaORM 2.0 upgrade. If anything, the simplified deployment (no external database to coordinate schema changes with) makes schema sync easier.

**Recommendation:** Proceed with the SQLite migration on SeaORM 1.1. Upgrade to SeaORM 2.0 as a separate effort after the storage model migration is complete and stable. Do not combine both changes — they are independent and combining them doubles the risk surface.

### Decision Timeline and Next Steps

| Step | Action | Owner | Timeline |
|------|--------|-------|----------|
| 1 | **Accept or reject this ADR** | Kubarr maintainers | Within 1 week |
| 2 | **Implement Quick Wins (Phase 0)** | Backend developer | Week 1 (regardless of ADR decision) |
| 3 | **If accepted:** Begin SQLite compatibility work (Phase 1) | Backend developer | Week 2 |
| 4 | **If accepted:** Kubernetes deployment changes (Phase 2) | Backend + infra developer | Week 3 |
| 5 | **If accepted:** Data migration and cutover (Phase 3) | Backend developer | Week 4 |
| 6 | **If rejected:** Continue with Option A (Optimized PostgreSQL) | Backend developer | Ongoing |
| 7 | **Post-migration:** Monitor for 2 weeks, then remove PostgreSQL/CNPG | Backend developer | Week 6 |
| 8 | **Future:** Evaluate SeaORM 2.0 upgrade | Backend developer | After migration stabilizes |

**Key decision point:** If the maintainers prefer to avoid the migration effort (~28 hours) and accept the CNPG operational overhead, Option A (Optimized PostgreSQL) is a valid alternative. The Quick Wins from Phase 0 deliver the most impactful improvements with the least effort and should be implemented regardless.

**This ADR recommends Option B (SQLite + Litestream) as the best fit for Kubarr's homelab context**, balancing operational simplicity, resource efficiency, and data durability against a manageable one-time migration effort.

---

## Decision

**Status:** Proposed — awaiting maintainer review.

**Proposed:** Option B — SQLite + Litestream, with all Quick Wins implemented in Phase 0.

**Alternatives considered:** Option A (Optimized PostgreSQL), Option C (Hybrid SQLite + Cache). See sections above for full analysis.
