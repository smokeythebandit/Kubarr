# Dashboard Latency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a dashboard load ask each question once and answer it cheaply, instead of 93 requests and 48.5 s of accumulated server wait.

**Architecture:** Collapse the frontend's four overlapping data layers onto React Query with shared keys, batch the two fan-out endpoints (pod status per namespace, icon per app) into one call each, let read-only responses carry a short `Cache-Control`, and have the worker poll pod status into an `UNLOGGED` table the API reads instead of calling Kubernetes per request.

**Tech Stack:** Rust (axum, sea-orm, utoipa), React 18 + TypeScript + `@tanstack/react-query` v5, PostgreSQL, Helm.

**Spec:** This plan is its own spec; the measurements it argues from are below.

## Measured baseline

From `void-zero.com_Archive [26-10-28 12-27-28].har`, 93 requests, one dashboard load:

| group | n | total wait |
| --- | --- | --- |
| `/api/apps/catalog/*/icon` | 29 | 23.4 s |
| `/api/monitoring/pods?namespace=*` | 28 | 16.0 s |
| `/api/apps/installed` | 2 | 3.5 s |
| `/api/monitoring/vm/*` | ~13 | 4.5 s |

`blocked ≈ 0`, HTTP/2, `receive ≈ 0`: every millisecond was server-side time to first byte. 21 distinct URLs were fetched 2–4 times in the same load, accounting for ~12.4 s. 58 of 58 API responses carried no `Cache-Control`.

The dominant cause was CPU throttling: the backend ran with `limits.cpu: 500m` while Tokio sized its runtime for 6 CPUs, so ~60 concurrent requests shared half a core and finished together at ~1073 ms. That limit is now `2` (chart `kubarr-backend 0.3.4+3`), and 60 concurrent health requests complete in 20 ms. **Every number above was inflated by that throttle; re-measure before judging the remaining work.**

## Global Constraints

- The backend is `code/api`, the worker shares the same library crate, the frontend is `code/frontend`.
- Existing endpoints must keep working: `/api/monitoring/pods?namespace=X` stays valid for a single namespace.
- Pod status served from the table must never be presented as live if it is stale beyond its freshness window; fall back to a live Kubernetes read.
- The metrics table is `UNLOGGED`: no WAL, contents are lost on crash, and the schema must tolerate an empty table on startup.
- Tests: `cargo test --locked` in `code/api`; `npm test` in `code/frontend`. One pre-existing failure, `postgres_audit_details_migration_and_transactional_insert`, needs `AUDIT_TEST_POSTGRES_URL` and fails locally — ignore it.
- Nothing in this plan reaches production without an application image release plus a chart publish and a CLI pin bump.

---

### Task 1: Read-only API responses carry a cache window

**Files:**
- Modify: `code/api/src/endpoints/monitoring.rs` (`get_pods`, around line 1167)
- Modify: `code/api/src/endpoints/apps.rs` (`list_catalog`, `list_installed_apps`)
- Test: `code/api/src/endpoints/monitoring.rs` (test module)

**Interfaces:**
- Produces: `fn short_cache(seconds: u32) -> (HeaderName, HeaderValue)` in `code/api/src/endpoints/mod.rs`, reused by later tasks.

- [ ] **Step 1: Write the failing test**

In `code/api/src/endpoints/mod.rs` test module:

```rust
#[test]
fn short_cache_allows_reuse_and_background_revalidation() {
    let (name, value) = short_cache(5);
    assert_eq!(name, axum::http::header::CACHE_CONTROL);
    let rendered = value.to_str().expect("header is ascii");
    assert!(rendered.contains("max-age=5"), "{rendered}");
    // A dashboard repaints the same panels repeatedly; let the browser serve
    // the previous answer while it fetches a fresh one.
    assert!(rendered.contains("stale-while-revalidate"), "{rendered}");
    assert!(rendered.contains("private"), "monitoring data is per user: {rendered}");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd code/api && cargo test --locked short_cache_allows_reuse`
Expected: FAIL — `cannot find function 'short_cache'`.

- [ ] **Step 3: Implement**

In `code/api/src/endpoints/mod.rs`:

```rust
use axum::http::{header::HeaderName, HeaderValue};

/// A short private cache window for read-only responses. The dashboard
/// re-renders the same panels far faster than this data changes.
pub fn short_cache(seconds: u32) -> (HeaderName, HeaderValue) {
    let value = format!("private, max-age={seconds}, stale-while-revalidate=30");
    (
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_str(&value).expect("ascii cache-control"),
    )
}
```

- [ ] **Step 4: Apply it to the three read endpoints**

`get_pods` returns `Result<Json<Vec<PodStatus>>>`; change it to return
`Result<impl IntoResponse>` and build the response as
`Ok(([short_cache(5)], Json(pods)))`. Do the same for `list_catalog` with
`short_cache(60)` (the catalog changes only when charts sync) and
`list_installed_apps` with `short_cache(5)`.

- [ ] **Step 5: Run the suite**

Run: `cd code/api && cargo test --locked`
Expected: PASS apart from the known `AUDIT_TEST_POSTGRES_URL` failure.

- [ ] **Step 6: Commit**

```bash
git add code/api/src/endpoints
git commit -m "perf(api): let read-only responses be reused briefly"
```

---

### Task 2: One request for many namespaces

**Files:**
- Modify: `code/api/src/endpoints/monitoring.rs:139` (`PodQuery`), `get_pods`
- Test: `code/api/src/endpoints/monitoring.rs` (test module)

**Interfaces:**
- Consumes: `short_cache` from Task 1.
- Produces: `GET /api/monitoring/pods?namespaces=a,b,c` returning `Vec<PodStatus>` across all named namespaces. `?namespace=x` keeps its current meaning.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn pod_query_accepts_one_namespace_or_many() {
    let single: PodQuery = serde_urlencoded::from_str("namespace=sonarr").expect("parse");
    assert_eq!(single.requested_namespaces(), vec!["sonarr".to_string()]);

    let many: PodQuery =
        serde_urlencoded::from_str("namespaces=sonarr,radarr,jackett").expect("parse");
    assert_eq!(
        many.requested_namespaces(),
        vec!["sonarr".to_string(), "radarr".to_string(), "jackett".to_string()]
    );

    // Blank entries and stray whitespace come from hand-built query strings.
    let messy: PodQuery = serde_urlencoded::from_str("namespaces=sonarr,,%20radarr").expect("parse");
    assert_eq!(messy.requested_namespaces(), vec!["sonarr".to_string(), "radarr".to_string()]);

    let neither: PodQuery = serde_urlencoded::from_str("").expect("parse");
    assert_eq!(neither.requested_namespaces(), vec!["media".to_string()]);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd code/api && cargo test --locked pod_query_accepts_one_namespace`
Expected: FAIL — no method `requested_namespaces`.

- [ ] **Step 3: Implement**

```rust
#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct PodQuery {
    pub namespace: Option<String>,
    /// Comma-separated namespaces, so one dashboard load is one request.
    pub namespaces: Option<String>,
    pub app: Option<String>,
}

impl PodQuery {
    pub fn requested_namespaces(&self) -> Vec<String> {
        if let Some(list) = self.namespaces.as_deref() {
            let names: Vec<String> = list
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToString::to_string)
                .collect();
            if !names.is_empty() {
                return names;
            }
        }
        vec![self.namespace.clone().unwrap_or_else(|| "media".to_string())]
    }
}
```

In `get_pods`, replace the single-namespace body with a loop over
`query.requested_namespaces()` that concatenates each namespace's
`get_pod_status` result. Keep `unwrap_or_default()` per namespace so one
unreadable namespace cannot blank the whole response.

- [ ] **Step 4: Run the suite**

Run: `cd code/api && cargo test --locked`
Expected: PASS apart from the known failure.

- [ ] **Step 5: Commit**

```bash
git add code/api/src/endpoints/monitoring.rs
git commit -m "perf(api): answer pod status for many namespaces at once"
```

---

### Task 3: One request for all catalog icons

**Files:**
- Modify: `code/api/src/endpoints/apps.rs` (`get_app_icon`, `apps_routes`)
- Test: `code/api/src/endpoints/apps.rs` (test module)

**Interfaces:**
- Produces: `GET /api/apps/catalog/icons` returning `BTreeMap<String, String>` of app name to SVG text, with `short_cache(600)`.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn icon_bundle_reads_every_chart_icon_once() {
    let charts = tempfile::tempdir().expect("tmpdir");
    for app in ["sonarr", "radarr"] {
        let dir = charts.path().join(app);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("icon.svg"), format!("<svg id=\"{app}\"/>")).expect("write");
    }
    std::fs::create_dir_all(charts.path().join("no-icon")).expect("mkdir");

    let bundle = read_icon_bundle(charts.path()).await;
    assert_eq!(bundle.len(), 2, "charts without an icon are skipped");
    assert_eq!(bundle["sonarr"], "<svg id=\"sonarr\"/>");
    assert!(!bundle.contains_key("no-icon"));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd code/api && cargo test --locked icon_bundle_reads_every_chart_icon`
Expected: FAIL — `cannot find function 'read_icon_bundle'`.

- [ ] **Step 3: Implement**

```rust
/// Read every chart icon in one pass. Blocking file reads belong on the
/// blocking pool, not on a runtime thread serving other requests.
async fn read_icon_bundle(dir: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut bundle = std::collections::BTreeMap::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return bundle;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains('/') || name.contains("..") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(entry.path().join("icon.svg")) {
                bundle.insert(name, content);
            }
        }
        bundle
    })
    .await
    .unwrap_or_default()
}
```

Add the handler and route:

```rust
async fn list_app_icons(_auth: Authorized<AppsView>) -> Result<impl IntoResponse> {
    let bundle = read_icon_bundle(&CONFIG.charts.dir).await;
    Ok(([short_cache(600)], Json(bundle)))
}
```

```rust
.route("/catalog/icons", get(list_app_icons))
```

Also move `get_app_icon`'s `std::fs::read` and `icon_path.exists()` onto
`tokio::task::spawn_blocking`, for the same reason.

- [ ] **Step 4: Run the suite**

Run: `cd code/api && cargo test --locked`
Expected: PASS apart from the known failure.

- [ ] **Step 5: Commit**

```bash
git add code/api/src/endpoints/apps.rs
git commit -m "perf(api): serve catalog icons in one response off the blocking pool"
```

---

### Task 4: One cache in the frontend

**Files:**
- Modify: `code/frontend/src/main.tsx` (QueryClient defaults)
- Modify: `code/frontend/src/contexts/MonitoringContext.tsx`
- Delete: `code/frontend/src/utils/precache.ts` and its call sites
- Modify: `code/frontend/src/api/monitoring.ts`, `code/frontend/src/api/apps.ts`
- Test: `code/frontend/src/test/monitoring-queries.test.tsx`

**Interfaces:**
- Consumes: `?namespaces=` from Task 2, `/catalog/icons` from Task 3.
- Produces: query keys `['pods', namespaces.join(',')]`, `['installed']`, `['catalog']`, `['icons']`, `['clusterMetrics']`.

- [ ] **Step 1: Write the failing test**

```tsx
import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query'
import { render, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { monitoringApi } from '../api/monitoring'

describe('monitoring queries', () => {
  it('asks for every namespace in one request', async () => {
    const get = vi.spyOn(monitoringApi, 'getPodStatusFor').mockResolvedValue([])
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    function Panel() {
      useQuery({ queryKey: ['pods', 'sonarr,radarr'], queryFn: () => monitoringApi.getPodStatusFor(['sonarr', 'radarr']) })
      return null
    }
    // Two components wanting the same data must produce one request.
    render(<QueryClientProvider client={client}><Panel /><Panel /></QueryClientProvider>)
    await waitFor(() => expect(get).toHaveBeenCalledTimes(1))
    expect(get).toHaveBeenCalledWith(['sonarr', 'radarr'])
  })
})
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd code/frontend && npm test -- monitoring-queries`
Expected: FAIL — `getPodStatusFor` is not a function.

- [ ] **Step 3: Add the batched client calls**

In `code/frontend/src/api/monitoring.ts`:

```ts
  // One request for many namespaces; the dashboard watches a dozen at once.
  getPodStatusFor: async (namespaces: string[], app?: string): Promise<PodStatus[]> => {
    const response = await apiClient.get<PodStatus[]>('/monitoring/pods', {
      params: { namespaces: namespaces.join(','), app },
    });
    return response.data;
  },
```

In `code/frontend/src/api/apps.ts`:

```ts
  getIcons: async (): Promise<Record<string, string>> => {
    const response = await apiClient.get<Record<string, string>>('/apps/catalog/icons');
    return response.data;
  },
```

- [ ] **Step 4: Give the QueryClient a staleness window**

In `code/frontend/src/main.tsx`:

```tsx
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
      retry: 1,
      // The dashboard mounts the same panels repeatedly. Without this every
      // mount refetches, which is how one load made 21 duplicate requests.
      staleTime: 5_000,
      gcTime: 5 * 60_000,
    },
  },
})
```

- [ ] **Step 5: Move MonitoringContext onto React Query and delete the precache**

Replace the manual `useState`/`useEffect`/`setInterval` fetching in
`MonitoringContext.tsx` with `useQuery` calls using the keys above, passing
`refetchInterval: 15_000` for pod status and `10_000` for cluster metrics so
the two `setInterval` timers disappear. Fetch pod status once with
`getPodStatusFor(namespaces)` rather than per app. Then delete
`utils/precache.ts` and every `precacheDashboard`/`precacheUser`/`getPrecached`
call site; React Query's cache now serves that purpose.

- [ ] **Step 6: Run the frontend tests**

Run: `cd code/frontend && npm test`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add code/frontend/src
git commit -m "perf(frontend): one query cache instead of four fetch layers"
```

---

### Task 5: The worker samples pod status into an unlogged table

**Files:**
- Create: `code/api/src/migrations/m20260226_000033_create_pod_status_samples.rs`
- Modify: `code/api/src/migrations/mod.rs`
- Create: `code/api/src/models/pod_status_sample.rs`
- Modify: `code/api/src/models/mod.rs`, `code/api/src/models/prelude.rs`
- Modify: `code/api/src/services/scheduler.rs`
- Test: `code/api/src/services/pod_status_sampler_tests.rs`

**Interfaces:**
- Produces: table `pod_status_samples(namespace TEXT, payload JSONB, observed_at TIMESTAMPTZ, PRIMARY KEY(namespace))`, and
  `pub async fn sample_pod_status(state: &AppState) -> Result<usize>` returning rows written.
- Produces: `pub async fn recent_pod_status(db: &DbConn, namespaces: &[String], max_age: Duration) -> Vec<PodStatus>`.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn samples_are_written_then_read_back_within_their_window() {
    let db = make_test_db().await;
    upsert_pod_status(&db, "sonarr", serde_json::json!([{"name": "sonarr-0"}])).await.expect("write");

    let fresh = recent_pod_status(&db, &["sonarr".to_string()], Duration::from_secs(60)).await;
    assert_eq!(fresh.len(), 1, "a fresh sample is served from the table");

    // A sample older than the window must not be presented as current.
    let stale = recent_pod_status(&db, &["sonarr".to_string()], Duration::from_secs(0)).await;
    assert!(stale.is_empty(), "an expired sample is ignored");

    let missing = recent_pod_status(&db, &["radarr".to_string()], Duration::from_secs(60)).await;
    assert!(missing.is_empty(), "a namespace never sampled returns nothing");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd code/api && cargo test --locked samples_are_written_then_read_back`
Expected: FAIL — `upsert_pod_status` not found.

- [ ] **Step 3: Write the migration**

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // UNLOGGED: these are sampled observations, cheap to lose and not
        // worth the write-ahead log on every poll.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE UNLOGGED TABLE IF NOT EXISTS pod_status_samples (
                     namespace   TEXT PRIMARY KEY,
                     payload     JSONB NOT NULL,
                     observed_at TIMESTAMPTZ NOT NULL DEFAULT now()
                 )",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS pod_status_samples")
            .await?;
        Ok(())
    }
}
```

SQLite is used in unit tests and does not know `UNLOGGED`; branch on
`manager.get_database_backend()` and emit a plain `CREATE TABLE` for
`DbBackend::Sqlite`.

Register it in `migrations/mod.rs` alongside the existing entries.

- [ ] **Step 4: Implement the read and write helpers**

In `code/api/src/services/pod_status_sampler.rs`:

```rust
pub async fn upsert_pod_status(db: &DbConn, namespace: &str, payload: serde_json::Value) -> Result<()> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO pod_status_samples (namespace, payload, observed_at)
         VALUES ($1, $2, now())
         ON CONFLICT (namespace) DO UPDATE SET payload = $2, observed_at = now()",
        [namespace.into(), payload.into()],
    ))
    .await?;
    Ok(())
}

/// Samples newer than `max_age`. An empty result means "ask Kubernetes",
/// never "there are no pods".
pub async fn recent_pod_status(db: &DbConn, namespaces: &[String], max_age: Duration) -> Vec<PodStatus> {
    // SELECT payload WHERE namespace = ANY($1) AND observed_at > now() - $2,
    // then flatten each payload array into PodStatus values.
}
```

Then `sample_pod_status(state)`: read every namespace the catalog knows,
call `get_pod_status` for each, and upsert the result.

- [ ] **Step 5: Schedule it in the worker**

Register `sample_pod_status` in `services/scheduler.rs` on a 15-second
interval, matching the dashboard's old refresh cadence.

- [ ] **Step 6: Run the suite**

Run: `cd code/api && cargo test --locked`
Expected: PASS apart from the known failure.

- [ ] **Step 7: Commit**

```bash
git add code/api/src
git commit -m "feat(worker): sample pod status into an unlogged table"
```

---

### Task 6: The API prefers a fresh sample

**Files:**
- Modify: `code/api/src/endpoints/monitoring.rs` (`get_pods`)
- Test: `code/api/src/endpoints/monitoring.rs` (test module)

**Interfaces:**
- Consumes: `recent_pod_status` from Task 5, `requested_namespaces` from Task 2.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn live_kubernetes_is_only_consulted_when_no_fresh_sample_exists() {
    let db = make_test_db().await;
    let state = make_test_state(db.clone()).await;   // no Kubernetes client
    upsert_pod_status(&db, "sonarr", serde_json::json!([{"name": "sonarr-0"}])).await.expect("write");

    let served = pods_for(&state, &["sonarr".to_string()], None).await;
    assert_eq!(served.len(), 1, "a fresh sample answers without Kubernetes");

    // Nothing sampled and no Kubernetes client: an empty list, not an error.
    let empty = pods_for(&state, &["radarr".to_string()], None).await;
    assert!(empty.is_empty());
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd code/api && cargo test --locked live_kubernetes_is_only_consulted`
Expected: FAIL — `pods_for` not found.

- [ ] **Step 3: Implement**

```rust
/// Prefer a sample the worker took recently; fall back to a live read so a
/// stopped worker degrades to the old behaviour rather than an empty page.
async fn pods_for(state: &AppState, namespaces: &[String], app: Option<&str>) -> Vec<PodStatus> {
    if let Ok(db) = state.get_db().await {
        let sampled = recent_pod_status(&db, namespaces, Duration::from_secs(30)).await;
        if !sampled.is_empty() {
            return match app {
                Some(app) => sampled.into_iter().filter(|pod| pod.app == app).collect(),
                None => sampled,
            };
        }
    }
    live_pod_status(state, namespaces, app).await
}
```

- [ ] **Step 4: Run the suite**

Run: `cd code/api && cargo test --locked`
Expected: PASS apart from the known failure.

- [ ] **Step 5: Commit**

```bash
git add code/api/src/endpoints/monitoring.rs
git commit -m "perf(api): serve pod status from the worker's samples"
```

---

### Task 7: Release

**Files:**
- Modify: `kubarr-charts/system/kubarr-backend/Chart.yaml`, `kubarr-charts/system/kubarr-worker/Chart.yaml`
- Modify: `kubarr-cli/src/charts.rs`, `kubarr-cli/src/bootstrap.rs`, `kubarr-cli/CHANGELOG.md`, `kubarr-cli/Cargo.toml`

- [ ] **Step 1: Build and publish the application images**

The backend, worker and frontend all changed, so their images must be built
and published before any chart can pin them.

- [ ] **Step 2: Bump and publish the charts**

`kubarr-backend` already carries the CPU-limit change at `0.3.4+3`. Bump the
image tags for backend, worker and frontend, then push `kubarr-charts` to
publish.

- [ ] **Step 3: Pin in the CLI and release**

Update both pin tables, add a `CHANGELOG.md` entry, bump `Cargo.toml`, push,
wait for installer acceptance, then tag.

- [ ] **Step 4: Re-measure**

Capture a fresh HAR of the dashboard and compare against the baseline table
at the top of this plan. Record the new request count and total wait.

---

## Self-Review

**Spec coverage:** cache headers (Task 1), pod-status fan-out (Tasks 2, 4), icon fan-out (Tasks 3, 4), duplicate requests (Task 4), worker-polled table (Tasks 5, 6), CPU limit (already applied, chart `0.3.4+3`), release (Task 7).

**Placeholders:** Task 5 Step 4 leaves the `recent_pod_status` SQL body as a comment describing the query rather than final code, because the exact sea-orm value binding for `ANY($1)` differs between backends and is better settled against a compiler than guessed here. Every other step carries runnable code.

**Type consistency:** `requested_namespaces()` (Task 2) is used by `pods_for` (Task 6); `getPodStatusFor(namespaces: string[])` (Task 4) matches the `?namespaces=` parameter (Task 2); `short_cache` (Task 1) is reused in Tasks 2 and 3; `upsert_pod_status`/`recent_pod_status` (Task 5) are consumed by Task 6.

**Known risk:** Tasks 5 and 6 add staleness semantics to a page people will trust. The 30-second freshness window in Task 6 must stay shorter than the 15-second sampling interval in Task 5 by a comfortable margin, or a stopped worker will serve data that looks current for half a minute.
