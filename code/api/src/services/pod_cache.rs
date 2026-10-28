//! Storage for pod status observed by the worker's Kubernetes watch.
//!
//! The rows this module writes are a mirror, never a source of truth. Readers
//! use them only while the watch is demonstrably alive, and treat an absent
//! namespace as "ask Kubernetes" rather than "this app has no pods" — see
//! [`fresh_namespaces`].
use std::collections::HashMap;
use std::time::Duration;

use sea_orm::{ConnectionTrait, DbConn, FromQueryResult, Statement, Value};

use crate::error::Result;
use crate::services::k8s::{format_age, PodStatus};

/// A pod as the watch saw it, plus the start time needed to keep its age honest.
///
/// `PodStatus::age` is a formatted string computed when the pod was observed. A
/// watch only writes on change, so a long-running pod's row could sit untouched
/// for days; storing the creation time lets readers recompute the age.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedPod {
    pub pod: PodStatus,
    pub created_unix: Option<i64>,
}

/// Replace a namespace's recorded pods.
///
/// Called with an empty slice when a namespace's last pod goes away, which is
/// what lets readers distinguish "empty" from "never observed".
pub async fn store_namespace(db: &DbConn, namespace: &str, pods: &[CachedPod]) -> Result<()> {
    let payload = serde_json::to_string(pods)
        .map_err(|e| crate::error::AppError::Internal(format!("encode pod cache: {e}")))?;

    db.execute_raw(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO pod_status_cache (namespace, payload, observed_unix)
         VALUES ($1, $2, $3)
         ON CONFLICT (namespace) DO UPDATE
         SET payload = $2, observed_unix = $3",
        [
            namespace.into(),
            payload.into(),
            Value::BigInt(Some(jiff::Timestamp::now().as_second())),
        ],
    ))
    .await?;

    Ok(())
}

/// Record that the watch is alive and current.
///
/// Namespaces with no pods never update their own row, and a quiet cluster
/// produces no events at all, so stream health is tracked separately from the
/// data it produces.
pub async fn touch_heartbeat(db: &DbConn) -> Result<()> {
    set_heartbeat(db, jiff::Timestamp::now().as_second()).await
}

async fn set_heartbeat(db: &DbConn, synced_unix: i64) -> Result<()> {
    db.execute_raw(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO pod_watch_heartbeat (id, synced_unix)
         VALUES (1, $1)
         ON CONFLICT (id) DO UPDATE SET synced_unix = $1",
        [Value::BigInt(Some(synced_unix))],
    ))
    .await?;
    Ok(())
}

/// Backdate the heartbeat so tests can exercise the stale-stream path.
pub async fn set_heartbeat_for_test(db: &DbConn, synced_unix: i64) -> Result<()> {
    set_heartbeat(db, synced_unix).await
}

/// Forget a namespace's rows so the next read goes to Kubernetes.
///
/// Used after an action that changes pods on purpose — install, restart,
/// delete — so the dashboard never shows the pre-action state back to the
/// person who just triggered it.
pub async fn invalidate(db: &DbConn, namespace: &str) -> Result<()> {
    db.execute_raw(Statement::from_sql_and_values(
        db.get_database_backend(),
        "DELETE FROM pod_status_cache WHERE namespace = $1",
        [namespace.into()],
    ))
    .await?;
    Ok(())
}

#[derive(FromQueryResult)]
struct HeartbeatRow {
    synced_unix: i64,
}

#[derive(FromQueryResult)]
struct CacheRow {
    namespace: String,
    payload: String,
}

/// Pods for the requested namespaces, as far as the cache can be trusted.
///
/// Returns an empty map when the watch has never synced or has been silent for
/// longer than `max_silence`; a namespace missing from the result has not been
/// observed. In both cases the caller must fall back to a live read.
pub async fn fresh_namespaces(
    db: &DbConn,
    namespaces: &[String],
    max_silence: Duration,
) -> HashMap<String, Vec<PodStatus>> {
    if namespaces.is_empty() {
        return HashMap::new();
    }

    match heartbeat_age(db).await {
        Some(age) if age <= max_silence.as_secs() as i64 => {}
        // No heartbeat, or one too old to vouch for the rows beneath it. This
        // also covers PostgreSQL truncating these unlogged tables on restart.
        _ => return HashMap::new(),
    }

    let placeholders = (1..=namespaces.len())
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let rows = CacheRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        format!(
            "SELECT namespace, payload FROM pod_status_cache WHERE namespace IN ({placeholders})"
        ),
        namespaces.iter().map(Value::from).collect::<Vec<_>>(),
    ))
    .all(db)
    .await;

    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "pod cache unreadable, falling back to live reads");
            return HashMap::new();
        }
    };

    let now = jiff::Timestamp::now().as_second();
    let mut by_namespace = HashMap::new();
    for row in rows {
        let cached: Vec<CachedPod> = match serde_json::from_str(&row.payload) {
            Ok(pods) => pods,
            Err(error) => {
                // A payload written by an older build: skip it rather than
                // serving half a namespace.
                tracing::warn!(%error, namespace = %row.namespace, "undecodable pod cache row");
                continue;
            }
        };
        by_namespace.insert(
            row.namespace,
            cached
                .into_iter()
                .map(|entry| {
                    let mut pod = entry.pod;
                    if let Some(created) = entry.created_unix {
                        pod.age = format_age(now - created);
                    }
                    pod
                })
                .collect(),
        );
    }

    by_namespace
}

/// Seconds since the watch last reported in, or `None` if it never has.
pub async fn heartbeat_age(db: &DbConn) -> Option<i64> {
    let row = HeartbeatRow::find_by_statement(Statement::from_string(
        db.get_database_backend(),
        "SELECT synced_unix FROM pod_watch_heartbeat WHERE id = 1",
    ))
    .one(db)
    .await
    .ok()
    .flatten()?;

    Some(jiff::Timestamp::now().as_second() - row.synced_unix)
}
