//! PostgreSQL-only migration and audit write regression test.
//! Run against a *fresh*, disposable local `audit_test` database via
//! AUDIT_TEST_POSTGRES_URL; never use DATABASE_URL (migration_tests.rs can drop tables).

use kubarr::migrations::Migrator;
use kubarr::models::audit_log::{self, AuditAction, ResourceType};
use kubarr::services::audit::{get_audit_logs, get_audit_stats, log_on_transaction, AuditLogQuery};
use sea_orm::{ConnectionTrait, Database, DbBackend, EntityTrait, Statement, TransactionTrait};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn postgres_audit_details_migration_and_transactional_insert() {
    let url = std::env::var("AUDIT_TEST_POSTGRES_URL").expect(
        "AUDIT_TEST_POSTGRES_URL must point to a fresh disposable local PostgreSQL audit_test database",
    );
    // Opt-in to a dedicated loopback test database only; never run migrations on
    // an arbitrary host or on the application's DATABASE_URL.
    let address = url
        .strip_prefix("postgres://kubarr_test:")
        .and_then(|rest| rest.split_once('@').map(|(_, address)| address))
        .expect("AUDIT_TEST_POSTGRES_URL must use postgres://kubarr_test:<password>@localhost:<port>/audit_test");
    let (host_port, database) = address
        .split_once('/')
        .expect("AUDIT_TEST_POSTGRES_URL must include /audit_test");
    let host = host_port.split(':').next().unwrap_or_default();
    assert!(
        (host == "localhost" || host == "127.0.0.1") && database == "audit_test",
        "AUDIT_TEST_POSTGRES_URL must target localhost/127.0.0.1 and the dedicated audit_test database"
    );

    let db = Database::connect(&url)
        .await
        .expect("connect to disposable PostgreSQL audit_test database");
    assert_eq!(db.get_database_backend(), DbBackend::Postgres);
    let existing = db
        .query_one(Statement::from_string(
            DbBackend::Postgres,
            "SELECT count(*) AS tables FROM pg_tables WHERE schemaname = 'public'".to_owned(),
        ))
        .await
        .expect("check that public schema is empty")
        .expect("table count row");
    assert_eq!(
        existing.try_get::<i64>("", "tables").expect("table count"),
        0,
        "audit_test must be fresh; refusing to migrate a database containing existing tables"
    );

    Migrator::up(&db, None)
        .await
        .expect("apply all migrations, including audit details TEXT migration 000030");
    let details_column = db
        .query_one(Statement::from_string(
            DbBackend::Postgres,
            "SELECT data_type FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'audit_logs' AND column_name = 'details'".to_owned(),
        ))
        .await
        .expect("inspect audit_logs.details")
        .expect("audit_logs.details column must exist");
    assert_eq!(
        details_column
            .try_get::<String>("", "data_type")
            .expect("data_type"),
        "text"
    );
    let outbox = db
        .query_one(Statement::from_string(
            DbBackend::Postgres,
            "SELECT to_regclass('public.app_audit_outbox')::text AS table_name".to_owned(),
        ))
        .await
        .expect("check outbox schema")
        .expect("outbox schema row");
    assert_eq!(
        outbox
            .try_get::<Option<String>>("", "table_name")
            .expect("outbox name"),
        Some("app_audit_outbox".to_owned())
    );

    let details = serde_json::json!({"role_ids": (0..150).collect::<Vec<i64>>()});
    let encoded = details.to_string();
    assert!(encoded.len() > 255 && encoded.len() < 8192);
    let username = format!("{}end", "é".repeat(130));
    assert!(username.len() > 255);
    let txn = db.begin().await.expect("begin audit transaction");
    let inserted = log_on_transaction(
        &txn,
        AuditAction::UserUpdated,
        ResourceType::User,
        Some("42".to_owned()),
        Some(42),
        Some(username),
        Some(details.clone()),
        None,
        None,
        true,
        None,
    )
    .await
    .expect("insert oversized allowlisted details on PostgreSQL transaction");
    txn.commit().await.expect("commit audit insert");

    let persisted = audit_log::Entity::find_by_id(inserted.id)
        .one(&db)
        .await
        .expect("read committed audit row")
        .expect("committed audit row exists");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            persisted.details.as_deref().expect("details stored")
        )
        .expect("valid details JSON"),
        details,
        "large allowlisted details must not be truncated or redacted"
    );
    let actor = persisted.username.expect("actor username stored");
    assert_eq!(actor, "é".repeat(127));
    assert!(actor.len() <= 255 && actor.is_char_boundary(actor.len()));
    assert_eq!(persisted.user_id, Some(42));

    let stats = get_audit_stats(&db)
        .await
        .expect("PostgreSQL audit aggregation");
    assert_eq!(stats.total_events, 1);
    assert_eq!(stats.successful_events, 1);
    assert_eq!(stats.top_actions[0].action, "user_updated");
    assert_eq!(stats.top_actions[0].count, 1);

    let filtered = get_audit_logs(
        &db,
        AuditLogQuery {
            page: None,
            per_page: None,
            action: Some("user_updated".into()),
            user_id: Some(42),
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: None,
        },
    )
    .await
    .expect("PostgreSQL audit filtering");
    assert_eq!(filtered.total, 1);
    assert_eq!(filtered.logs[0].id, inserted.id);
}
