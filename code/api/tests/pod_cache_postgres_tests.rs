//! PostgreSQL-only checks for the pod status cache.
//!
//! SQLite accepts the schema and upsert used by `pod_cache` regardless of
//! whether PostgreSQL would, so the UNLOGGED table and the `ON CONFLICT` write
//! are verified against a real server here. Opt in with a fresh, disposable
//! local database via POD_CACHE_TEST_POSTGRES_URL.
use kubarr::migrations::Migrator;
use kubarr::services::k8s::PodStatus;
use kubarr::services::pod_cache::{self, CachedPod};
use sea_orm::{ConnectionTrait, Database, DbBackend, FromQueryResult, Statement};
use sea_orm_migration::MigratorTrait;
use std::time::Duration;

#[derive(FromQueryResult)]
struct Persistence {
    relpersistence: String,
}

fn pod(name: &str) -> CachedPod {
    CachedPod {
        pod: PodStatus {
            name: name.to_string(),
            app: "sonarr".to_string(),
            namespace: "sonarr".to_string(),
            status: "Running".to_string(),
            ready: true,
            restart_count: 0,
            containers: Vec::new(),
            age: "0s".to_string(),
            node: None,
            ip: None,
            cpu_usage: None,
            memory_usage: None,
        },
        created_unix: None,
    }
}

#[tokio::test]
async fn postgres_pod_cache_is_unlogged_and_upserts() {
    let Ok(url) = std::env::var("POD_CACHE_TEST_POSTGRES_URL") else {
        eprintln!("POD_CACHE_TEST_POSTGRES_URL unset; skipping PostgreSQL pod cache test");
        return;
    };
    // A dedicated loopback test database only; never an arbitrary host and
    // never the application's DATABASE_URL.
    assert!(
        url.contains("@127.0.0.1:") || url.contains("@localhost:"),
        "refusing to migrate a non-local database"
    );

    let db = Database::connect(&url).await.expect("connect");
    Migrator::up(&db, None).await.expect("migrate");

    let persistence = Persistence::find_by_statement(Statement::from_string(
        DbBackend::Postgres,
        "SELECT relpersistence::text FROM pg_class WHERE relname = 'pod_status_cache'",
    ))
    .one(&db)
    .await
    .expect("query")
    .expect("table exists");
    assert_eq!(persistence.relpersistence, "u", "table must be UNLOGGED");

    pod_cache::store_namespace(&db, "sonarr", &[pod("sonarr-0")])
        .await
        .expect("insert");
    // Second write takes the ON CONFLICT path, which is the one that only ever
    // ran against SQLite before.
    pod_cache::store_namespace(&db, "sonarr", &[pod("sonarr-0"), pod("sonarr-1")])
        .await
        .expect("upsert");
    pod_cache::touch_heartbeat(&db).await.expect("heartbeat");
    pod_cache::touch_heartbeat(&db).await.expect("heartbeat upsert");

    let fresh =
        pod_cache::fresh_namespaces(&db, &["sonarr".to_string()], Duration::from_secs(30)).await;
    assert_eq!(fresh["sonarr"].len(), 2);

    pod_cache::invalidate(&db, "sonarr").await.expect("invalidate");
    let after =
        pod_cache::fresh_namespaces(&db, &["sonarr".to_string()], Duration::from_secs(30)).await;
    assert!(!after.contains_key("sonarr"));

    db.execute_unprepared("DROP TABLE IF EXISTS pod_status_cache, pod_watch_heartbeat")
        .await
        .expect("cleanup");
}
