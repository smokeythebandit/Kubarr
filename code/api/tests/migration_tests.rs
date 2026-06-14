//! Migration tests - verify that all migrations work correctly
//!
//! Tests cover:
//! - Applying all migrations (up)
//! - Rolling back all migrations (down)
//! - Verifying correct table structure
//! - Testing foreign key relationships
//!
//! Tests run against both SQLite (in-memory) and PostgreSQL (if DATABASE_URL is set).
//! To run PostgreSQL tests:
//!   DATABASE_URL=postgres://user:pass@localhost/test_db cargo test --test migration_tests

use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, QueryResult, Statement};
use sea_orm_migration::MigratorTrait;

use kubarr::migrations::Migrator;

/// Helper to create a fresh in-memory SQLite database without running migrations
async fn create_sqlite_db() -> DatabaseConnection {
    Database::connect("sqlite::memory:")
        .await
        .expect("Failed to create SQLite test database")
}

/// Helper to create a PostgreSQL database connection for testing.
/// Returns None if DATABASE_URL is not set.
async fn create_postgres_db() -> Option<DatabaseConnection> {
    let db_url = std::env::var("DATABASE_URL").ok()?;
    if !db_url.starts_with("postgres") {
        return None;
    }

    let db = Database::connect(&db_url)
        .await
        .expect("Failed to connect to PostgreSQL test database");

    // Clean up any existing tables from previous test runs
    cleanup_postgres_tables(&db).await;

    Some(db)
}

/// Clean up PostgreSQL tables for a fresh test
async fn cleanup_postgres_tables(db: &DatabaseConnection) {
    // Drop all tables in reverse dependency order
    let tables = [
        "user_notifications",
        "notification_logs",
        "user_notification_prefs",
        "notification_events",
        "notification_channels",
        "audit_logs",
        "invites",
        "user_preferences",
        "system_settings",
        "storage_config",
        "oauth_providers",
        "oauth_accounts",
        "vpn_providers",
        "app_vpn_configs",
        "role_permissions",
        "role_app_permissions",
        "user_roles",
        "roles",
        "users",
        "seaql_migrations",
    ];

    for table in tables {
        let sql = format!("DROP TABLE IF EXISTS \"{}\" CASCADE", table);
        let _ = db
            .execute(Statement::from_string(DbBackend::Postgres, sql))
            .await;
    }
}

/// Helper to get table names from the database
async fn get_table_names(db: &DatabaseConnection) -> Vec<String> {
    let backend = db.get_database_backend();
    let sql = match backend {
        DbBackend::Sqlite => {
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'seaql_%' ORDER BY name".to_string()
        }
        DbBackend::Postgres => {
            "SELECT tablename AS name FROM pg_tables WHERE schemaname = 'public' AND tablename NOT LIKE 'seaql_%' ORDER BY tablename".to_string()
        }
        _ => panic!("Unsupported database backend"),
    };

    let result: Vec<QueryResult> = db
        .query_all(Statement::from_string(backend, sql))
        .await
        .expect("Failed to query tables");

    result
        .iter()
        .filter_map(|row| row.try_get::<String>("", "name").ok())
        .collect()
}

/// Helper to get column info for a table
async fn get_table_columns(db: &DatabaseConnection, table: &str) -> Vec<(String, String)> {
    let backend = db.get_database_backend();
    let sql = match backend {
        DbBackend::Sqlite => format!("PRAGMA table_info({})", table),
        DbBackend::Postgres => format!(
            "SELECT column_name AS name, data_type AS type FROM information_schema.columns WHERE table_name = '{}' AND table_schema = 'public'",
            table
        ),
        _ => panic!("Unsupported database backend"),
    };

    let result: Vec<QueryResult> = db
        .query_all(Statement::from_string(backend, sql))
        .await
        .expect("Failed to query table info");

    result
        .iter()
        .filter_map(|row| {
            let name: String = row.try_get("", "name").ok()?;
            let col_type: String = row.try_get("", "type").ok()?;
            Some((name, col_type))
        })
        .collect()
}

/// Helper to get foreign key info for a table
async fn get_foreign_keys(db: &DatabaseConnection, table: &str) -> Vec<(String, String, String)> {
    let backend = db.get_database_backend();

    match backend {
        DbBackend::Sqlite => {
            let sql = format!("PRAGMA foreign_key_list({})", table);
            let result: Vec<QueryResult> = db
                .query_all(Statement::from_string(backend, sql))
                .await
                .expect("Failed to query foreign keys");

            result
                .iter()
                .filter_map(|row| {
                    let from: String = row.try_get("", "from").ok()?;
                    let table: String = row.try_get("", "table").ok()?;
                    let to: String = row.try_get("", "to").ok()?;
                    Some((from, table, to))
                })
                .collect()
        }
        DbBackend::Postgres => {
            let sql = format!(
                r#"
                SELECT
                    kcu.column_name AS from_col,
                    ccu.table_name AS to_table,
                    ccu.column_name AS to_col
                FROM information_schema.table_constraints tc
                JOIN information_schema.key_column_usage kcu
                    ON tc.constraint_name = kcu.constraint_name
                    AND tc.table_schema = kcu.table_schema
                JOIN information_schema.constraint_column_usage ccu
                    ON ccu.constraint_name = tc.constraint_name
                    AND ccu.table_schema = tc.table_schema
                WHERE tc.constraint_type = 'FOREIGN KEY'
                    AND tc.table_name = '{}'
                "#,
                table
            );
            let result: Vec<QueryResult> = db
                .query_all(Statement::from_string(backend, sql))
                .await
                .expect("Failed to query foreign keys");

            result
                .iter()
                .filter_map(|row| {
                    let from: String = row.try_get("", "from_col").ok()?;
                    let table: String = row.try_get("", "to_table").ok()?;
                    let to: String = row.try_get("", "to_col").ok()?;
                    Some((from, table, to))
                })
                .collect()
        }
        _ => panic!("Unsupported database backend"),
    }
}

/// Helper to get index info
async fn get_indexes(db: &DatabaseConnection, table: &str) -> Vec<String> {
    let backend = db.get_database_backend();

    let sql = match backend {
        DbBackend::Sqlite => format!(
            "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='{}' AND name NOT LIKE 'sqlite_%'",
            table
        ),
        DbBackend::Postgres => format!(
            "SELECT indexname AS name FROM pg_indexes WHERE tablename = '{}' AND schemaname = 'public'",
            table
        ),
        _ => panic!("Unsupported database backend"),
    };

    let result: Vec<QueryResult> = db
        .query_all(Statement::from_string(backend, sql))
        .await
        .expect("Failed to query indexes");

    result
        .iter()
        .filter_map(|row| row.try_get::<String>("", "name").ok())
        .collect()
}

/// Run a test against both SQLite and PostgreSQL (if available)
macro_rules! test_both_databases {
    ($test_name:ident, $test_fn:expr) => {
        pastey::paste! {
            #[tokio::test]
            async fn [<$test_name _sqlite>]() {
                let db = create_sqlite_db().await;
                $test_fn(&db).await;
            }

            #[tokio::test]
            async fn [<$test_name _postgres>]() {
                if let Some(db) = create_postgres_db().await {
                    $test_fn(&db).await;
                } else {
                    eprintln!("Skipping PostgreSQL test: DATABASE_URL not set");
                }
            }
        }
    };
}

// =============================================================================
// Migration Application Tests
// =============================================================================

async fn migrations_up_succeeds_impl(db: &DatabaseConnection) {
    let result = Migrator::up(db, None).await;
    assert!(
        result.is_ok(),
        "Migrations should apply successfully: {:?}",
        result.err()
    );
}

test_both_databases!(test_migrations_up_succeeds, migrations_up_succeeds_impl);

async fn migrations_down_succeeds_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let result = Migrator::down(db, None).await;
    assert!(
        result.is_ok(),
        "Migrations should roll back successfully: {:?}",
        result.err()
    );

    let tables = get_table_names(db).await;
    assert!(
        tables.is_empty(),
        "All tables should be dropped, found: {:?}",
        tables
    );
}

test_both_databases!(test_migrations_down_succeeds, migrations_down_succeeds_impl);

async fn migrations_up_down_up_succeeds_impl(db: &DatabaseConnection) {
    Migrator::up(db, None).await.expect("First up failed");
    Migrator::down(db, None).await.expect("Down failed");
    let result = Migrator::up(db, None).await;
    assert!(
        result.is_ok(),
        "Second up should succeed: {:?}",
        result.err()
    );
}

test_both_databases!(
    test_migrations_up_down_up_succeeds,
    migrations_up_down_up_succeeds_impl
);

async fn migrations_are_idempotent_impl(db: &DatabaseConnection) {
    Migrator::up(db, None).await.expect("First up failed");
    let result = Migrator::up(db, None).await;
    assert!(
        result.is_ok(),
        "Second up should be idempotent: {:?}",
        result.err()
    );
}

test_both_databases!(
    test_migrations_are_idempotent,
    migrations_are_idempotent_impl
);

// =============================================================================
// Table Creation Tests
// =============================================================================

async fn all_tables_created_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let tables = get_table_names(db).await;

    let expected_tables = [
        "app_vpn_configs",
        "audit_logs",
        "invites",
        "notification_channels",
        "notification_events",
        "notification_logs",
        "oauth_accounts",
        "oauth_providers",
        "role_app_permissions",
        "role_permissions",
        "roles",
        "server_config",
        "sessions",
        "storage_config",
        "system_settings",
        "user_notification_prefs",
        "user_notifications",
        "user_preferences",
        "user_roles",
        "users",
        "vpn_providers",
        "two_factor_recovery_codes",
        "cloudflare_tunnels",
    ];

    for table in expected_tables {
        assert!(
            tables.contains(&table.to_string()),
            "Table '{}' should exist. Found tables: {:?}",
            table,
            tables
        );
    }

    assert_eq!(
        tables.len(),
        expected_tables.len(),
        "Should have exactly {} tables, found {}",
        expected_tables.len(),
        tables.len()
    );
}

test_both_databases!(test_all_tables_created, all_tables_created_impl);

// =============================================================================
// Schema Structure Tests
// =============================================================================

async fn users_table_structure_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let columns = get_table_columns(db, "users").await;
    let column_names: Vec<&str> = columns.iter().map(|(n, _)| n.as_str()).collect();

    let expected_columns = [
        "id",
        "username",
        "email",
        "hashed_password",
        "is_active",
        "is_approved",
        "totp_secret",
        "totp_enabled",
        "totp_verified_at",
        "created_at",
        "updated_at",
    ];

    for col in expected_columns {
        assert!(
            column_names.contains(&col),
            "Column '{}' should exist in users table. Found: {:?}",
            col,
            column_names
        );
    }
}

test_both_databases!(test_users_table_structure, users_table_structure_impl);

async fn roles_table_structure_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let columns = get_table_columns(db, "roles").await;
    let column_names: Vec<&str> = columns.iter().map(|(n, _)| n.as_str()).collect();

    let expected_columns = [
        "id",
        "name",
        "description",
        "is_system",
        "requires_2fa",
        "created_at",
    ];

    for col in expected_columns {
        assert!(
            column_names.contains(&col),
            "Column '{}' should exist in roles table",
            col
        );
    }
}

test_both_databases!(test_roles_table_structure, roles_table_structure_impl);

// Note: oauth2_* tables removed in favor of oauth_providers and oauth_accounts

async fn audit_logs_table_structure_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let columns = get_table_columns(db, "audit_logs").await;
    let column_names: Vec<&str> = columns.iter().map(|(n, _)| n.as_str()).collect();

    let expected_columns = [
        "id",
        "timestamp",
        "user_id",
        "username",
        "action",
        "resource_type",
        "resource_id",
        "details",
        "ip_address",
        "user_agent",
        "success",
        "error_message",
    ];

    for col in expected_columns {
        assert!(
            column_names.contains(&col),
            "Column '{}' should exist in audit_logs table",
            col
        );
    }
}

test_both_databases!(
    test_audit_logs_table_structure,
    audit_logs_table_structure_impl
);

// =============================================================================
// Foreign Key Tests
// =============================================================================

async fn user_roles_foreign_keys_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let fks = get_foreign_keys(db, "user_roles").await;

    let has_users_fk = fks
        .iter()
        .any(|(from, table, to)| from == "user_id" && table == "users" && to == "id");
    let has_roles_fk = fks
        .iter()
        .any(|(from, table, to)| from == "role_id" && table == "roles" && to == "id");

    assert!(
        has_users_fk,
        "user_roles should have FK to users. FKs: {:?}",
        fks
    );
    assert!(
        has_roles_fk,
        "user_roles should have FK to roles. FKs: {:?}",
        fks
    );
}

test_both_databases!(test_user_roles_foreign_keys, user_roles_foreign_keys_impl);

// Note: oauth2_authorization_codes and oauth2_tokens foreign key tests removed
// These tables were replaced by oauth_providers and oauth_accounts

async fn invites_foreign_keys_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let fks = get_foreign_keys(db, "invites").await;

    let has_created_by_fk = fks
        .iter()
        .any(|(from, table, to)| from == "created_by_id" && table == "users" && to == "id");
    let has_used_by_fk = fks
        .iter()
        .any(|(from, table, to)| from == "used_by_id" && table == "users" && to == "id");

    assert!(
        has_created_by_fk,
        "invites should have FK to users (created_by_id). FKs: {:?}",
        fks
    );
    assert!(
        has_used_by_fk,
        "invites should have FK to users (used_by_id). FKs: {:?}",
        fks
    );
}

test_both_databases!(test_invites_foreign_keys, invites_foreign_keys_impl);

async fn user_notification_prefs_foreign_keys_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let fks = get_foreign_keys(db, "user_notification_prefs").await;

    let has_users_fk = fks
        .iter()
        .any(|(from, table, to)| from == "user_id" && table == "users" && to == "id");

    assert!(
        has_users_fk,
        "user_notification_prefs should have FK to users. FKs: {:?}",
        fks
    );
}

test_both_databases!(
    test_user_notification_prefs_foreign_keys,
    user_notification_prefs_foreign_keys_impl
);

// =============================================================================
// Index Tests
// =============================================================================

async fn users_indexes_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let indexes = get_indexes(db, "users").await;

    assert!(
        indexes.iter().any(|i| i.contains("username")),
        "users should have index on username. Indexes: {:?}",
        indexes
    );
    assert!(
        indexes.iter().any(|i| i.contains("email")),
        "users should have index on email. Indexes: {:?}",
        indexes
    );
}

test_both_databases!(test_users_indexes, users_indexes_impl);

async fn audit_logs_indexes_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let indexes = get_indexes(db, "audit_logs").await;

    assert!(
        indexes.iter().any(|i| i.contains("timestamp")),
        "audit_logs should have index on timestamp. Indexes: {:?}",
        indexes
    );
    assert!(
        indexes.iter().any(|i| i.contains("user_id")),
        "audit_logs should have index on user_id. Indexes: {:?}",
        indexes
    );
    assert!(
        indexes.iter().any(|i| i.contains("action")),
        "audit_logs should have index on action. Indexes: {:?}",
        indexes
    );
}

test_both_databases!(test_audit_logs_indexes, audit_logs_indexes_impl);

// =============================================================================
// Data Insertion Tests (verify schema works for actual data)
// =============================================================================

async fn can_insert_user_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let backend = db.get_database_backend();
    let sql = match backend {
        DbBackend::Sqlite => "INSERT INTO users (username, email, hashed_password, is_active, is_approved, totp_enabled, created_at, updated_at) VALUES ('testuser', 'test@example.com', 'hashed', 1, 1, 0, datetime('now'), datetime('now'))".to_string(),
        DbBackend::Postgres => "INSERT INTO users (username, email, hashed_password, is_active, is_approved, totp_enabled, created_at, updated_at) VALUES ('testuser', 'test@example.com', 'hashed', true, true, false, NOW(), NOW())".to_string(),
        _ => panic!("Unsupported database backend"),
    };

    let result = db.execute(Statement::from_string(backend, sql)).await;
    assert!(
        result.is_ok(),
        "Should be able to insert user: {:?}",
        result.err()
    );
}

test_both_databases!(test_can_insert_user, can_insert_user_impl);

async fn can_insert_role_and_assign_to_user_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let backend = db.get_database_backend();

    // Insert user
    let user_sql = match backend {
        DbBackend::Sqlite => "INSERT INTO users (username, email, hashed_password, is_active, is_approved, totp_enabled, created_at, updated_at) VALUES ('testuser', 'test@example.com', 'hashed', 1, 1, 0, datetime('now'), datetime('now'))".to_string(),
        DbBackend::Postgres => "INSERT INTO users (username, email, hashed_password, is_active, is_approved, totp_enabled, created_at, updated_at) VALUES ('testuser', 'test@example.com', 'hashed', true, true, false, NOW(), NOW())".to_string(),
        _ => panic!("Unsupported database backend"),
    };
    db.execute(Statement::from_string(backend, user_sql))
        .await
        .expect("Failed to insert user");

    // Insert role (use unique name to avoid conflict with seeded data)
    let role_sql = match backend {
        DbBackend::Sqlite => "INSERT INTO roles (name, is_system, requires_2fa, created_at) VALUES ('test_role_unique', 0, 0, datetime('now'))".to_string(),
        DbBackend::Postgres => "INSERT INTO roles (name, is_system, requires_2fa, created_at) VALUES ('test_role_unique', false, false, NOW())".to_string(),
        _ => panic!("Unsupported database backend"),
    };
    db.execute(Statement::from_string(backend, role_sql))
        .await
        .expect("Failed to insert role");

    // Assign role to user (use subqueries to get correct IDs)
    let assign_sql = match backend {
        DbBackend::Sqlite => "INSERT INTO user_roles (user_id, role_id) SELECT u.id, r.id FROM users u, roles r WHERE u.username = 'testuser' AND r.name = 'test_role_unique'".to_string(),
        DbBackend::Postgres => "INSERT INTO user_roles (user_id, role_id) SELECT u.id, r.id FROM users u, roles r WHERE u.username = 'testuser' AND r.name = 'test_role_unique'".to_string(),
        _ => panic!("Unsupported database backend"),
    };
    let result = db
        .execute(Statement::from_string(backend, assign_sql))
        .await;

    assert!(
        result.is_ok(),
        "Should be able to assign role to user: {:?}",
        result.err()
    );
}

test_both_databases!(
    test_can_insert_role_and_assign_to_user,
    can_insert_role_and_assign_to_user_impl
);

// Note: oauth2 client and token insert test removed - tables no longer exist

async fn can_insert_notification_data_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let backend = db.get_database_backend();

    // Insert user
    let user_sql = match backend {
        DbBackend::Sqlite => "INSERT INTO users (username, email, hashed_password, is_active, is_approved, totp_enabled, created_at, updated_at) VALUES ('testuser', 'test@example.com', 'hashed', 1, 1, 0, datetime('now'), datetime('now'))".to_string(),
        DbBackend::Postgres => "INSERT INTO users (username, email, hashed_password, is_active, is_approved, totp_enabled, created_at, updated_at) VALUES ('testuser', 'test@example.com', 'hashed', true, true, false, NOW(), NOW())".to_string(),
        _ => panic!("Unsupported database backend"),
    };
    db.execute(Statement::from_string(backend, user_sql))
        .await
        .expect("Failed to insert user");

    // Insert notification channel
    let channel_sql = match backend {
        DbBackend::Sqlite => "INSERT INTO notification_channels (channel_type, enabled, config, created_at, updated_at) VALUES ('email', 1, '{}', datetime('now'), datetime('now'))".to_string(),
        DbBackend::Postgres => "INSERT INTO notification_channels (channel_type, enabled, config, created_at, updated_at) VALUES ('email', true, '{}', NOW(), NOW())".to_string(),
        _ => panic!("Unsupported database backend"),
    };
    db.execute(Statement::from_string(backend, channel_sql))
        .await
        .expect("Failed to insert notification channel");

    // Insert notification event
    let event_sql = match backend {
        DbBackend::Sqlite => "INSERT INTO notification_events (event_type, enabled, severity) VALUES ('user_login', 1, 'info')".to_string(),
        DbBackend::Postgres => "INSERT INTO notification_events (event_type, enabled, severity) VALUES ('user_login', true, 'info')".to_string(),
        _ => panic!("Unsupported database backend"),
    };
    db.execute(Statement::from_string(backend, event_sql))
        .await
        .expect("Failed to insert notification event");

    // Insert user notification (use subquery for user_id)
    let notif_sql = match backend {
        DbBackend::Sqlite => "INSERT INTO user_notifications (user_id, title, message, severity, read, created_at) SELECT id, 'Test', 'Test message', 'info', 0, datetime('now') FROM users WHERE username = 'testuser'".to_string(),
        DbBackend::Postgres => "INSERT INTO user_notifications (user_id, title, message, severity, read, created_at) SELECT id, 'Test', 'Test message', 'info', false, NOW() FROM users WHERE username = 'testuser'".to_string(),
        _ => panic!("Unsupported database backend"),
    };
    let result = db.execute(Statement::from_string(backend, notif_sql)).await;

    assert!(
        result.is_ok(),
        "Should be able to insert user notification: {:?}",
        result.err()
    );
}

test_both_databases!(
    test_can_insert_notification_data,
    can_insert_notification_data_impl
);

async fn can_insert_audit_log_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let backend = db.get_database_backend();
    let sql = match backend {
        DbBackend::Sqlite => "INSERT INTO audit_logs (timestamp, action, resource_type, success) VALUES (datetime('now'), 'login', 'user', 1)".to_string(),
        DbBackend::Postgres => "INSERT INTO audit_logs (timestamp, action, resource_type, success) VALUES (NOW(), 'login', 'user', true)".to_string(),
        _ => panic!("Unsupported database backend"),
    };

    let result = db.execute(Statement::from_string(backend, sql)).await;
    assert!(
        result.is_ok(),
        "Should be able to insert audit log: {:?}",
        result.err()
    );
}

test_both_databases!(test_can_insert_audit_log, can_insert_audit_log_impl);

async fn cascade_delete_user_removes_related_data_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let backend = db.get_database_backend();

    // Enable foreign keys for SQLite
    if backend == DbBackend::Sqlite {
        db.execute(Statement::from_string(
            backend,
            "PRAGMA foreign_keys = ON".to_string(),
        ))
        .await
        .expect("Failed to enable foreign keys");
    }

    // Insert user
    let user_sql = match backend {
        DbBackend::Sqlite => "INSERT INTO users (username, email, hashed_password, is_active, is_approved, totp_enabled, created_at, updated_at) VALUES ('testuser', 'test@example.com', 'hashed', 1, 1, 0, datetime('now'), datetime('now'))".to_string(),
        DbBackend::Postgres => "INSERT INTO users (username, email, hashed_password, is_active, is_approved, totp_enabled, created_at, updated_at) VALUES ('testuser', 'test@example.com', 'hashed', true, true, false, NOW(), NOW())".to_string(),
        _ => panic!("Unsupported database backend"),
    };
    db.execute(Statement::from_string(backend, user_sql))
        .await
        .expect("Failed to insert user");

    // Insert role (use unique name to avoid conflict with seeded data)
    let role_sql = match backend {
        DbBackend::Sqlite => "INSERT INTO roles (name, is_system, requires_2fa, created_at) VALUES ('test_role_unique', 0, 0, datetime('now'))".to_string(),
        DbBackend::Postgres => "INSERT INTO roles (name, is_system, requires_2fa, created_at) VALUES ('test_role_unique', false, false, NOW())".to_string(),
        _ => panic!("Unsupported database backend"),
    };
    db.execute(Statement::from_string(backend, role_sql))
        .await
        .expect("Failed to insert role");

    // Assign role to user (use subqueries to get correct IDs)
    let assign_sql = match backend {
        DbBackend::Sqlite => "INSERT INTO user_roles (user_id, role_id) SELECT u.id, r.id FROM users u, roles r WHERE u.username = 'testuser' AND r.name = 'test_role_unique'".to_string(),
        DbBackend::Postgres => "INSERT INTO user_roles (user_id, role_id) SELECT u.id, r.id FROM users u, roles r WHERE u.username = 'testuser' AND r.name = 'test_role_unique'".to_string(),
        _ => panic!("Unsupported database backend"),
    };
    db.execute(Statement::from_string(backend, assign_sql))
        .await
        .expect("Failed to assign role");

    // Insert user notification (use subquery for user_id)
    let notif_sql = match backend {
        DbBackend::Sqlite => "INSERT INTO user_notifications (user_id, title, message, severity, read, created_at) SELECT id, 'Test', 'Test message', 'info', 0, datetime('now') FROM users WHERE username = 'testuser'".to_string(),
        DbBackend::Postgres => "INSERT INTO user_notifications (user_id, title, message, severity, read, created_at) SELECT id, 'Test', 'Test message', 'info', false, NOW() FROM users WHERE username = 'testuser'".to_string(),
        _ => panic!("Unsupported database backend"),
    };
    db.execute(Statement::from_string(backend, notif_sql))
        .await
        .expect("Failed to insert notification");

    // Delete user
    db.execute(Statement::from_string(
        backend,
        "DELETE FROM users WHERE username = 'testuser'".to_string(),
    ))
    .await
    .expect("Failed to delete user");

    // Verify user_roles was cascaded (check for orphan records referencing non-existent users)
    let user_roles: Vec<QueryResult> = db
        .query_all(Statement::from_string(
            backend,
            "SELECT COUNT(*) as cnt FROM user_roles WHERE user_id NOT IN (SELECT id FROM users)"
                .to_string(),
        ))
        .await
        .expect("Failed to query user_roles");

    let count: i64 = user_roles[0].try_get("", "cnt").unwrap();
    assert_eq!(
        count, 0,
        "user_roles should be cascaded on user delete (no orphan records)"
    );

    // Verify user_notifications was cascaded (check for orphan records referencing non-existent users)
    let notifications: Vec<QueryResult> = db
        .query_all(Statement::from_string(
            backend,
            "SELECT COUNT(*) as cnt FROM user_notifications WHERE user_id NOT IN (SELECT id FROM users)".to_string(),
        ))
        .await
        .expect("Failed to query notifications");

    let count: i64 = notifications[0].try_get("", "cnt").unwrap();
    assert_eq!(
        count, 0,
        "user_notifications should be cascaded on user delete (no orphan records)"
    );
}

test_both_databases!(
    test_cascade_delete_user_removes_related_data,
    cascade_delete_user_removes_related_data_impl
);

// =============================================================================
// Migration Count Test
// =============================================================================

async fn migration_count_impl(db: &DatabaseConnection) {
    Migrator::up(db, None)
        .await
        .expect("Failed to apply migrations");

    let backend = db.get_database_backend();
    let result: Vec<QueryResult> = db
        .query_all(Statement::from_string(
            backend,
            "SELECT COUNT(*) as cnt FROM seaql_migrations".to_string(),
        ))
        .await
        .expect("Failed to query migrations");

    let count: i64 = result[0].try_get("", "cnt").unwrap();
    let expected_count = Migrator::migrations().len() as i64;
    assert_eq!(
        count, expected_count,
        "Should have exactly {expected_count} migrations applied"
    );
}

test_both_databases!(test_migration_count, migration_count_impl);
