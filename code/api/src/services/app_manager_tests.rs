//! Real migrated SQLite and worker control flow; only external execution is faked.
use super::*;
use crate::models::{audit_log, audit_outbox};
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Notify, RwLock};

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

async fn audits(manager: &AppManager) -> Vec<audit_log::Model> {
    audit_log::Entity::find()
        .order_by_asc(audit_log::Column::Id)
        .all(&manager.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn controls_compete_with_claim_and_only_queued_work_executes() {
    let manager = setup().await;
    let queued = manager
        .enqueue_operation(APP, OP_RESTART, HashMap::new(), None)
        .await
        .unwrap();
    let (claim, pause) = tokio::join!(
        manager.claim_operation(operation(&manager, &queued.id).await),
        manager.control_operation(&queued.id, "pause", 1)
    );
    assert_ne!(claim.unwrap().is_some(), pause.is_ok());
    if pause.is_ok() {
        assert!(manager.claim_next_operation().await.unwrap().is_none());
        manager
            .control_operation(&queued.id, "resume", 1)
            .await
            .unwrap();
        assert!(manager.claim_next_operation().await.unwrap().is_some());
    } else {
        assert!(
            manager
                .control_operation(&queued.id, "cancel", 1)
                .await
                .unwrap()
                .stop_requested
        );
        assert!(manager
            .control_operation(&queued.id, "pause", 1)
            .await
            .is_err());
    }
    let second = manager
        .enqueue_operation(APP, OP_UPDATE, HashMap::new(), None)
        .await
        .unwrap();
    manager
        .control_operation(&second.id, "pause", 1)
        .await
        .unwrap();
    manager
        .control_operation(&second.id, "cancel", 1)
        .await
        .unwrap();
    assert_eq!(
        operation(&manager, &second.id).await.status,
        STATUS_CANCELLED
    );
    assert!(manager
        .claim_operation(operation(&manager, &second.id).await)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn running_stop_wins_completion_and_cannot_be_retried() {
    let manager = Arc::new(setup().await);
    let queued = manager
        .enqueue_operation(APP, OP_INSTALL, HashMap::new(), None)
        .await
        .unwrap();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let task = tokio::spawn({
        let manager = manager.clone();
        let started = started.clone();
        let release = release.clone();
        async move {
            manager
                .process_next_operation_with(move |_| async move {
                    started.notify_one();
                    release.notified().await;
                    Ok("Helm returned success".into())
                })
                .await
        }
    });
    started.notified().await;
    let requested = manager
        .control_operation(&queued.id, "cancel", 7)
        .await
        .unwrap();
    assert_eq!(requested.status, STATUS_RUNNING);
    assert_eq!(requested.message.as_deref(), Some("Stop requested"));
    assert!(requested.stop_requested);
    assert!(requested.finished_at.is_none());
    assert!(manager
        .control_operation(&queued.id, "cancel", 7)
        .await
        .is_err());
    release.notify_one();
    task.await.unwrap().unwrap();
    let stopped = operation(&manager, &queued.id).await;
    assert_eq!(stopped.status, STATUS_CANCELLED);
    assert!(stopped.error.unwrap().contains("indeterminate"));
    assert!(manager.retry_operation(&queued.id, 7).await.is_err());
    assert_ne!(
        state(&manager).await.message.as_deref(),
        Some("Helm returned success")
    );
    assert_eq!(
        audit_outbox::Entity::find_by_id(&queued.id)
            .one(&manager.db)
            .await
            .unwrap()
            .unwrap()
            .outcome,
        "indeterminate"
    );
}

#[tokio::test]
async fn stale_requested_stop_is_terminal_without_replay() {
    let manager = setup().await;
    let queued = manager
        .enqueue_operation(APP, OP_DELETE, HashMap::new(), None)
        .await
        .unwrap();
    manager.claim_next_operation().await.unwrap().unwrap();
    manager
        .control_operation(&queued.id, "cancel", 1)
        .await
        .unwrap();
    let mut stale: app_operation::ActiveModel = operation(&manager, &queued.id).await.into();
    stale.updated_at = Set(Utc::now() - chrono::Duration::minutes(16));
    stale.update(&manager.db).await.unwrap();
    manager.recover_stale_operations().await.unwrap();
    assert_eq!(
        operation(&manager, &queued.id).await.status,
        STATUS_CANCELLED
    );
    assert!(manager.claim_next_operation().await.unwrap().is_none());
    assert!(manager.retry_operation(&queued.id, 1).await.is_err());
}

#[tokio::test]
async fn stale_recovery_snapshot_cannot_erase_a_new_stop_request() {
    let manager = setup().await;
    let queued = manager
        .enqueue_operation(APP, OP_DELETE, HashMap::new(), None)
        .await
        .unwrap();
    manager.claim_next_operation().await.unwrap().unwrap();
    let cutoff = Utc::now() - STALE_RUNNING_AFTER;
    let mut stale: app_operation::ActiveModel = operation(&manager, &queued.id).await.into();
    stale.updated_at = Set(cutoff - chrono::Duration::seconds(1));
    stale.update(&manager.db).await.unwrap();
    let snapshot = operation(&manager, &queued.id).await;
    manager
        .control_operation(&queued.id, "cancel", 1)
        .await
        .unwrap();
    manager
        .recover_stale_operation(snapshot, cutoff)
        .await
        .unwrap();
    let current = operation(&manager, &queued.id).await;
    assert_eq!(current.status, STATUS_RUNNING);
    assert!(current.stop_requested);
    assert!(current.finished_at.is_none());
    assert!(audit_outbox::Entity::find_by_id(&queued.id)
        .one(&manager.db)
        .await
        .unwrap()
        .is_none());
}

#[test]
fn corrupt_queued_configuration_is_rejected_instead_of_resetting_chart_values() {
    assert!(parse_queued_config(Some("not json")).is_err());
    assert!(parse_queued_config(Some(r#"{"gpu.enabled":true}"#)).is_err());
    assert_eq!(parse_queued_config(Some("{}")).unwrap(), HashMap::new());
}

#[tokio::test]
async fn retry_is_single_use_and_preserves_gpu_queue_input_and_new_actor() {
    crate::services::gpu::test_charts::set_gpu_chart("plex");
    let manager = setup().await;
    let mut plex = manager.catalog.read().await.get_app(APP).unwrap().clone();
    plex.name = "plex".into();
    *manager.catalog.write().await = AppCatalog::with_apps(HashMap::from([("plex".into(), plex)]));
    let config = HashMap::from([(QUEUED_GPU_KEY.into(), "null".into())]);
    let failed = manager
        .enqueue_operation_unchecked("plex", OP_UPDATE, config, None)
        .await
        .unwrap();
    manager
        .claim_operation(operation(&manager, &failed.id).await)
        .await
        .unwrap();
    manager
        .finish_operation(&failed.id, STATUS_FAILED, None, Some("error".into()))
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        manager.retry_operation(&failed.id, 3),
        manager.retry_operation(&failed.id, 4)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let new = a.or(b).unwrap();
    assert_eq!(new.status, STATUS_QUEUED);
    assert!(new.created_by == Some(3) || new.created_by == Some(4));
    assert_eq!(
        operation(&manager, &new.id).await.custom_config,
        operation(&manager, &failed.id).await.custom_config
    );
    assert_eq!(operation(&manager, &failed.id).await.status, STATUS_RETRIED);
    assert_eq!(
        app_state::Entity::find_by_id("plex")
            .one(&manager.db)
            .await
            .unwrap()
            .unwrap()
            .last_operation_id
            .as_deref(),
        Some(new.id.as_str())
    );
}

#[tokio::test]
async fn gpu_disable_survives_queue_and_managed_override_cannot_be_enqueued() {
    crate::services::gpu::test_charts::set_gpu_chart("plex");
    let manager = setup().await;
    let mut plex = manager.catalog.read().await.get_app(APP).unwrap().clone();
    plex.name = "plex".into();
    *manager.catalog.write().await = AppCatalog::with_apps(HashMap::from([("plex".into(), plex)]));
    let queued = manager
        .enqueue_gpu_operation("plex", OP_UPDATE, HashMap::new(), Some(None), None)
        .await
        .unwrap();
    let config: HashMap<String, String> = serde_json::from_str(
        operation(&manager, &queued.id)
            .await
            .custom_config
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(config.get(QUEUED_GPU_KEY).map(String::as_str), Some("null"));
    for config in [
        HashMap::from([(QUEUED_GPU_KEY.into(), "null".into())]),
        HashMap::from([("gpu.enabled".into(), "true".into())]),
        HashMap::from([("image.tag".into(), "stable,gpu.enabled=true".into())]),
    ] {
        assert!(manager
            .enqueue_operation("plex", OP_INSTALL, config, None)
            .await
            .is_err());
    }
    assert_eq!(
        app_operation::Entity::find()
            .all(&manager.db)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn audit_enqueue_and_terminal_correlate_without_exposing_config_or_errors() {
    for (kind, success_action, failure_action) in [
        (OP_INSTALL, "app_installed", "app_installed"),
        (OP_UPDATE, "app_updated", "app_update_failed"),
        (OP_DELETE, "app_uninstalled", "app_uninstalled"),
        (OP_RESTART, "app_restarted", "app_restarted"),
    ] {
        for succeeds in [true, false] {
            let action = if succeeds {
                success_action
            } else {
                failure_action
            };
            let manager = setup().await;
            let queued = manager
                .enqueue_operation(
                    APP,
                    kind,
                    HashMap::from([("password".into(), "secret-123".into())]),
                    Some(4242),
                )
                .await
                .unwrap();
            let logs = audits(&manager).await;
            assert_eq!(logs.len(), 1);
            assert_eq!(logs[0].action, "app_configured");
            assert_eq!(logs[0].user_id, Some(4242));
            assert_eq!(logs[0].username, None); // deleted user retains its ID
            assert_eq!(logs[0].resource_id.as_deref(), Some(APP));
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(logs[0].details.as_ref().unwrap())
                    .unwrap(),
                serde_json::json!({"operation_id":queued.id,"operation":kind,"phase":"queued"})
            );
            manager
                .process_next_operation_with(|_| async move {
                    if succeeds {
                        Ok("done".into())
                    } else {
                        Err(AppError::Internal(
                            "secret-123 helm --set password=secret-123".into(),
                        ))
                    }
                })
                .await
                .unwrap();
            let logs = audits(&manager).await;
            assert_eq!(logs.len(), 2);
            assert_eq!(logs[1].action, action);
            assert_eq!(logs[1].user_id, Some(4242));
            assert_eq!(logs[1].success, succeeds);
            assert_eq!(
                logs[1].error_message.as_deref(),
                (!succeeds).then_some("app_operation_failed")
            );
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(logs[1].details.as_ref().unwrap())
                    .unwrap(),
                serde_json::json!({"operation_id":queued.id,"attempts":1,"phase":"completed"})
            );
            assert!(!format!("{:?}", logs).contains("secret-123"));
        }
    }
}

#[tokio::test]
async fn audit_insert_failure_rolls_back_enqueue_but_terminal_is_durable() {
    let manager = setup().await;
    manager
        .db
        .execute_unprepared("DROP TABLE audit_logs")
        .await
        .unwrap();
    assert!(manager
        .enqueue_operation(APP, OP_INSTALL, HashMap::new(), None)
        .await
        .is_err());
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
    let manager = setup().await;
    let queued = manager
        .enqueue_operation(APP, OP_INSTALL, HashMap::new(), None)
        .await
        .unwrap();
    manager
        .db
        .execute_unprepared("DROP TABLE audit_logs")
        .await
        .unwrap();
    let calls = AtomicUsize::new(0);
    manager
        .process_next_operation_with(|_| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok("external deployment finished".into())
        })
        .await
        .unwrap();
    assert_eq!(
        operation(&manager, &queued.id).await.status,
        STATUS_SUCCEEDED
    );
    assert!(audit_outbox::Entity::find_by_id(&queued.id)
        .one(&manager.db)
        .await
        .unwrap()
        .unwrap()
        .processed_at
        .is_none());
    manager
        .process_next_operation_with(|_| async { panic!("cannot repeat external deployment") })
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(manager.drain_audit_outbox().await.is_err());
    assert_eq!(
        operation(&manager, &queued.id).await.status,
        STATUS_SUCCEEDED
    );
    restore_audit_table(&manager).await;
    manager.drain_audit_outbox().await.unwrap();
    manager.drain_audit_outbox().await.unwrap();
    let logs = audits(&manager).await;
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].action, "app_installed");
    assert!(logs[0].success);
    assert!(audit_outbox::Entity::find_by_id(&queued.id)
        .one(&manager.db)
        .await
        .unwrap()
        .unwrap()
        .processed_at
        .is_some());
}

async fn restore_audit_table(manager: &AppManager) {
    manager
        .db
        .execute_unprepared(
            "CREATE TABLE audit_logs (
        id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp TEXT NOT NULL, user_id INTEGER,
        username TEXT, action TEXT NOT NULL, resource_type TEXT NOT NULL,
        resource_id TEXT, details TEXT, ip_address TEXT, user_agent TEXT,
        success BOOLEAN NOT NULL, error_message TEXT)",
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn failed_operation_outbox_replays_once_after_audit_outage() {
    let manager = setup().await;
    let queued = manager
        .enqueue_operation(APP, OP_DELETE, HashMap::new(), None)
        .await
        .unwrap();
    manager
        .db
        .execute_unprepared("DROP TABLE audit_logs")
        .await
        .unwrap();
    manager
        .process_next_operation_with(|_| async {
            Err(AppError::Internal("password=private".into()))
        })
        .await
        .unwrap();
    assert_eq!(operation(&manager, &queued.id).await.status, STATUS_FAILED);
    let pending = audit_outbox::Entity::find_by_id(&queued.id)
        .one(&manager.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.error_label.as_deref(), Some("app_operation_failed"));
    assert!(!format!("{pending:?}").contains("private"));
    restore_audit_table(&manager).await;
    manager.drain_audit_outbox().await.unwrap();
    manager.drain_audit_outbox().await.unwrap();
    let logs = audits(&manager).await;
    assert_eq!(logs.len(), 1);
    assert!(!logs[0].success);
    assert_eq!(
        logs[0].error_message.as_deref(),
        Some("app_operation_failed")
    );
}

#[tokio::test]
async fn stale_running_recovers_as_indeterminate_without_reexecuting() {
    let manager = setup().await;
    let old = manager
        .enqueue_operation(APP, OP_INSTALL, HashMap::new(), Some(731))
        .await
        .unwrap();
    let recent = manager
        .enqueue_operation(APP, OP_UPDATE, HashMap::new(), None)
        .await
        .unwrap();
    let claimed_old = manager
        .claim_operation(operation(&manager, &old.id).await)
        .await
        .unwrap()
        .unwrap();
    manager
        .claim_operation(operation(&manager, &recent.id).await)
        .await
        .unwrap()
        .unwrap();
    let mut stale: app_operation::ActiveModel = claimed_old.into();
    stale.updated_at = Set(Utc::now() - chrono::Duration::minutes(16));
    stale.update(&manager.db).await.unwrap();
    manager.recover_stale_operations().await.unwrap();
    manager.recover_stale_operations().await.unwrap();
    assert_eq!(operation(&manager, &old.id).await.status, STATUS_FAILED);
    assert_eq!(
        operation(&manager, &old.id).await.error.as_deref(),
        Some(INTERRUPTED_LABEL)
    );
    assert_eq!(operation(&manager, &recent.id).await.status, STATUS_RUNNING);
    let pending = audit_outbox::Entity::find_by_id(&old.id)
        .one(&manager.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.outcome, "indeterminate");
    assert!(!pending.success);
    assert_eq!(pending.created_by, Some(731));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&pending.details).unwrap(),
        serde_json::json!({"operation_id":old.id,"attempts":1,"phase":"indeterminate"})
    );
    manager.drain_audit_outbox().await.unwrap();
    manager.drain_audit_outbox().await.unwrap();
    let logs = audits(&manager).await;
    assert_eq!(logs.len(), 3); // two enqueue events, one interruption
    assert_eq!(logs[2].error_message.as_deref(), Some(INTERRUPTED_LABEL));
    assert!(audit_outbox::Entity::find_by_id(&recent.id)
        .one(&manager.db)
        .await
        .unwrap()
        .is_none());
    manager
        .process_next_operation_with(|_| async { panic!("cannot replay running work") })
        .await
        .unwrap();
}

#[tokio::test]
async fn heartbeat_keeps_long_running_external_work_out_of_stale_recovery() {
    let manager = Arc::new(setup().await);
    let queued = manager
        .enqueue_operation(APP, OP_INSTALL, HashMap::new(), None)
        .await
        .unwrap();
    let started = Arc::new(Notify::new());
    let finish = Arc::new(Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn({
        let manager = manager.clone();
        let started = started.clone();
        let finish = finish.clone();
        let calls = calls.clone();
        async move {
            manager
                .process_next_operation_with_heartbeat(
                    move |_| async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        started.notify_one();
                        finish.notified().await;
                        Ok("external work finished".into())
                    },
                    Duration::from_millis(10),
                )
                .await
        }
    });
    started.notified().await;
    let mut stale: app_operation::ActiveModel = operation(&manager, &queued.id).await.into();
    stale.updated_at = Set(Utc::now() - chrono::Duration::minutes(16));
    stale.update(&manager.db).await.unwrap();

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if operation(&manager, &queued.id).await.updated_at
                > Utc::now() - chrono::Duration::minutes(15)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("heartbeat should refresh the running row");
    manager.recover_stale_operations().await.unwrap();
    assert_eq!(operation(&manager, &queued.id).await.status, STATUS_RUNNING);
    assert!(audit_outbox::Entity::find_by_id(&queued.id)
        .one(&manager.db)
        .await
        .unwrap()
        .is_none());

    finish.notify_one();
    task.await.unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let completed = operation(&manager, &queued.id).await;
    assert_eq!(completed.status, STATUS_SUCCEEDED);
    assert_eq!(completed.finished_at, Some(completed.updated_at));
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(operation(&manager, &queued.id).await, completed);
    let outbox = audit_outbox::Entity::find_by_id(&queued.id)
        .one(&manager.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbox.outcome, STATUS_SUCCEEDED);
    assert!(!outbox.details.contains("indeterminate"));
}

#[tokio::test]
async fn terminal_cas_allows_only_one_outbox_row() {
    let manager = setup().await;
    let queued = manager
        .enqueue_operation(APP, OP_RESTART, HashMap::new(), None)
        .await
        .unwrap();
    manager
        .claim_operation(operation(&manager, &queued.id).await)
        .await
        .unwrap()
        .unwrap();
    let (first, second) = tokio::join!(
        manager.finish_operation(&queued.id, STATUS_SUCCEEDED, Some("done".into()), None),
        manager.finish_operation(&queued.id, STATUS_FAILED, None, None)
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(
        operation(&manager, &queued.id).await.status,
        if first.is_ok() {
            STATUS_SUCCEEDED
        } else {
            STATUS_FAILED
        }
    );
    assert_eq!(
        audit_outbox::Entity::find()
            .all(&manager.db)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn queued_and_completed_audit_use_existing_actor_username() {
    let manager = setup().await;
    let now = Utc::now();
    let actor = user::ActiveModel {
        username: Set("operator".into()),
        email: Set("operator@example.test".into()),
        hashed_password: Set("never-audit-this".into()),
        is_active: Set(true),
        is_approved: Set(true),
        totp_secret: Set(None),
        totp_enabled: Set(false),
        totp_verified_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&manager.db)
    .await
    .unwrap();
    let queued = manager
        .enqueue_operation(APP, OP_UPDATE, HashMap::new(), Some(actor.id))
        .await
        .unwrap();
    manager
        .process_next_operation_with(|_| async { Ok("done".into()) })
        .await
        .unwrap();
    let logs = audits(&manager).await;
    assert_eq!(logs.len(), 2);
    for log in logs {
        assert_eq!(log.username.as_deref(), Some("operator"));
        assert_eq!(log.user_id, Some(actor.id));
        assert_eq!(log.resource_id.as_deref(), Some(APP));
        assert!(log.details.unwrap().contains(&queued.id));
    }
}

#[tokio::test]
async fn outbox_preserves_username_if_actor_is_deleted_before_delivery() {
    let manager = setup().await;
    let now = Utc::now();
    let actor = user::ActiveModel {
        username: Set("former_operator".into()),
        email: Set("former@example.test".into()),
        hashed_password: Set("not-in-audit".into()),
        is_active: Set(true),
        is_approved: Set(true),
        totp_secret: Set(None),
        totp_enabled: Set(false),
        totp_verified_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&manager.db)
    .await
    .unwrap();
    let queued = manager
        .enqueue_operation(APP, OP_RESTART, HashMap::new(), Some(actor.id))
        .await
        .unwrap();
    manager
        .db
        .execute_unprepared("DROP TABLE audit_logs")
        .await
        .unwrap();
    manager
        .process_next_operation_with(|_| async { Ok("done".into()) })
        .await
        .unwrap();
    user::Entity::delete_by_id(actor.id)
        .exec(&manager.db)
        .await
        .unwrap();
    restore_audit_table(&manager).await;
    manager.drain_audit_outbox().await.unwrap();
    let event = audits(&manager).await.pop().unwrap();
    assert_eq!(event.user_id, Some(actor.id));
    assert_eq!(event.username.as_deref(), Some("former_operator"));
    assert!(event.details.unwrap().contains(&queued.id));
}

#[tokio::test]
async fn caller_transaction_update_audit_failure_can_roll_back_operation_and_state() {
    let manager = setup().await;
    manager
        .db
        .execute_unprepared("DROP TABLE audit_logs")
        .await
        .unwrap();
    let transaction = manager.db.begin().await.unwrap();
    assert!(manager
        .enqueue_update_in_transaction(&transaction, APP, None)
        .await
        .is_err());
    transaction.rollback().await.unwrap();
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
            // Success state is published only after terminal CAS wins.
            assert!(current.updated_at >= finished.updated_at);
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
async fn cancelled_operation_loop_does_not_claim_queued_work() {
    let manager = Arc::new(setup().await);
    let queued = manager
        .enqueue_operation(APP, OP_INSTALL, HashMap::new(), None)
        .await
        .unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let calls = AtomicUsize::new(0);

    manager
        .run_operation_loop_with(Duration::from_millis(1), cancellation, |_, _| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok("unexpected execution".to_string().into())
        })
        .await;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let untouched = operation(&manager, &queued.id).await;
    assert_eq!(untouched.status, STATUS_QUEUED);
    assert_eq!(untouched.attempts, 0);
}

#[tokio::test]
async fn cancelled_operation_loop_drains_claimed_work_without_claiming_next() {
    let manager = Arc::new(setup().await);
    let first = manager
        .enqueue_operation(APP, OP_INSTALL, HashMap::new(), None)
        .await
        .unwrap();
    let second = manager
        .enqueue_operation(APP, OP_DELETE, HashMap::new(), None)
        .await
        .unwrap();
    let cancellation = CancellationToken::new();
    let started = Arc::new(Notify::new());
    let finish = Arc::new(Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));

    let task = tokio::spawn({
        let manager = manager.clone();
        let cancellation = cancellation.clone();
        let started = started.clone();
        let finish = finish.clone();
        let calls = calls.clone();
        async move {
            manager
                .run_operation_loop_with(Duration::from_millis(1), cancellation, move |_, _| {
                    let started = started.clone();
                    let finish = finish.clone();
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        started.notify_one();
                        finish.notified().await;
                        Ok("Executed during shutdown".to_string().into())
                    }
                })
                .await;
        }
    });

    started.notified().await;
    cancellation.cancel();
    finish.notify_one();
    task.await.unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        operation(&manager, &first.id).await.status,
        STATUS_SUCCEEDED
    );
    let untouched = operation(&manager, &second.id).await;
    assert_eq!(untouched.status, STATUS_QUEUED);
    assert_eq!(untouched.attempts, 0);
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

async fn write_observation(
    manager: &AppManager,
    observed_available: Option<&str>,
    installed: Option<&str>,
) {
    manager
        .upsert_state_with_chart_versions(
            APP,
            APP,
            DESIRED_INSTALLED,
            OBS_INSTALLED,
            true,
            Some("Observed".into()),
            None,
            false,
            observed_available.map(str::to_owned),
            installed.map(str::to_owned),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn stale_observer_cannot_replace_authoritative_available_version() {
    let manager = setup().await;
    write_observation(&manager, Some("2.0.0"), Some("1.0.0")).await;

    write_observation(&manager, Some("1.0.0"), Some("1.0.0")).await;

    let current = state(&manager).await;
    assert_eq!(current.installed_chart_version.as_deref(), Some("1.0.0"));
    assert_eq!(current.available_chart_version.as_deref(), Some("2.0.0"));
    assert!(current.update_available);
}

#[tokio::test]
async fn successful_sync_remains_authoritative_during_stale_reconciliation() {
    let manager = setup().await;
    write_observation(&manager, Some("1.0.0"), Some("1.0.0")).await;
    assert!(!state(&manager).await.update_available);

    manager
        .refresh_available_chart_versions_with(|name| {
            assert_eq!(name, APP);
            Some("2.0.0".into())
        })
        .await
        .unwrap();
    let synced = state(&manager).await;
    assert_eq!(synced.available_chart_version.as_deref(), Some("2.0.0"));
    assert!(synced.update_available);

    write_observation(&manager, Some("1.0.0"), Some("1.0.0")).await;
    let reconciled = state(&manager).await;
    assert_eq!(reconciled.available_chart_version.as_deref(), Some("2.0.0"));
    assert!(reconciled.update_available);
}

#[tokio::test]
async fn successful_sync_missing_catalog_entry_preserves_existing_pin() {
    let manager = setup().await;
    write_observation(&manager, Some("2.0.0"), Some("1.0.0")).await;

    manager
        .refresh_available_chart_versions_with(|_| None)
        .await
        .unwrap();

    let current = state(&manager).await;
    assert_eq!(current.available_chart_version.as_deref(), Some("2.0.0"));
    assert!(current.update_available);
}

#[tokio::test]
async fn catalog_sync_preserves_queued_operation_intent() {
    let manager = setup().await;
    write_observation(&manager, Some("1.0.0"), Some("1.0.0")).await;
    let queued = manager
        .enqueue_operation(APP, OP_DELETE, HashMap::new(), None)
        .await
        .unwrap();

    manager
        .refresh_available_chart_versions_with(|_| Some("2.0.0".into()))
        .await
        .unwrap();

    let current = state(&manager).await;
    assert_eq!(current.desired_state, DESIRED_REMOVED);
    assert_eq!(current.observed_state, OBS_DELETING);
    assert_eq!(current.last_operation_id, Some(queued.id));
    assert_eq!(current.available_chart_version.as_deref(), Some("2.0.0"));
}

#[tokio::test]
async fn test_migration_adds_auto_update_default_true() {
    let manager = setup().await;
    manager
        .db
        .execute_unprepared(
            "INSERT INTO app_states (app_name, namespace, desired_state, observed_state, \
             healthy, update_available, updated_at) \
             VALUES ('legacy-app', 'ns', 'installed', 'installed', 1, 0, '2026-01-01 00:00:00')",
        )
        .await
        .unwrap();
    let state = app_state::Entity::find_by_id("legacy-app")
        .one(&manager.db)
        .await
        .unwrap()
        .unwrap();
    assert!(state.auto_update);
}

#[tokio::test]
async fn test_notification_events_seeded() {
    use crate::models::notification_event;
    let manager = setup().await;
    for (event, enabled, severity) in [
        ("app_updated", false, "info"),
        ("app_update_failed", true, "warning"),
        ("app_update_rollback_failed", true, "critical"),
    ] {
        let row = notification_event::Entity::find()
            .filter(notification_event::Column::EventType.eq(event))
            .one(&manager.db)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("missing seeded event {event}"));
        assert_eq!((row.enabled, row.severity.as_str()), (enabled, severity));
    }
}

#[tokio::test]
async fn test_auto_update_migration_down_removes_columns_and_events() {
    use crate::models::notification_event;
    let manager = setup().await;
    crate::migrations::Migrator::down(&manager.db, Some(1))
        .await
        .unwrap();
    let seeded = notification_event::Entity::find()
        .filter(notification_event::Column::EventType.like("app_update%"))
        .all(&manager.db)
        .await
        .unwrap();
    assert!(seeded.is_empty());
    assert!(manager
        .db
        .execute_unprepared("SELECT auto_update FROM app_states")
        .await
        .is_err());
    crate::migrations::Migrator::up(&manager.db, None)
        .await
        .unwrap();
}

// ---- Update verification and rollback (Helm and cluster reads are stubbed) ----

const FROM: &str = "1.0.0";
const TO: &str = "1.1.0";
const CAPTURED: &str = "=== pods ===\ncaptured evidence";

/// Scripted cluster: each health/snapshot call consumes the next entry; the last repeats.
struct StubProbe {
    health: std::sync::Mutex<Vec<bool>>,
    snapshots: std::sync::Mutex<Vec<RestartSnapshot>>,
    /// `helm history` before Helm, then after (last entry repeats).
    histories: std::sync::Mutex<Vec<Vec<(u32, String)>>>,
    history_fails: bool,
    rollback_fails: bool,
    calls: std::sync::Mutex<Vec<String>>,
}

impl StubProbe {
    /// `deployed`: the live revision before the update. The update adds the next one.
    fn new(health: &[bool], deployed: Option<u32>) -> Self {
        let before: Vec<(u32, String)> = deployed
            .map(|revision| vec![(revision, "deployed".to_string())])
            .unwrap_or_default();
        let mut after = before.clone();
        after.push((deployed.map_or(1, |revision| revision + 1), "failed".into()));
        Self::with_histories(health, vec![before, after])
    }

    fn with_histories(health: &[bool], histories: Vec<Vec<(u32, String)>>) -> Self {
        Self {
            health: std::sync::Mutex::new(health.to_vec()),
            snapshots: std::sync::Mutex::new(vec![RestartSnapshot::new()]),
            histories: std::sync::Mutex::new(histories),
            history_fails: false,
            rollback_fails: false,
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

fn next_scripted<T: Clone>(script: &std::sync::Mutex<Vec<T>>) -> T {
    let mut script = script.lock().unwrap();
    if script.len() > 1 {
        script.remove(0)
    } else {
        script[0].clone()
    }
}

#[async_trait::async_trait]
impl UpdateProbe for StubProbe {
    async fn restart_snapshot(&self) -> Result<RestartSnapshot> {
        self.record("snapshot".into());
        Ok(next_scripted(&self.snapshots))
    }

    async fn health(&self) -> Result<(bool, String)> {
        self.record("health".into());
        let healthy = next_scripted(&self.health);
        Ok((
            healthy,
            if healthy { "" } else { "Deployment not ready" }.into(),
        ))
    }

    fn history(&self) -> Result<Vec<(u32, String)>> {
        self.record("history".into());
        if self.history_fails {
            return Err(AppError::Internal("helm history unreachable".into()));
        }
        Ok(next_scripted(&self.histories))
    }

    async fn scrub(&self, text: &str) -> String {
        update_diagnostics::scrub_inline(text)
    }

    async fn capture(&self, failed: Option<u32>, previous: Option<u32>) -> String {
        self.record(format!("capture {failed:?} {previous:?}"));
        CAPTURED.into()
    }

    fn rollback(&self, revision: u32) -> Result<()> {
        self.record(format!("rollback {revision}"));
        if self.rollback_fails {
            return Err(AppError::Internal(
                "helm rollback exploded token=rollbacksecret".into(),
            ));
        }
        Ok(())
    }
}

/// A manager that announces through the real notification service, plus a user to receive it.
async fn notifying_setup() -> (AppManager, i64) {
    let manager = setup().await;
    let notifications = crate::services::notification::NotificationService::new();
    notifications.set_db(manager.db.clone()).await;
    let manager = manager
        .with_notifications(Arc::new(notifications))
        .with_update_health_timeout(Duration::ZERO);
    let now = Utc::now();
    let owner = user::ActiveModel {
        username: Set("owner".into()),
        email: Set("owner@example.test".into()),
        hashed_password: Set("unused".into()),
        is_active: Set(true),
        is_approved: Set(true),
        totp_secret: Set(None),
        totp_enabled: Set(false),
        totp_verified_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&manager.db)
    .await
    .unwrap();
    (manager, owner.id)
}

async fn enable_event(manager: &AppManager, event: &str) {
    use crate::models::notification_event;
    notification_event::Entity::update_many()
        .col_expr(notification_event::Column::Enabled, true.into())
        .filter(notification_event::Column::EventType.eq(event))
        .exec(&manager.db)
        .await
        .unwrap();
}

async fn notifications(manager: &AppManager) -> Vec<crate::models::user_notification::Model> {
    crate::models::user_notification::Entity::find()
        .all(&manager.db)
        .await
        .unwrap()
}

/// Enqueue an update as `owner` and run it through the worker with stubbed Helm.
async fn run_update(
    manager: &AppManager,
    owner: Option<i64>,
    probe: &StubProbe,
    helm: Result<String>,
) -> app_operation::Model {
    let queued = manager
        .enqueue_operation(APP, OP_UPDATE, HashMap::new(), owner)
        .await
        .unwrap();
    manager
        .process_next_operation_with_stop(
            |_, stop| async move {
                manager
                    .run_verified_update(
                        probe,
                        UpdateVersions {
                            from: FROM.into(),
                            to: TO.into(),
                        },
                        &stop,
                        async { helm },
                    )
                    .await
            },
            RUNNING_HEARTBEAT_INTERVAL,
        )
        .await
        .unwrap();
    operation(manager, &queued.id).await
}

#[tokio::test]
async fn test_update_healthy_succeeds_and_notifies_updated() {
    let (manager, owner) = notifying_setup().await;
    enable_event(&manager, "app_updated").await; // seeded disabled
    let probe = StubProbe::new(&[true], Some(3));

    let finished = run_update(&manager, Some(owner), &probe, Ok("helm ok".into())).await;

    assert_eq!(finished.status, STATUS_SUCCEEDED);
    assert_eq!(
        finished.message.as_deref(),
        Some("Updated operation-test-app 1.0.0 → 1.1.0")
    );
    assert_eq!(finished.diagnostics, None);
    assert_eq!(finished.error, None);
    assert_eq!(probe.calls(), ["snapshot", "history", "snapshot", "health"]);
    assert_eq!(audits(&manager).await.last().unwrap().action, "app_updated");
    let sent = notifications(&manager).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].event_type.as_deref(), Some("app_updated"));
    assert_eq!(sent[0].user_id, owner);
    assert!(sent[0]
        .message
        .contains("app=operation-test-app from=1.0.0 to=1.1.0"));
}

#[tokio::test]
async fn test_update_unhealthy_rolls_back_and_stores_diagnostics() {
    let (manager, owner) = notifying_setup().await;
    let probe = StubProbe::new(&[false, true], Some(3));

    let finished = run_update(&manager, Some(owner), &probe, Ok("helm ok".into())).await;

    assert_eq!(finished.status, STATUS_FAILED);
    assert_eq!(
        finished.message.as_deref(),
        Some("Update to 1.1.0 failed; rolled back to 1.0.0")
    );
    let diagnostics = finished.diagnostics.unwrap();
    let first = diagnostics.lines().next().unwrap();
    assert_eq!(first, "Not healthy within 0 min: Deployment not ready");
    assert!(diagnostics.contains("Rolled back to revision 3"));
    assert!(diagnostics.contains(CAPTURED));
    assert_eq!(finished.error.as_deref(), Some(first));
    // Evidence is captured before the rollback replaces the failing pods,
    // and health is re-checked afterwards against a fresh restart baseline.
    assert_eq!(
        probe.calls(),
        [
            "snapshot",
            "history",
            "snapshot",
            "health",
            "history",
            "capture Some(4) Some(3)",
            "snapshot",
            "rollback 3",
            "snapshot",
            "health"
        ]
    );
    assert_eq!(
        audits(&manager).await.last().unwrap().action,
        "app_update_failed"
    );
    assert_eq!(state(&manager).await.observed_state, OBS_FAILED);
    let sent = notifications(&manager).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].event_type.as_deref(), Some("app_update_failed"));
    assert_eq!(sent[0].severity, "warning");
    assert!(sent[0].message.contains(&format!(
        "app=operation-test-app from=1.0.0 to=1.1.0; rolled back to 1.0.0; {first}"
    )));
}

#[tokio::test]
async fn test_update_restart_limit_rolls_back_even_when_ready() {
    let (manager, owner) = notifying_setup().await;
    let probe = StubProbe::new(&[true], Some(3));
    *probe.snapshots.lock().unwrap() = vec![
        HashMap::from([("pod/app".to_string(), 1)]), // before Helm
        HashMap::from([("pod/app".to_string(), 4)]), // +3 after
        HashMap::new(),                              // after rollback: new pods
    ];

    let finished = run_update(&manager, Some(owner), &probe, Ok("helm ok".into())).await;

    assert_eq!(finished.status, STATUS_FAILED);
    assert!(finished
        .diagnostics
        .unwrap()
        .starts_with("A container restarted more than 2 times since the update started\n"));
    assert!(probe.calls().contains(&"rollback 3".to_string()));
}

#[tokio::test]
async fn test_update_helm_wait_failure_still_rolls_back() {
    let (manager, owner) = notifying_setup().await;
    let probe = StubProbe::new(&[false, true], Some(7));

    let finished = run_update(
        &manager,
        Some(owner),
        &probe,
        Err(AppError::Internal(
            "Error: UPGRADE FAILED: context deadline exceeded\nmore".into(),
        )),
    )
    .await;

    assert_eq!(finished.status, STATUS_FAILED);
    assert_eq!(
        finished.message.as_deref(),
        Some("Update to 1.1.0 failed; rolled back to 1.0.0")
    );
    let diagnostics = finished.diagnostics.unwrap();
    let first = diagnostics.lines().next().unwrap();
    assert!(first.starts_with("Helm upgrade failed: "), "{first}");
    assert!(first.contains("UPGRADE FAILED: context deadline exceeded"));
    assert!(probe.calls().contains(&"rollback 7".to_string()));
    assert_eq!(
        audits(&manager).await.last().unwrap().action,
        "app_update_failed"
    );
}

#[tokio::test]
async fn test_helm_failure_with_healthy_app_notifies_not_applied() {
    let (manager, owner) = notifying_setup().await;
    let unchanged = vec![(3, "deployed".to_string())];
    let probe = StubProbe::with_histories(&[true], vec![unchanged.clone(), unchanged]);

    let finished = run_update(
        &manager,
        Some(owner),
        &probe,
        Err(AppError::Internal("chart pull failed".into())),
    )
    .await;

    assert_eq!(finished.status, STATUS_FAILED);
    assert_eq!(
        finished.message.as_deref(),
        Some("Update to 1.1.0 failed; operation-test-app unchanged at 1.0.0")
    );
    let diagnostics = finished.diagnostics.unwrap();
    assert!(
        diagnostics.starts_with("Helm upgrade failed: "),
        "{diagnostics}"
    );
    assert!(diagnostics.contains("chart pull failed"));
    assert!(!probe
        .calls()
        .iter()
        .any(|call| call.starts_with("rollback")));
    assert_eq!(
        audits(&manager).await.last().unwrap().action,
        "app_update_failed"
    );
    let sent = notifications(&manager).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].event_type.as_deref(), Some("app_update_failed"));
    assert!(sent[0]
        .message
        .contains("Helm reported an error; the app is still running 1.0.0"));
    assert!(
        !sent[0].message.contains("rolled back"),
        "{}",
        sent[0].message
    );
}

#[tokio::test]
async fn test_no_new_revision_never_rolls_back_below_pre_update() {
    // Helm failed before writing a revision while the app was already unhealthy:
    // rolling back to "the previous deployed revision" would go to 2, older than live 3.
    let (manager, owner) = notifying_setup().await;
    let history = vec![(2, "superseded".to_string()), (3, "deployed".to_string())];
    let probe = StubProbe::with_histories(&[false], vec![history.clone(), history]);

    let finished = run_update(
        &manager,
        Some(owner),
        &probe,
        Err(AppError::Internal("values validation failed".into())),
    )
    .await;

    assert_eq!(finished.status, STATUS_FAILED);
    let diagnostics = finished.diagnostics.unwrap();
    assert!(diagnostics.contains("No new Helm revision was created; nothing to roll back"));
    assert!(diagnostics.contains(CAPTURED));
    assert!(!probe
        .calls()
        .iter()
        .any(|call| call.starts_with("rollback")));
    assert_eq!(
        audits(&manager).await.last().unwrap().action,
        "app_update_failed"
    );
}

#[tokio::test]
async fn test_rollback_targets_revision_deployed_before_update() {
    // A failed earlier attempt (4) is the latest revision; the live one is 3.
    let (manager, owner) = notifying_setup().await;
    let before = vec![(3, "deployed".to_string()), (4, "failed".to_string())];
    let mut after = before.clone();
    after.push((5, "failed".into()));
    let probe = StubProbe::with_histories(&[false, true], vec![before, after]);

    let finished = run_update(&manager, Some(owner), &probe, Ok("helm ok".into())).await;

    assert_eq!(finished.status, STATUS_FAILED);
    let calls = probe.calls();
    assert!(
        calls.contains(&"capture Some(5) Some(3)".to_string()),
        "{calls:?}"
    );
    assert!(calls.contains(&"rollback 3".to_string()), "{calls:?}");
}

#[tokio::test]
async fn test_helm_history_failure_skips_rollback() {
    let (manager, owner) = notifying_setup().await;
    let mut probe = StubProbe::new(&[false], Some(3));
    probe.history_fails = true;

    let finished = run_update(&manager, Some(owner), &probe, Ok("helm ok".into())).await;

    assert!(finished
        .diagnostics
        .unwrap()
        .contains("Rollback not attempted: Helm history unavailable"));
    assert!(!probe
        .calls()
        .iter()
        .any(|call| call.starts_with("rollback")));
    assert_eq!(
        audits(&manager).await.last().unwrap().action,
        "app_update_rollback_failed"
    );
}

#[tokio::test]
async fn test_helm_and_rollback_errors_are_scrubbed_everywhere() {
    let (manager, owner) = notifying_setup().await;
    let mut probe = StubProbe::new(&[false], Some(3));
    probe.rollback_fails = true;

    let finished = run_update(
        &manager,
        Some(owner),
        &probe,
        Err(AppError::Internal(
            "Error: UPGRADE FAILED: --set password=hunter22 rejected".into(),
        )),
    )
    .await;

    let diagnostics = finished.diagnostics.clone().unwrap();
    let error = finished.error.clone().unwrap();
    let state_message = state(&manager).await.message.unwrap_or_default();
    let sent = notifications(&manager).await;
    assert_eq!(sent.len(), 1);
    let audit = format!("{:?}", audits(&manager).await);
    for text in [
        &diagnostics,
        &error,
        &state_message,
        &sent[0].message,
        &audit,
    ] {
        assert!(!text.contains("hunter22"), "{text}");
        assert!(!text.contains("rollbacksecret"), "{text}");
    }
    assert!(error.contains("password=[redacted]"), "{error}");
    assert!(sent[0].message.contains("password=[redacted]"));
    assert!(diagnostics.contains("token=[redacted]"), "{diagnostics}");
}

#[tokio::test]
async fn test_rollback_without_previous_revision_notifies_critical() {
    let (manager, owner) = notifying_setup().await;
    let probe = StubProbe::new(&[false], None);

    let finished = run_update(&manager, Some(owner), &probe, Ok("helm ok".into())).await;

    assert_eq!(finished.status, STATUS_FAILED);
    assert_eq!(
        finished.message.as_deref(),
        Some("Update to 1.1.0 failed; rollback to 1.0.0 failed")
    );
    let diagnostics = finished.diagnostics.unwrap();
    assert!(diagnostics.contains("Rollback not attempted: no deployed revision before the update"));
    assert!(diagnostics.contains(CAPTURED));
    assert!(!probe
        .calls()
        .iter()
        .any(|call| call.starts_with("rollback")));
    assert_eq!(
        audits(&manager).await.last().unwrap().action,
        "app_update_rollback_failed"
    );
    let sent = notifications(&manager).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].event_type.as_deref(),
        Some("app_update_rollback_failed")
    );
    assert_eq!(sent[0].severity, "critical");
}

#[tokio::test]
async fn test_rollback_error_or_still_unhealthy_is_rollback_failed() {
    for rollback_fails in [true, false] {
        let (manager, owner) = notifying_setup().await;
        let mut probe = StubProbe::new(&[false], Some(2));
        probe.rollback_fails = rollback_fails;

        let finished = run_update(&manager, Some(owner), &probe, Ok("helm ok".into())).await;

        assert_eq!(finished.status, STATUS_FAILED);
        let diagnostics = finished.diagnostics.unwrap();
        assert!(diagnostics.contains(if rollback_fails {
            "Rollback to revision 2 failed"
        } else {
            "Still unhealthy after rollback to revision 2"
        }));
        assert_eq!(
            audits(&manager).await.last().unwrap().action,
            "app_update_rollback_failed"
        );
    }
}

#[tokio::test]
async fn test_install_path_unchanged() {
    let (manager, owner) = notifying_setup().await;
    for event in [
        "app_updated",
        "app_update_failed",
        "app_update_rollback_failed",
    ] {
        enable_event(&manager, event).await;
    }
    let queued = manager
        .enqueue_operation(APP, OP_INSTALL, HashMap::new(), Some(owner))
        .await
        .unwrap();

    manager
        .process_next_operation_with(|_| async { Ok("Installed".into()) })
        .await
        .unwrap();

    let finished = operation(&manager, &queued.id).await;
    assert_eq!(finished.status, STATUS_SUCCEEDED);
    assert_eq!(finished.message.as_deref(), Some("Installed"));
    assert_eq!(finished.diagnostics, None);
    assert_eq!(
        audits(&manager).await.last().unwrap().action,
        "app_installed"
    );
    assert!(notifications(&manager).await.is_empty());
}

#[test]
fn test_compose_diagnostics_caps_summary_and_size() {
    let summary = format!("{}\nRolled back", "x".repeat(1000));
    let report = compose_diagnostics(&summary, &"y".repeat(70 * 1024));
    let first = report.lines().next().unwrap();
    assert_eq!(first.len(), SUMMARY_LINE_MAX);
    assert!(report.len() <= update_diagnostics::MAX_DIAGNOSTICS_BYTES);
    assert!(report.ends_with("[truncated]"));
}

#[tokio::test]
async fn test_stop_during_verification_abandons_without_rollback() {
    let manager = setup()
        .await
        .with_update_health_timeout(Duration::from_secs(3600));
    let probe = StubProbe::new(&[false], Some(3));
    let stop = CancellationToken::new();
    let stopper = stop.clone();
    let result = manager
        .run_verified_update(
            &probe,
            UpdateVersions {
                from: FROM.into(),
                to: TO.into(),
            },
            &stop,
            async move {
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    stopper.cancel();
                });
                Ok("helm ok".to_string())
            },
        )
        .await;
    assert!(result.is_err());
    assert!(!probe
        .calls()
        .iter()
        .any(|call| call.starts_with("rollback")));
}

#[tokio::test]
async fn test_auto_update_outcome_reaches_settings_managers() {
    use crate::models::{role, role_permission, user_role};
    let (manager, admin) = notifying_setup().await;
    let now = Utc::now();
    let bystander = user::ActiveModel {
        username: Set("bystander".into()),
        email: Set("bystander@example.test".into()),
        hashed_password: Set("unused".into()),
        is_active: Set(true),
        is_approved: Set(true),
        totp_secret: Set(None),
        totp_enabled: Set(false),
        totp_verified_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&manager.db)
    .await
    .unwrap();
    let ops = role::ActiveModel {
        name: Set("update-ops".into()),
        description: Set(None),
        is_system: Set(false),
        requires_2fa: Set(false),
        created_at: Set(now),
        ..Default::default()
    }
    .insert(&manager.db)
    .await
    .unwrap();
    role_permission::ActiveModel {
        role_id: Set(ops.id),
        permission: Set("settings.manage".into()),
        ..Default::default()
    }
    .insert(&manager.db)
    .await
    .unwrap();
    user_role::ActiveModel {
        user_id: Set(admin),
        role_id: Set(ops.id),
    }
    .insert(&manager.db)
    .await
    .unwrap();
    let probe = StubProbe::new(&[false], None);

    run_update(&manager, None, &probe, Ok("helm ok".into())).await;

    let sent = notifications(&manager).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].user_id, admin);
    assert_ne!(sent[0].user_id, bystander.id);
    assert_eq!(
        sent[0].event_type.as_deref(),
        Some("app_update_rollback_failed")
    );
}

#[tokio::test]
async fn test_health_timeout_follows_settings_unless_overridden() {
    let manager = setup().await;
    assert_eq!(manager.health_timeout().await, Duration::from_secs(600));
    let settings = auto_update_settings::AutoUpdateSettings {
        health_timeout_minutes: 25,
        ..Default::default()
    };
    auto_update_settings::save(&manager.db, &settings, None)
        .await
        .unwrap();
    assert_eq!(manager.health_timeout().await, Duration::from_secs(25 * 60));
    let fixed = manager.with_update_health_timeout(Duration::from_secs(1));
    assert_eq!(fixed.health_timeout().await, Duration::from_secs(1));
}

#[tokio::test]
async fn test_auto_op_not_executed_when_chart_moved_since_queued() {
    let manager = setup().await;
    // Installed 1.30.0, the loop vetted 1.30.1; the chart on disk is now 2.0.0.
    manager
        .enqueue_operation(APP, OP_INSTALL, HashMap::new(), None)
        .await
        .unwrap();
    app_operation::Entity::delete_many()
        .exec(&manager.db)
        .await
        .unwrap();
    let mut row: app_state::ActiveModel = state(&manager).await.into();
    row.installed_chart_version = Set(Some("1.30.0".into()));
    row.update(&manager.db).await.unwrap();
    manager.enqueue_auto_update(APP, "1.30.1").await.unwrap();
    let helm_calls = AtomicUsize::new(0);
    let calls = &helm_calls;
    let manager_ref = &manager;
    manager
        .process_next_operation_with_stop(
            |op, _stop| async move {
                manager_ref
                    .guard_auto_target(&op, Some("1.30.1"), Some("2.0.0"))
                    .await?;
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(OperationOutput::Done("deployed".into()))
            },
            RUNNING_HEARTBEAT_INTERVAL,
        )
        .await
        .unwrap();
    assert_eq!(helm_calls.load(Ordering::SeqCst), 0);
    let op = app_operation::Entity::find()
        .one(&manager.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(op.status, STATUS_FAILED);
    assert!(op.error.as_deref().unwrap().contains(AUTO_SKIPPED_MESSAGE));
    let mut row: app_state::ActiveModel = state(&manager).await.into();
    row.installed_chart_version = Set(Some("1.30.0".into()));
    row.update(&manager.db).await.unwrap();
    // Still vetted: same chart on disk and an eligible minor update passes.
    manager
        .guard_auto_target(&op, Some("1.30.1"), Some("1.30.1"))
        .await
        .unwrap();
    // A major or an unknown target is refused.
    assert!(manager
        .guard_auto_target(&op, Some("2.0.0"), Some("2.0.0"))
        .await
        .is_err());
    assert!(manager
        .guard_auto_target(&op, None, Some("1.30.1"))
        .await
        .is_err());
}
