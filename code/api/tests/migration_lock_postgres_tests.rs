//! PostgreSQL-only check that concurrent backends can migrate one database.
//!
//! Opt in with a fresh, disposable local database via
//! MIGRATION_LOCK_TEST_POSTGRES_URL.
use kubarr::application::database::connect_with_url;

#[tokio::test]
async fn concurrent_connects_both_migrate_cleanly() {
    let Ok(url) = std::env::var("MIGRATION_LOCK_TEST_POSTGRES_URL") else {
        eprintln!("MIGRATION_LOCK_TEST_POSTGRES_URL not set; skipping");
        return;
    };

    let (a, b) = tokio::join!(connect_with_url(&url), connect_with_url(&url));
    assert!(a.is_ok(), "first connect failed: {:?}", a.err());
    assert!(b.is_ok(), "second connect failed: {:?}", b.err());
}
