//! Evidence capture for failed app updates: restart accounting and a
//! diagnostics report taken before Kubarr rolls the Helm release back.

use std::collections::HashMap;
use std::fmt::Write as _;

use k8s_openapi::api::core::v1::Pod;

use crate::error::Result;
use crate::services::deployment::DeploymentManager;
use crate::services::K8sClient;

/// Maximum size of a stored diagnostics report.
pub const MAX_DIAGNOSTICS_BYTES: usize = 64 * 1024;
const TRUNCATED_MARKER: &str = "\n[truncated]";
const EVENT_LIMIT: usize = 50;
const LOG_TAIL_LINES: i32 = 200;

/// `pod/container` -> restart count.
pub type RestartSnapshot = HashMap<String, i32>;

fn container_statuses(pod: &Pod) -> Vec<(String, i32)> {
    let Some(status) = pod.status.as_ref() else {
        return Vec::new();
    };
    status
        .init_container_statuses
        .iter()
        .flatten()
        .chain(status.container_statuses.iter().flatten())
        .map(|c| (c.name.clone(), c.restart_count))
        .collect()
}

fn snapshot_from_pods(pods: &[Pod]) -> RestartSnapshot {
    let mut snapshot = RestartSnapshot::new();
    for pod in pods {
        let pod_name = pod.metadata.name.clone().unwrap_or_default();
        for (container, restarts) in container_statuses(pod) {
            snapshot.insert(format!("{pod_name}/{container}"), restarts);
        }
    }
    snapshot
}

pub async fn restart_snapshot(k8s: &K8sClient, ns: &str) -> Result<RestartSnapshot> {
    Ok(snapshot_from_pods(&k8s.list_pods(ns, None).await?))
}

/// True when any container restarted more than `limit` times since `before`.
/// A container absent from `before` (new pod) counts from a baseline of 0.
pub fn restarts_exceeded(before: &RestartSnapshot, now: &RestartSnapshot, limit: i32) -> bool {
    now.iter()
        .any(|(key, count)| count - before.get(key).copied().unwrap_or(0) > limit)
}

fn truncate_report(mut report: String) -> String {
    if report.len() <= MAX_DIAGNOSTICS_BYTES {
        return report;
    }
    let mut end = MAX_DIAGNOSTICS_BYTES - TRUNCATED_MARKER.len();
    while !report.is_char_boundary(end) {
        end -= 1;
    }
    report.truncate(end);
    report.push_str(TRUNCATED_MARKER);
    report
}

fn pods_section(pods: &[Pod]) -> String {
    let mut out = String::from("NAME PHASE READY RESTARTS\n");
    for pod in pods {
        let name = pod.metadata.name.as_deref().unwrap_or("?");
        let phase = pod
            .status
            .as_ref()
            .and_then(|s| s.phase.as_deref())
            .unwrap_or("Unknown");
        let statuses = pod
            .status
            .as_ref()
            .and_then(|s| s.container_statuses.as_ref())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let ready = statuses.iter().filter(|c| c.ready).count();
        let restarts: i32 = statuses.iter().map(|c| c.restart_count).sum();
        let _ = writeln!(out, "{name} {phase} {ready}/{} {restarts}", statuses.len());
    }
    out
}

async fn events_section(k8s: &K8sClient, ns: &str) -> String {
    match k8s.list_events(ns, EVENT_LIMIT).await {
        Ok(events) => {
            let mut out = String::new();
            for e in events {
                let _ = writeln!(
                    out,
                    "{} {} {}/{}: {}",
                    e.type_.as_deref().unwrap_or("-"),
                    e.reason.as_deref().unwrap_or("-"),
                    e.involved_object.kind.as_deref().unwrap_or("-"),
                    e.involved_object.name.as_deref().unwrap_or("-"),
                    e.message.as_deref().unwrap_or("")
                );
            }
            out
        }
        Err(error) => format!("unavailable: {error}\n"),
    }
}

async fn logs_section(k8s: &K8sClient, ns: &str, pods: &[Pod]) -> String {
    let mut out = String::new();
    for pod in pods {
        let pod_name = pod.metadata.name.as_deref().unwrap_or_default();
        let statuses = pod
            .status
            .as_ref()
            .and_then(|s| s.container_statuses.as_ref())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for status in statuses {
            let container = status.name.as_str();
            let _ = writeln!(out, "--- {pod_name}/{container} ---");
            match k8s
                .get_pod_logs(pod_name, ns, Some(container), LOG_TAIL_LINES)
                .await
            {
                Ok(logs) => out.push_str(&logs),
                Err(error) => {
                    let _ = writeln!(out, "logs unavailable: {error}");
                }
            }
            if status.restart_count > 0 {
                let _ = writeln!(out, "\n--- {pod_name}/{container} (previous) ---");
                match k8s
                    .get_previous_pod_logs(pod_name, ns, container, LOG_TAIL_LINES)
                    .await
                {
                    Ok(logs) => out.push_str(&logs),
                    Err(error) => {
                        let _ = writeln!(out, "previous logs unavailable: {error}");
                    }
                }
            }
            out.push('\n');
        }
    }
    out
}

fn values_section(
    deploy: &DeploymentManager<'_>,
    app: &str,
    label: &str,
    rev: Option<u32>,
) -> String {
    match rev {
        None => format!("=== values ({label}): revision unknown ===\n"),
        Some(rev) => match deploy.values_at(app, rev) {
            Ok(values) => format!("=== values ({label}, revision {rev}) ===\n{values}\n"),
            Err(error) => {
                format!("=== values ({label}, revision {rev}) ===\nunavailable: {error}\n")
            }
        },
    }
}

/// Build the diagnostics report. Never fails: a section that cannot be
/// gathered says so inline. Output is capped at 64 KiB.
pub async fn capture(
    k8s: &K8sClient,
    deploy: &DeploymentManager<'_>,
    app: &str,
    ns: &str,
    failed_rev: Option<u32>,
    previous_rev: Option<u32>,
) -> String {
    let mut report = String::new();

    let pods = k8s.list_pods(ns, None).await;
    report.push_str("=== pods ===\n");
    match &pods {
        Ok(pods) => report.push_str(&pods_section(pods)),
        Err(error) => {
            let _ = writeln!(report, "unavailable: {error}");
        }
    }

    report.push_str("\n=== events (last 50) ===\n");
    report.push_str(&events_section(k8s, ns).await);

    report.push_str("\n=== logs (last 200 lines per container) ===\n");
    match &pods {
        Ok(pods) => report.push_str(&logs_section(k8s, ns, pods).await),
        Err(_) => report.push_str("unavailable: pod list failed\n"),
    }

    report.push('\n');
    report.push_str(&values_section(deploy, app, "failed", failed_rev));
    report.push_str(&values_section(deploy, app, "previous", previous_rev));

    truncate_report(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(entries: &[(&str, i32)]) -> RestartSnapshot {
        entries.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn test_restarts_exceeded() {
        let before = snap(&[("a/app", 1)]);
        assert!(!restarts_exceeded(&before, &snap(&[("a/app", 3)]), 2));
        assert!(restarts_exceeded(&before, &snap(&[("a/app", 4)]), 2));
        // New pod counts from a baseline of 0.
        assert!(!restarts_exceeded(&before, &snap(&[("b/app", 2)]), 2));
        assert!(restarts_exceeded(&before, &snap(&[("b/app", 3)]), 2));
    }

    #[test]
    fn test_capture_truncates_to_64k() {
        let big = "é".repeat(MAX_DIAGNOSTICS_BYTES);
        let out = truncate_report(big);
        assert!(out.len() <= MAX_DIAGNOSTICS_BYTES);
        assert!(out.ends_with("[truncated]"));
        let small = truncate_report("ok".into());
        assert_eq!(small, "ok");
    }
}
