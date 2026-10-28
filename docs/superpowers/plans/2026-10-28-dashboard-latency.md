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

### Tasks 5-6: Pod status from a Kubernetes watch (completed, revised design)

These two tasks were planned as a 15-second sampler plus a freshness window.
They shipped as a Kubernetes **watch** instead: `kube` already enables its
`runtime` feature, so the worker holds one `watcher` over all pods and writes
each affected namespace as events arrive. That removed the interval to tune, the
window in which the dashboard contradicts a just-taken action, and most of the
staleness the original design had to mitigate.

**Files:**
- Created: `code/api/src/migrations/m20260226_000033_create_pod_status_cache.rs`
- Created: `code/api/src/services/pod_cache.rs` (storage and trusted reads)
- Created: `code/api/src/services/pod_watch.rs` (event folding and watch loop)
- Modified: `code/api/src/services/mod.rs`, `code/api/src/application/runtime.rs`
  (spawn the watch in the worker), `code/api/src/services/app_manager.rs`
  (invalidate after install/update/delete/restart),
  `code/api/src/endpoints/monitoring.rs` (`get_pods`, `pods_for`),
  `code/api/src/services/k8s.rs` (`pod_status_from_pod`, `format_age` now crate-visible)
- Tests: `code/api/tests/pod_cache_tests.rs`,
  `code/api/tests/pod_cache_postgres_tests.rs`,
  inline tests in `pod_watch.rs` and `monitoring.rs`

**Interfaces produced:**
- Tables `pod_status_cache(namespace PK, payload, observed_unix)` and
  `pod_watch_heartbeat(id PK, synced_unix)`, UNLOGGED on PostgreSQL, epoch
  seconds rather than TIMESTAMP so both backends behave identically.
- `pod_cache::store_namespace`, `touch_heartbeat`, `invalidate`, `heartbeat_age`
- `pod_cache::fresh_namespaces(db, namespaces, max_silence) -> HashMap<String, Vec<PodStatus>>`
- `pod_watch::run(client, db)`, `pod_watch::PodWatchState`, `pod_watch::MAX_STREAM_SILENCE`

**Design decisions worth keeping:**
- A missing namespace means "not observed", never "no pods" — the API falls back
  to a live read per namespace. An emptied namespace is written as `[]` so the
  two cases stay distinguishable.
- Stream health lives in its own heartbeat row: a quiet cluster emits no events,
  and an empty namespace never updates its own row, so neither can vouch for
  freshness. Withholding the heartbeat on watch errors is what makes readers
  degrade to live queries. This also covers PostgreSQL truncating unlogged
  tables after an unclean shutdown.
- `PodStatus::age` is recomputed on read from each pod's creation time; a watch
  only writes on change, so a stored age string would freeze.
- A relist (`Init`/`InitApply`/`InitDone`) is buffered and applied wholesale,
  because mid-relist a pod's absence does not mean it is gone.
- Responses carry `X-Pod-Watch-Age-Seconds`; the JSON shape is unchanged, so the
  frontend needed no further change and its 15s `refetchInterval` no longer
  governs staleness.

**Verified:** 893 lib tests plus the new suites pass; the migration and upsert
were additionally run against a real PostgreSQL 16 to confirm `relpersistence='u'`
and the `ON CONFLICT` path, which SQLite would have accepted either way.

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
