//! The pod status cache is a disposable mirror of what the worker's Kubernetes
//! watch has seen. Every test here pins one half of that contract: what a
//! reader gets when the mirror is trustworthy, and what it gets when it isn't.
use kubarr::services::pod_cache::{self, CachedPod};
use kubarr::services::k8s::PodStatus;
use std::time::Duration;

async fn db() -> sea_orm::DatabaseConnection {
    kubarr::application::database::connect_with_url("sqlite::memory:")
        .await
        .expect("connect")
}

fn pod(name: &str, namespace: &str, created_unix: Option<i64>) -> CachedPod {
    CachedPod {
        pod: PodStatus {
            name: name.to_string(),
            app: namespace.to_string(),
            namespace: namespace.to_string(),
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
        created_unix,
    }
}

#[tokio::test]
async fn returns_stored_pods_for_a_fresh_stream() {
    let db = db().await;
    pod_cache::store_namespace(&db, "sonarr", &[pod("sonarr-0", "sonarr", None)])
        .await
        .expect("store");
    pod_cache::touch_heartbeat(&db).await.expect("heartbeat");

    let fresh = pod_cache::fresh_namespaces(&db, &["sonarr".to_string()], Duration::from_secs(30))
        .await;

    assert_eq!(fresh.get("sonarr").map(|p| p.len()), Some(1));
    assert_eq!(fresh["sonarr"][0].name, "sonarr-0");
}

#[tokio::test]
async fn recomputes_age_so_it_does_not_freeze_between_events() {
    let db = db().await;
    // A pod that started two hours ago. The watch last wrote this row when the
    // pod was seconds old, so the stored age string is stale by design.
    let two_hours_ago = jiff::Timestamp::now().as_second() - 7200;
    pod_cache::store_namespace(
        &db,
        "radarr",
        &[pod("radarr-0", "radarr", Some(two_hours_ago))],
    )
    .await
    .expect("store");
    pod_cache::touch_heartbeat(&db).await.expect("heartbeat");

    let fresh = pod_cache::fresh_namespaces(&db, &["radarr".to_string()], Duration::from_secs(30))
        .await;

    assert_eq!(fresh["radarr"][0].age, "2h");
}

#[tokio::test]
async fn reports_nothing_when_the_watch_has_gone_quiet() {
    let db = db().await;
    pod_cache::store_namespace(&db, "sonarr", &[pod("sonarr-0", "sonarr", None)])
        .await
        .expect("store");
    // Heartbeat from well outside the freshness window: the worker may be dead
    // and these rows may describe a cluster that has since moved on.
    pod_cache::set_heartbeat_for_test(&db, jiff::Timestamp::now().as_second() - 600)
        .await
        .expect("heartbeat");

    let fresh = pod_cache::fresh_namespaces(&db, &["sonarr".to_string()], Duration::from_secs(30))
        .await;

    assert!(fresh.is_empty(), "stale stream must force a live read");
}

#[tokio::test]
async fn reports_nothing_before_the_watch_has_ever_synced() {
    let db = db().await;

    let fresh = pod_cache::fresh_namespaces(&db, &["sonarr".to_string()], Duration::from_secs(30))
        .await;

    assert!(fresh.is_empty());
}

#[tokio::test]
async fn omits_namespaces_the_watch_has_not_seen() {
    let db = db().await;
    pod_cache::store_namespace(&db, "sonarr", &[pod("sonarr-0", "sonarr", None)])
        .await
        .expect("store");
    pod_cache::touch_heartbeat(&db).await.expect("heartbeat");

    let fresh = pod_cache::fresh_namespaces(
        &db,
        &["sonarr".to_string(), "bazarr".to_string()],
        Duration::from_secs(30),
    )
    .await;

    // bazarr is absent rather than empty, so the caller knows to ask Kubernetes
    // instead of reporting an app with no pods.
    assert!(fresh.contains_key("sonarr"));
    assert!(!fresh.contains_key("bazarr"));
}

#[tokio::test]
async fn an_emptied_namespace_reads_as_empty_not_missing() {
    let db = db().await;
    pod_cache::store_namespace(&db, "sonarr", &[pod("sonarr-0", "sonarr", None)])
        .await
        .expect("store");
    // The app was uninstalled: the watch saw the last pod go and recorded that.
    pod_cache::store_namespace(&db, "sonarr", &[]).await.expect("store");
    pod_cache::touch_heartbeat(&db).await.expect("heartbeat");

    let fresh = pod_cache::fresh_namespaces(&db, &["sonarr".to_string()], Duration::from_secs(30))
        .await;

    assert_eq!(fresh.get("sonarr").map(|p| p.len()), Some(0));
}
