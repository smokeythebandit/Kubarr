//! Leader-only loop that applies eligible chart updates inside the owner's
//! window, one app at a time. The operation itself (verify, rollback,
//! notifications) runs in the worker; this loop only decides what to enqueue.

use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
use tokio_util::sync::CancellationToken;

use crate::error::Result;
use crate::models::{app_operation, app_state};
use crate::services::app_manager::{
    AppManager, AUTO_TARGET_KEY, OBS_INSTALLED, OP_UPDATE, STATUS_FAILED, STATUS_PAUSED,
    STATUS_QUEUED, STATUS_RUNNING,
};
use crate::services::auto_update_settings;
use crate::services::catalog::AppCatalog;
use crate::services::update_policy::{auto_eligible, update_kind, window_bounds};
use crate::state::SharedCatalog;

const TICK_INTERVAL: Duration = Duration::from_secs(5 * 60);
const EXCLUDED_CATEGORIES: [&str; 2] = ["system", "monitoring"];
const KUBARR_PREFIX: &str = "kubarr-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickResult {
    Disabled,
    OutsideWindow,
    Busy,
    Enqueued(String),
    Nothing,
}

/// Apps that may be updated automatically right now, ordered by name.
pub async fn candidates<C: ConnectionTrait>(db: &C, catalog: &AppCatalog) -> Result<Vec<String>> {
    let busy: HashSet<String> = app_operation::Entity::find()
        .filter(app_operation::Column::Status.is_in([STATUS_QUEUED, STATUS_RUNNING, STATUS_PAUSED]))
        .all(db)
        .await?
        .into_iter()
        .map(|op| op.app_name)
        .collect();

    let states = app_state::Entity::find()
        .filter(app_state::Column::UpdateAvailable.eq(true))
        .filter(app_state::Column::AutoUpdate.eq(true))
        .filter(app_state::Column::ObservedState.eq(OBS_INSTALLED))
        .filter(app_state::Column::Healthy.eq(true))
        .order_by_asc(app_state::Column::AppName)
        .all(db)
        .await?;

    let mut out = Vec::new();
    for state in states {
        if state.app_name.starts_with(KUBARR_PREFIX) || busy.contains(&state.app_name) {
            continue;
        }
        let Some(app) = catalog.get_app(&state.app_name) else {
            continue;
        };
        if EXCLUDED_CATEGORIES.contains(&app.category.as_str()) {
            continue;
        }
        let (Some(installed), Some(available)) = (
            state.installed_chart_version.as_deref(),
            state.available_chart_version.as_deref(),
        ) else {
            continue;
        };
        if !auto_eligible(update_kind(installed, available)) {
            continue;
        }
        if failed_on_version(db, &state.app_name, available).await? {
            continue;
        }
        out.push(state.app_name);
    }
    Ok(out)
}

/// True when the newest update operation for the app, whoever started it,
/// failed while targeting `version` after Helm ran. Only a newer operation (a
/// success) or a different available version clears it; a failed manual retry
/// keeps it. Only failures that reached Helm count: the worker stores
/// diagnostics for exactly those. Failures before Helm (a blip, a dropped stale
/// target) and interrupted operations leave none, so the next night re-evaluates.
async fn failed_on_version<C: ConnectionTrait>(
    db: &C,
    app_name: &str,
    version: &str,
) -> Result<bool> {
    let latest = app_operation::Entity::find()
        .filter(app_operation::Column::AppName.eq(app_name))
        .filter(app_operation::Column::Operation.eq(OP_UPDATE))
        .order_by_desc(app_operation::Column::CreatedAt)
        .one(db)
        .await?;
    Ok(latest.is_some_and(|op| {
        let target = op
            .custom_config
            .as_deref()
            .and_then(|raw| {
                serde_json::from_str::<std::collections::HashMap<String, String>>(raw).ok()
            })
            .and_then(|mut config| config.remove(AUTO_TARGET_KEY));
        op.status == STATUS_FAILED && target.as_deref() == Some(version) && op.diagnostics.is_some()
    }))
}

/// Run one pass of the loop: enqueue at most one update.
pub async fn tick<C: ConnectionTrait>(
    manager: &AppManager,
    db: &C,
    catalog: &SharedCatalog,
    now: DateTime<Utc>,
) -> Result<TickResult> {
    let settings = auto_update_settings::load(db).await?;
    if !settings.enabled {
        return Ok(TickResult::Disabled);
    }
    let in_window = settings
        .window()
        .is_some_and(|window| window_bounds(&window, now).is_some());
    if !in_window {
        return Ok(TickResult::OutsideWindow);
    }
    let auto_in_flight = app_operation::Entity::find()
        .filter(app_operation::Column::Trigger.eq("auto"))
        .filter(app_operation::Column::Status.is_in([STATUS_QUEUED, STATUS_RUNNING]))
        .count(db)
        .await?;
    if auto_in_flight > 0 {
        return Ok(TickResult::Busy);
    }
    let picks = candidates(db, &*catalog.read().await).await?;
    for app_name in picks {
        let target = app_state::Entity::find_by_id(app_name.clone())
            .one(db)
            .await?
            .and_then(|s| s.available_chart_version);
        let Some(target) = target else { continue };
        manager.enqueue_auto_update(&app_name, &target).await?;
        tracing::info!("Auto-update queued for {app_name} -> {target}");
        return Ok(TickResult::Enqueued(app_name));
    }
    Ok(TickResult::Nothing)
}

/// Tick every five minutes until cancelled.
pub async fn run<C: ConnectionTrait>(
    manager: std::sync::Arc<AppManager>,
    db: C,
    catalog: SharedCatalog,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(TICK_INTERVAL) => {}
        }
        if let Err(e) = tick(&manager, &db, &catalog, Utc::now()).await {
            tracing::warn!("Auto-update tick failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::app_state;
    use crate::services::app_manager::{AUTO_SKIPPED_MESSAGE, STATUS_SUCCEEDED};
    use crate::services::auto_update_settings::AutoUpdateSettings;
    use sea_orm::{ActiveModelTrait, Database, DatabaseConnection, Set};
    use sea_orm_migration::MigratorTrait;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn app(name: &str, category: &str) -> crate::services::catalog::AppConfig {
        serde_json::from_value(serde_json::json!({
            "name": name, "display_name": name, "description": "Fixture",
            "icon": "", "container_image": "unused", "default_port": 8080,
            "resource_requirements": {"cpu_request": "100m", "cpu_limit": "1",
                "memory_request": "128Mi", "memory_limit": "256Mi"},
            "volumes": [], "environment_variables": {}, "category": category,
            "is_system": false, "is_hidden": false, "is_browseable": true
        }))
        .unwrap()
    }

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// Inside the default 04:00 UTC, 120 minute window.
    fn in_window() -> DateTime<Utc> {
        utc("2026-11-25T04:30:00Z")
    }

    struct Fixture {
        manager: AppManager,
        catalog: SharedCatalog,
    }

    impl Fixture {
        fn db(&self) -> &DatabaseConnection {
            &self.manager.db
        }
        async fn tick(&self, now: DateTime<Utc>) -> TickResult {
            tick(&self.manager, self.db(), &self.catalog, now)
                .await
                .unwrap()
        }
        async fn candidates(&self) -> Vec<String> {
            candidates(self.db(), &*self.catalog.read().await)
                .await
                .unwrap()
        }
        async fn state(&self, name: &str, installed: &str, available: &str) {
            app_state::ActiveModel {
                app_name: Set(name.into()),
                namespace: Set(name.into()),
                desired_state: Set("installed".into()),
                observed_state: Set(OBS_INSTALLED.into()),
                healthy: Set(true),
                message: Set(None),
                installed_chart_version: Set(Some(installed.into())),
                available_chart_version: Set(Some(available.into())),
                update_available: Set(true),
                auto_update: Set(true),
                last_operation_id: Set(None),
                last_checked_at: Set(None),
                updated_at: Set(Utc::now()),
            }
            .insert(self.db())
            .await
            .unwrap();
        }
        async fn patch_state(&self, name: &str, f: impl FnOnce(&mut app_state::ActiveModel)) {
            let row = app_state::Entity::find_by_id(name.to_string())
                .one(self.db())
                .await
                .unwrap()
                .unwrap();
            let mut active: app_state::ActiveModel = row.into();
            f(&mut active);
            active.update(self.db()).await.unwrap();
        }
        /// What the worker leaves behind after an operation ends: installed and healthy.
        async fn settle(&self, name: &str) {
            self.patch_state(name, |s| {
                s.observed_state = Set(OBS_INSTALLED.into());
                s.healthy = Set(true);
            })
            .await;
        }
        async fn ops(&self, name: &str) -> Vec<app_operation::Model> {
            app_operation::Entity::find()
                .filter(app_operation::Column::AppName.eq(name))
                .order_by_asc(app_operation::Column::CreatedAt)
                .all(self.db())
                .await
                .unwrap()
        }
        /// A failure that reached Helm (the worker stores diagnostics for it).
        async fn fail_after_helm(&self, id: &str) {
            let op = app_operation::Entity::find_by_id(id.to_string())
                .one(self.db())
                .await
                .unwrap()
                .unwrap();
            let mut active: app_operation::ActiveModel = op.into();
            active.status = Set(STATUS_FAILED.into());
            active.diagnostics = Set(Some("Not healthy within 10 min".into()));
            active.update(self.db()).await.unwrap();
        }

        async fn set_status(&self, id: &str, status: &str) {
            let op = app_operation::Entity::find_by_id(id.to_string())
                .one(self.db())
                .await
                .unwrap()
                .unwrap();
            let mut active: app_operation::ActiveModel = op.into();
            active.status = Set(status.into());
            active.update(self.db()).await.unwrap();
        }
    }

    async fn fixture() -> Fixture {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        crate::migrations::Migrator::up(&db, None).await.unwrap();
        let apps = HashMap::from([
            ("alpha".to_string(), app("alpha", "media")),
            ("beta".to_string(), app("beta", "media")),
            ("sys".to_string(), app("sys", "system")),
            ("mon".to_string(), app("mon", "monitoring")),
            (
                "kubarr-dashboard".to_string(),
                app("kubarr-dashboard", "media"),
            ),
        ]);
        let catalog: SharedCatalog = Arc::new(RwLock::new(AppCatalog::with_apps(apps)));
        let manager = AppManager::new(db, Arc::new(RwLock::new(None)), catalog.clone());
        Fixture { manager, catalog }
    }

    #[tokio::test]
    async fn test_tick_disabled() {
        let f = fixture().await;
        f.state("alpha", "1.0.0+1", "1.0.1+1").await;
        let off = AutoUpdateSettings {
            enabled: false,
            ..Default::default()
        };
        auto_update_settings::save(f.db(), &off, None)
            .await
            .unwrap();
        assert_eq!(f.tick(in_window()).await, TickResult::Disabled);
        assert!(f.ops("alpha").await.is_empty());
    }

    #[tokio::test]
    async fn test_tick_outside_window() {
        let f = fixture().await;
        f.state("alpha", "1.0.0+1", "1.0.1+1").await;
        assert_eq!(
            f.tick(utc("2026-11-25T12:00:00Z")).await,
            TickResult::OutsideWindow
        );
        // Just after the 120 minute window ends.
        assert_eq!(
            f.tick(utc("2026-11-25T06:00:00Z")).await,
            TickResult::OutsideWindow
        );
        assert!(f.ops("alpha").await.is_empty());
    }

    #[tokio::test]
    async fn test_enqueues_one_minor_in_window() {
        let f = fixture().await;
        f.state("alpha", "1.0.0+1", "1.1.0+1").await;
        f.state("beta", "1.0.0+1", "1.0.1+1").await;
        assert_eq!(
            f.tick(in_window()).await,
            TickResult::Enqueued("alpha".into())
        );
        let ops = f.ops("alpha").await;
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].operation, OP_UPDATE);
        assert_eq!(ops[0].trigger.as_deref(), Some("auto"));
        assert_eq!(ops[0].created_by, None);
        assert_eq!(ops[0].status, STATUS_QUEUED);
        // One at a time: the queued auto operation blocks the next pick.
        assert_eq!(f.tick(in_window()).await, TickResult::Busy);
        assert!(f.ops("beta").await.is_empty());
        // Next starts only after the previous reached a terminal status.
        f.set_status(&ops[0].id, STATUS_SUCCEEDED).await;
        f.patch_state("alpha", |s| s.update_available = Set(false))
            .await;
        assert_eq!(
            f.tick(in_window()).await,
            TickResult::Enqueued("beta".into())
        );
    }

    #[tokio::test]
    async fn test_major_and_platform_not_candidates() {
        let f = fixture().await;
        f.state("alpha", "6.4.4+25", "7.0.3+26").await;
        f.state("sys", "1.0.0+1", "1.0.1+1").await;
        f.state("mon", "1.0.0+1", "1.0.1+1").await;
        f.state("kubarr-dashboard", "1.0.0+1", "1.0.1+1").await;
        f.state("beta", "1.30.1+3", "1.30.1+4").await;
        f.state("ghost", "1.0.0+1", "1.0.1+1").await;
        assert_eq!(f.candidates().await, vec!["beta".to_string()]);
    }

    #[tokio::test]
    async fn test_unhealthy_app_not_candidate() {
        let f = fixture().await;
        f.state("alpha", "1.0.0+1", "1.0.1+1").await;
        f.state("beta", "1.0.0+1", "1.0.1+1").await;
        f.patch_state("alpha", |s| s.healthy = Set(false)).await;
        assert_eq!(f.candidates().await, vec!["beta".to_string()]);
        assert_eq!(
            f.tick(in_window()).await,
            TickResult::Enqueued("beta".into())
        );
    }

    #[tokio::test]
    async fn test_per_app_toggle_respected() {
        let f = fixture().await;
        f.state("alpha", "1.0.0+1", "1.0.1+1").await;
        f.patch_state("alpha", |s| s.auto_update = Set(false)).await;
        assert!(f.candidates().await.is_empty());
        assert_eq!(f.tick(in_window()).await, TickResult::Nothing);
    }

    #[tokio::test]
    async fn test_auto_update_skips_app_with_active_operation() {
        let f = fixture().await;
        f.state("alpha", "1.0.0+1", "1.0.1+1").await;
        // A manual operation (e.g. from before a leader handover) is running.
        f.manager
            .enqueue_operation("alpha", OP_UPDATE, HashMap::new(), Some(1))
            .await
            .unwrap();
        f.patch_state("alpha", |s| {
            s.observed_state = Set(OBS_INSTALLED.into());
            s.healthy = Set(true);
        })
        .await;
        assert!(f.candidates().await.is_empty());
        assert_eq!(f.tick(in_window()).await, TickResult::Nothing);
        assert_eq!(f.ops("alpha").await.len(), 1);
    }

    #[tokio::test]
    async fn test_busy_while_auto_running() {
        let f = fixture().await;
        f.state("alpha", "1.0.0+1", "1.0.1+1").await;
        f.state("beta", "1.0.0+1", "1.0.1+1").await;
        let queued = f
            .manager
            .enqueue_auto_update("alpha", "1.0.1+1")
            .await
            .unwrap();
        f.set_status(&queued.id, STATUS_RUNNING).await;
        assert_eq!(f.tick(in_window()).await, TickResult::Busy);
        assert!(f.ops("beta").await.is_empty());
    }

    #[tokio::test]
    async fn test_failed_version_skipped_until_version_changes() {
        let f = fixture().await;
        f.state("alpha", "1.0.0+1", "1.0.1+1").await;
        assert_eq!(
            f.tick(in_window()).await,
            TickResult::Enqueued("alpha".into())
        );
        let op = f.ops("alpha").await.remove(0);
        f.fail_after_helm(&op.id).await;
        f.settle("alpha").await;
        // Same available version: skipped on every later tick.
        assert!(f.candidates().await.is_empty());
        assert_eq!(f.tick(in_window()).await, TickResult::Nothing);
        // A new available version clears the skip.
        f.patch_state("alpha", |s| {
            s.available_chart_version = Set(Some("1.0.2+1".into()))
        })
        .await;
        assert_eq!(f.candidates().await, vec!["alpha".to_string()]);
    }

    #[tokio::test]
    async fn test_failed_manual_retry_keeps_skip() {
        let f = fixture().await;
        f.state("alpha", "1.0.0+1", "1.0.1+1").await;
        let queued = f
            .manager
            .enqueue_auto_update("alpha", "1.0.1+1")
            .await
            .unwrap();
        f.fail_after_helm(&queued.id).await;
        f.settle("alpha").await;
        assert!(f.candidates().await.is_empty());
        // The owner retries through the real retry path; that fails too.
        let retry = f.manager.retry_operation(&queued.id, 1).await.unwrap();
        f.fail_after_helm(&retry.id).await;
        f.settle("alpha").await;
        assert_eq!(
            f.ops("alpha").await.last().unwrap().trigger.as_deref(),
            Some("manual")
        );
        assert!(f.candidates().await.is_empty());
        assert_eq!(f.tick(in_window()).await, TickResult::Nothing);
        // A plain manual update (records its target) that fails also keeps it.
        let manual = f
            .manager
            .enqueue_operation("alpha", OP_UPDATE, HashMap::new(), Some(1))
            .await
            .unwrap();
        f.fail_after_helm(&manual.id).await;
        f.settle("alpha").await;
        assert!(f.candidates().await.is_empty());
    }

    #[tokio::test]
    async fn test_successful_manual_update_makes_next_version_eligible() {
        let f = fixture().await;
        f.state("alpha", "1.0.0+1", "1.0.1+1").await;
        let queued = f
            .manager
            .enqueue_auto_update("alpha", "1.0.1+1")
            .await
            .unwrap();
        f.fail_after_helm(&queued.id).await;
        f.settle("alpha").await;
        let manual = f
            .manager
            .enqueue_operation("alpha", OP_UPDATE, HashMap::new(), Some(1))
            .await
            .unwrap();
        f.set_status(&manual.id, STATUS_SUCCEEDED).await;
        f.patch_state("alpha", |s| {
            s.installed_chart_version = Set(Some("1.0.1+1".into()));
            s.available_chart_version = Set(Some("1.0.2+1".into()));
        })
        .await;
        f.settle("alpha").await;
        assert_eq!(f.candidates().await, vec!["alpha".to_string()]);
    }

    #[tokio::test]
    async fn test_dropped_stale_auto_op_does_not_count_as_failure() {
        let f = fixture().await;
        f.state("alpha", "1.0.0+1", "1.0.1+1").await;
        let queued = f
            .manager
            .enqueue_auto_update("alpha", "1.0.1+1")
            .await
            .unwrap();
        let op = f.ops("alpha").await.remove(0);
        let mut active: app_operation::ActiveModel = op.into();
        active.status = Set(STATUS_FAILED.into());
        active.error = Set(Some(AUTO_SKIPPED_MESSAGE.into()));
        active.update(f.db()).await.unwrap();
        let _ = queued;
        f.settle("alpha").await;
        assert_eq!(f.candidates().await, vec!["alpha".to_string()]);
    }

    #[tokio::test]
    async fn test_failure_before_helm_or_interruption_does_not_count() {
        let f = fixture().await;
        f.state("alpha", "1.0.0+1", "1.0.1+1").await;
        let queued = f
            .manager
            .enqueue_auto_update("alpha", "1.0.1+1")
            .await
            .unwrap();
        // Failed before Helm (or recovered as interrupted): no diagnostics.
        f.set_status(&queued.id, STATUS_FAILED).await;
        f.settle("alpha").await;
        assert_eq!(f.candidates().await, vec!["alpha".to_string()]);
    }

    #[tokio::test]
    async fn test_auto_target_not_exposed_to_helm_config() {
        let f = fixture().await;
        let err = f
            .manager
            .enqueue_operation(
                "alpha",
                OP_UPDATE,
                HashMap::from([(AUTO_TARGET_KEY.to_string(), "9.9.9".to_string())]),
                None,
            )
            .await;
        assert!(err.is_err());
    }
}
