//! Real migrated SQLite and worker control flow; only external execution is faked.
use super::*;
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use tokio::sync::RwLock;

const APP: &str = "operation-test-app";

async fn manager(db: DatabaseConnection) -> AppManager {
    crate::migrations::Migrator::up(&db, None).await.unwrap();
    let app = serde_json::from_value(serde_json::json!({
        "name": APP, "display_name": "Operation test", "description": "Fixture",
        "icon": "", "container_image": "unused", "default_port": 8080,
        "resource_requirements": {"cpu_request": "100m", "cpu_limit": "1",
            "memory_request": "128Mi", "memory_limit": "256Mi"},
        "volumes": [], "environment_variables": {}, "category": "test",
        "is_system": false, "is_hidden": false, "is_browseable": true
    }))
    .unwrap();
    AppManager::new(
        db,
        Arc::new(RwLock::new(None)),
        Arc::new(RwLock::new(AppCatalog::with_apps(HashMap::from([(
            APP.into(),
            app,
        )])))),
    )
}

async fn setup() -> AppManager {
    manager(Database::connect("sqlite::memory:").await.unwrap()).await
}

async fn operation(manager: &AppManager, id: &str) -> app_operation::Model {
    app_operation::Entity::find_by_id(id)
        .one(&manager.db)
        .await
        .unwrap()
        .unwrap()
}

async fn state(manager: &AppManager) -> app_state::Model {
    app_state::Entity::find_by_id(APP)
        .one(&manager.db)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn worker_success_persists_claim_and_completion_for_each_operation() {
    for kind in [OP_INSTALL, OP_UPDATE, OP_DELETE, OP_RESTART] {
        let manager = setup().await;
        let config = HashMap::from([("setting".into(), "exact value".into())]);
        let queued = manager
            .enqueue_operation(APP, kind, config.clone(), Some(42))
            .await
            .unwrap();
        let before = operation(&manager, &queued.id).await;
        let queued_state = state(&manager).await;
        let mut claimed = None;
        manager
            .process_next_operation_with(|running| {
                claimed = Some(running.clone());
                let manager = &manager;
                let before = &before;
                async move {
                    assert_eq!(operation(manager, &running.id).await, running);
                    assert_eq!(running.status, STATUS_RUNNING);
                    assert_eq!(running.message.as_deref(), Some("Worker started operation"));
                    assert_eq!(running.attempts, 1);
                    assert_eq!(running.created_by, Some(42));
                    assert_eq!(running.created_at, before.created_at);
                    assert_eq!(running.custom_config, before.custom_config);
                    assert_eq!(running.error, None);
                    assert_eq!(running.finished_at, None);
                    assert_eq!(running.started_at, Some(running.updated_at));
                    assert!(running.updated_at >= before.updated_at);
                    Ok("Executed successfully".into())
                }
            })
            .await
            .unwrap();
        let claimed = claimed.unwrap();
        let finished = operation(&manager, &queued.id).await;
        let mut expected = claimed.clone();
        expected.status = STATUS_SUCCEEDED.into();
        expected.message = Some("Executed successfully".into());
        expected.finished_at = finished.finished_at;
        expected.updated_at = finished.updated_at;
        assert_eq!(finished, expected);
        assert_eq!(finished.finished_at, Some(finished.updated_at));
        assert!(finished.updated_at >= claimed.updated_at);
        assert!(finished.updated_at <= Utc::now());
        assert_eq!(
            serde_json::from_str::<HashMap<String, String>>(
                finished.custom_config.as_deref().unwrap()
            )
            .unwrap(),
            config
        );

        let current = state(&manager).await;
        assert_eq!(
            current.last_operation_id.as_deref(),
            Some(queued.id.as_str())
        );
        assert_eq!(current.desired_state, desired_state_for_operation(kind));
        assert_eq!(current.namespace, APP);
        assert!(!current.healthy);
        assert_eq!(current.last_checked_at, Some(current.updated_at));
        if kind == OP_RESTART {
            // Success is not a health check. With no client, reconciliation is a no-op.
            assert_eq!(current, queued_state);
        } else {
            assert_eq!(
                current.observed_state,
                if kind == OP_DELETE {
                    OBS_NOT_INSTALLED
                } else {
                    OBS_INSTALLING
                }
            );
            assert_eq!(
                current.message,
                if kind == OP_DELETE {
                    Some("Removed".into())
                } else {
                    finished.message.clone()
                }
            );
            assert!(current.updated_at >= claimed.updated_at);
            assert!(current.updated_at <= finished.updated_at);
        }
        manager
            .process_next_operation_with(|_| async { panic!("terminal operation was re-executed") })
            .await
            .unwrap();
        assert_eq!(operation(&manager, &queued.id).await, finished);
        assert_eq!(state(&manager).await, current);
    }
}

#[tokio::test]
async fn worker_failure_persists_exact_error_and_state_for_each_operation() {
    for kind in [OP_INSTALL, OP_UPDATE, OP_DELETE, OP_RESTART] {
        let manager = setup().await;
        let queued = manager
            .enqueue_operation(APP, kind, HashMap::new(), Some(17))
            .await
            .unwrap();
        let mut claimed = None;
        let error = AppError::BadRequest("fake execution rejected".into()).to_string();
        manager
            .process_next_operation_with(|running| {
                claimed = Some(running.clone());
                let manager = &manager;
                async move {
                    assert_eq!(operation(manager, &running.id).await, running);
                    assert_eq!(running.status, STATUS_RUNNING);
                    Err(AppError::BadRequest("fake execution rejected".into()))
                }
            })
            .await
            .unwrap();
        let claimed = claimed.unwrap();
        let finished = operation(&manager, &queued.id).await;
        let mut expected = claimed.clone();
        expected.status = STATUS_FAILED.into();
        expected.message = Some(format!("{kind} failed"));
        expected.error = Some(error.clone());
        expected.finished_at = finished.finished_at;
        expected.updated_at = finished.updated_at;
        assert_eq!(finished, expected);
        assert_eq!(finished.attempts, 1);
        assert_eq!(finished.created_by, Some(17));
        assert_eq!(finished.created_at, queued.created_at);
        assert_eq!(finished.started_at, Some(claimed.updated_at));
        assert_eq!(finished.finished_at, Some(finished.updated_at));
        assert!(finished.updated_at >= claimed.updated_at);
        let current = state(&manager).await;
        assert_eq!(current.desired_state, desired_state_for_operation(kind));
        assert_eq!(current.observed_state, OBS_FAILED);
        assert_eq!(current.message, Some(error));
        assert_eq!(current.last_operation_id, Some(queued.id.clone()));
        assert_eq!(current.namespace, APP);
        assert!(!current.healthy);
        assert_eq!(current.last_checked_at, Some(current.updated_at));
        assert!(current.updated_at >= finished.updated_at);
        assert!(current.updated_at <= Utc::now());
        manager
            .process_next_operation_with(|_| async { panic!("failed operation was retried") })
            .await
            .unwrap();
        assert_eq!(operation(&manager, &queued.id).await, finished);
    }
}

#[tokio::test]
async fn old_success_or_failure_preserves_newer_queued_intent() {
    for succeeds in [true, false] {
        for (old_kind, new_kind) in [(OP_INSTALL, OP_DELETE), (OP_DELETE, OP_INSTALL)] {
            let manager = setup().await;
            let old = manager
                .enqueue_operation(APP, old_kind, HashMap::new(), None)
                .await
                .unwrap();
            let mut newer = None;
            manager
                .process_next_operation_with(|running| {
                    let newer = &mut newer;
                    let manager = &manager;
                    let old_id = &old.id;
                    async move {
                        assert_eq!(&running.id, old_id);
                        manager
                            .enqueue_operation(APP, new_kind, HashMap::new(), Some(99))
                            .await
                            .unwrap();
                        *newer = Some(state(manager).await);
                        if succeeds {
                            Ok("Old execution completed".into())
                        } else {
                            Err(AppError::Internal("Old execution failed".into()))
                        }
                    }
                })
                .await
                .unwrap();
            let newer = newer.unwrap();
            assert_eq!(state(&manager).await, newer);
            assert_eq!(newer.desired_state, desired_state_for_operation(new_kind));
            let pending = operation(&manager, newer.last_operation_id.as_deref().unwrap()).await;
            assert_eq!(pending.status, STATUS_QUEUED);
            assert_eq!(pending.attempts, 0);
            assert_eq!(pending.started_at, None);
            assert_eq!(pending.finished_at, None);
            assert_eq!(pending.created_by, Some(99));
            assert_eq!(
                operation(&manager, &old.id).await.status,
                if succeeds {
                    STATUS_SUCCEEDED
                } else {
                    STATUS_FAILED
                }
            );
        }
    }
}

#[tokio::test]
async fn sqlite_competing_claims_of_same_candidate_have_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("claims.db").display()
    );
    let first = manager(Database::connect(&url).await.unwrap()).await;
    let second = AppManager::new(
        Database::connect(&url).await.unwrap(),
        first.k8s_client.clone(),
        first.catalog.clone(),
    );
    let queued = first
        .enqueue_operation(APP, OP_INSTALL, HashMap::new(), None)
        .await
        .unwrap();
    // Force the problematic interleaving: both workers have selected the same queued row.
    let a = operation(&first, &queued.id).await;
    let b = operation(&second, &queued.id).await;
    let (a, b) = tokio::join!(first.claim_operation(a), second.claim_operation(b));
    let winners: Vec<_> = [a.unwrap(), b.unwrap()].into_iter().flatten().collect();
    assert_eq!(winners.len(), 1);
    let running = operation(&first, &queued.id).await;
    assert_eq!(running, winners[0]);
    assert_eq!(running.status, STATUS_RUNNING);
    assert_eq!(running.attempts, 1);
    assert_eq!(running.started_at, Some(running.updated_at));
    assert_eq!(running.finished_at, None);
    assert!(first.claim_next_operation().await.unwrap().is_none());
    assert!(second.claim_next_operation().await.unwrap().is_none());
}

#[tokio::test]
async fn competing_workers_execute_only_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let manager = setup().await;
    let queued = manager
        .enqueue_operation(APP, OP_DELETE, HashMap::new(), None)
        .await
        .unwrap();
    let calls = AtomicUsize::new(0);
    let execute = |_| async {
        calls.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        Ok("Removed".into())
    };
    let (a, b) = tokio::join!(
        manager.process_next_operation_with(execute),
        manager.process_next_operation_with(execute)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let finished = operation(&manager, &queued.id).await;
    assert_eq!(finished.status, STATUS_SUCCEEDED);
    assert_eq!(finished.attempts, 1);
}

#[tokio::test]
async fn worker_claims_oldest_queued_operation_first() {
    let manager = setup().await;
    let older = manager
        .enqueue_operation(APP, OP_INSTALL, HashMap::new(), None)
        .await
        .unwrap();
    let newer = manager
        .enqueue_operation(APP, OP_DELETE, HashMap::new(), None)
        .await
        .unwrap();
    let mut active: app_operation::ActiveModel = operation(&manager, &older.id).await.into();
    active.created_at = Set(newer.created_at - chrono::Duration::seconds(1));
    active.update(&manager.db).await.unwrap();
    manager
        .process_next_operation_with(|running| async move {
            assert_eq!(running.id, older.id);
            Ok("Installed".into())
        })
        .await
        .unwrap();
    assert_eq!(operation(&manager, &newer.id).await.status, STATUS_QUEUED);
    assert_eq!(state(&manager).await.last_operation_id, Some(newer.id));
}

#[tokio::test]
async fn invalid_operation_and_missing_catalog_app_leave_no_rows() {
    let manager = setup().await;
    for kind in ["", "upgrade", "INSTALL"] {
        assert!(matches!(
            manager
                .enqueue_operation(APP, kind, HashMap::new(), None)
                .await,
            Err(AppError::BadRequest(_))
        ));
    }
    for kind in [OP_INSTALL, OP_UPDATE, OP_RESTART] {
        assert!(matches!(
            manager
                .enqueue_operation("missing", kind, HashMap::new(), None)
                .await,
            Err(AppError::NotFound(_))
        ));
    }
    assert!(app_operation::Entity::find()
        .all(&manager.db)
        .await
        .unwrap()
        .is_empty());
    assert!(app_state::Entity::find()
        .all(&manager.db)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn reconciliation_state_write_preserves_requested_intent() {
    let manager = setup().await;
    for (kind, observed_desired, observed) in [
        (OP_INSTALL, DESIRED_REMOVED, OBS_NOT_INSTALLED),
        (OP_DELETE, DESIRED_INSTALLED, OBS_INSTALLED),
    ] {
        let queued = manager
            .enqueue_operation(APP, kind, HashMap::new(), None)
            .await
            .unwrap();
        // Test the persistence used by reconciliation, not a fake health check.
        manager
            .upsert_state(
                APP,
                APP,
                observed_desired,
                observed,
                false,
                Some("Observed".into()),
                None,
                false,
            )
            .await
            .unwrap();
        let current = state(&manager).await;
        assert_eq!(current.desired_state, desired_state_for_operation(kind));
        assert_eq!(current.last_operation_id, Some(queued.id));
        assert_eq!(current.observed_state, observed);
        assert_eq!(current.message.as_deref(), Some("Observed"));
    }
}
