//! Tests that exercise the database code paths in `services/bootstrap.rs`
//!
//! The existing `bootstrap_service_tests.rs` only exercises the in-memory
//! fallback paths (DB empty → falls back to in-memory status). This file
//! covers the DB paths by pre-inserting bootstrap_status records.

mod common;
use common::create_test_db_with_seed;

use kubarr::models::{bootstrap_status, prelude::BootstrapStatus};
use kubarr::services::{
    bootstrap::{BootstrapService, BOOTSTRAP_COMPONENTS},
    catalog::AppCatalog,
    k8s::K8sClient,
};

use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

// ============================================================================
// Helper: build BootstrapService with a DB that has records
// ============================================================================

async fn make_service_with_db_records(
    statuses: &[(&str, &str, &str, Option<&str>)], // (component, display_name, status, error)
) -> BootstrapService {
    let db = create_test_db_with_seed().await;

    // Insert bootstrap_status records
    for (component, display_name, status, error) in statuses {
        let record = bootstrap_status::ActiveModel {
            component: Set(component.to_string()),
            display_name: Set(display_name.to_string()),
            status: Set(status.to_string()),
            message: Set(Some(format!("{} message", status))),
            error: Set(error.map(|e| e.to_string())),
            started_at: Set(None),
            completed_at: Set(None),
            ..Default::default()
        };
        record.insert(&db).await.expect("insert bootstrap_status");
    }

    let shared_db = Arc::new(RwLock::new(Some(db)));
    let k8s: Arc<RwLock<Option<K8sClient>>> = Arc::new(RwLock::new(None));
    let catalog = Arc::new(RwLock::new(AppCatalog::default()));
    let (tx, _rx) = broadcast::channel(100);
    BootstrapService::new(shared_db, k8s, catalog, tx)
}

// ============================================================================
// get_status() — DB path (when records exist)
// ============================================================================

#[tokio::test]
async fn test_get_status_uses_db_when_records_exist() {
    let svc = make_service_with_db_records(&[
        ("postgresql", "PostgreSQL", "pending", None),
        ("victoriametrics", "VictoriaMetrics", "pending", None),
        ("victorialogs", "VictoriaLogs", "pending", None),
        ("fluent-bit", "Fluent Bit", "pending", None),
    ])
    .await;

    let statuses = svc.get_status().await.expect("get_status must succeed");

    // DB has records → must return those records (not in-memory fallback)
    assert_eq!(statuses.len(), 4, "must return all 4 components from DB");
    for s in &statuses {
        assert_eq!(s.status, "pending", "all must be pending");
    }
}

#[tokio::test]
async fn test_get_status_db_returns_correct_fields() {
    let svc = make_service_with_db_records(&[
        ("postgresql", "PostgreSQL", "healthy", None),
        ("victoriametrics", "VictoriaMetrics", "installing", None),
        ("victorialogs", "VictoriaLogs", "failed", Some("helm error")),
        ("fluent-bit", "Fluent Bit", "pending", None),
    ])
    .await;

    let statuses = svc.get_status().await.expect("get_status must succeed");
    assert_eq!(statuses.len(), 4);

    let pg = statuses
        .iter()
        .find(|s| s.component == "postgresql")
        .unwrap();
    assert_eq!(pg.status, "healthy");
    assert_eq!(pg.display_name, "PostgreSQL");

    let vm = statuses
        .iter()
        .find(|s| s.component == "victoriametrics")
        .unwrap();
    assert_eq!(vm.status, "installing");

    let vl = statuses
        .iter()
        .find(|s| s.component == "victorialogs")
        .unwrap();
    assert_eq!(vl.status, "failed");
    assert_eq!(vl.error, Some("helm error".to_string()));
}

// ============================================================================
// is_complete() — various DB states
// ============================================================================

#[tokio::test]
async fn test_is_complete_false_when_db_has_non_healthy_statuses() {
    let svc = make_service_with_db_records(&[
        ("postgresql", "PostgreSQL", "healthy", None),
        ("victoriametrics", "VictoriaMetrics", "installing", None),
        ("victorialogs", "VictoriaLogs", "pending", None),
        ("fluent-bit", "Fluent Bit", "pending", None),
    ])
    .await;

    assert!(
        !svc.is_complete().await,
        "must not be complete when some are not healthy"
    );
}

#[tokio::test]
async fn test_is_complete_true_when_all_healthy() {
    let svc = make_service_with_db_records(&[
        ("postgresql", "PostgreSQL", "healthy", None),
        ("victoriametrics", "VictoriaMetrics", "healthy", None),
        ("victorialogs", "VictoriaLogs", "healthy", None),
        ("fluent-bit", "Fluent Bit", "healthy", None),
    ])
    .await;

    assert!(
        svc.is_complete().await,
        "must be complete when all are healthy"
    );
}

// ============================================================================
// has_started() — DB path
// ============================================================================

#[tokio::test]
async fn test_has_started_true_when_db_has_non_pending_status() {
    let svc = make_service_with_db_records(&[
        ("postgresql", "PostgreSQL", "installing", None),
        ("victoriametrics", "VictoriaMetrics", "pending", None),
        ("victorialogs", "VictoriaLogs", "pending", None),
        ("fluent-bit", "Fluent Bit", "pending", None),
    ])
    .await;

    assert!(
        svc.has_started().await,
        "must have started when postgresql is installing"
    );
}

#[tokio::test]
async fn test_has_started_false_when_all_pending_in_db() {
    let svc = make_service_with_db_records(&[
        ("postgresql", "PostgreSQL", "pending", None),
        ("victoriametrics", "VictoriaMetrics", "pending", None),
        ("victorialogs", "VictoriaLogs", "pending", None),
        ("fluent-bit", "Fluent Bit", "pending", None),
    ])
    .await;

    // All pending → has_started checks if any != "pending"
    assert!(
        !svc.has_started().await,
        "must not have started when all are pending"
    );
}

#[tokio::test]
async fn test_has_started_true_when_completed() {
    let svc = make_service_with_db_records(&[
        ("postgresql", "PostgreSQL", "healthy", None),
        ("victoriametrics", "VictoriaMetrics", "healthy", None),
        ("victorialogs", "VictoriaLogs", "healthy", None),
        ("fluent-bit", "Fluent Bit", "healthy", None),
    ])
    .await;

    assert!(
        svc.has_started().await,
        "must have started when all healthy"
    );
}

// ============================================================================
// Test with empty DB (in-memory fallback — already covered but ensures path)
// ============================================================================

#[tokio::test]
async fn test_get_status_falls_back_to_in_memory_when_db_empty() {
    let db = create_test_db_with_seed().await;
    // DB has no bootstrap_status records
    let count = BootstrapStatus::find().all(&db).await.unwrap().len();
    assert_eq!(count, 0, "DB must be empty before this test");

    let shared_db = Arc::new(RwLock::new(Some(db)));
    let k8s: Arc<RwLock<Option<K8sClient>>> = Arc::new(RwLock::new(None));
    let catalog = Arc::new(RwLock::new(AppCatalog::default()));
    let (tx, _rx) = broadcast::channel(100);
    let svc = BootstrapService::new(shared_db, k8s, catalog, tx);

    let statuses = svc.get_status().await.expect("get_status must succeed");
    // In-memory fallback: 4 components, all pending
    assert_eq!(statuses.len(), BOOTSTRAP_COMPONENTS.len());
    for s in &statuses {
        assert_eq!(s.status, "pending");
    }
}

// ============================================================================
// Mixed DB + in-memory path: just 1 record in DB
// ============================================================================

#[tokio::test]
async fn test_get_status_with_partial_db_uses_db_if_nonempty() {
    // Even with just 1 record, the DB path returns DB results (not in-memory)
    let svc = make_service_with_db_records(&[("postgresql", "PostgreSQL", "healthy", None)]).await;

    let statuses = svc.get_status().await.expect("get_status must succeed");
    // DB has 1 record → returns that 1 record (DB takes priority over in-memory)
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].component, "postgresql");
    assert_eq!(statuses[0].status, "healthy");
}

// ============================================================================
// retry_component — exercises update_in_memory_status, update_db_status,
// install_component and the helm-error path
// ============================================================================

#[tokio::test]
async fn test_retry_component_unknown_returns_not_found() {
    // No DB records needed — the component lookup fails before touching the DB
    let svc = make_service_with_db_records(&[]).await;

    let result = svc.retry_component("totally-unknown-component").await;
    assert!(result.is_err(), "unknown component must return error");
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.to_lowercase().contains("unknown") || err_str.to_lowercase().contains("not found"),
        "unexpected error: {}",
        err_str
    );
}

#[tokio::test]
async fn test_retry_component_victoriametrics_fails_at_helm() {
    // Create a DB with a victoriametrics record so update_db_status actually updates it.
    let svc = make_service_with_db_records(&[(
        "victoriametrics",
        "VictoriaMetrics",
        "failed",
        Some("previous error"),
    )])
    .await;

    // retry_component triggers:
    //   update_in_memory_status → update_db_status → install_component
    // install_component runs helm, which fails (chart not present in test env).
    let result = svc.retry_component("victoriametrics").await;

    // Expect an error because helm is not available or the chart doesn't exist.
    assert!(
        result.is_err(),
        "retry_component must fail without real helm/chart"
    );
}

#[tokio::test]
async fn test_retry_component_postgresql_fails_at_helm() {
    // Same test for the postgresql component which takes a slightly different code
    // path (install_postgresql is called instead of install_component directly).
    let svc = make_service_with_db_records(&[(
        "postgresql",
        "PostgreSQL",
        "failed",
        Some("previous error"),
    )])
    .await;

    let result = svc.retry_component("postgresql").await;

    assert!(
        result.is_err(),
        "retry_component(postgresql) must fail without real helm/chart"
    );
}

#[tokio::test]
async fn test_retry_component_victoriametrics_with_all_records() {
    // Ensure update_db_status covers the branch where all 4 records exist.
    let svc = make_service_with_db_records(&[
        ("postgresql", "PostgreSQL", "healthy", None),
        ("victoriametrics", "VictoriaMetrics", "failed", Some("err")),
        ("victorialogs", "VictoriaLogs", "pending", None),
        ("fluent-bit", "Fluent Bit", "pending", None),
    ])
    .await;

    let result = svc.retry_component("victoriametrics").await;
    assert!(result.is_err(), "must fail without helm");
}

#[tokio::test]
async fn test_retry_component_fluent_bit_with_db_record() {
    // "fluent-bit" exercises the install_component path (not postgresql).
    // The record already has status="failed" so update_db_status has an existing row.
    let svc = make_service_with_db_records(&[("fluent-bit", "Fluent Bit", "failed", None)]).await;

    let result = svc.retry_component("fluent-bit").await;
    assert!(result.is_err(), "must fail without helm");
}
