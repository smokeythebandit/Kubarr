//! Evidence capture for failed app updates: restart accounting and a
//! diagnostics report taken before Kubarr rolls the Helm release back.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::api::{Api, ListParams};
use regex::Regex;

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

pub(crate) fn truncate_report(mut report: String) -> String {
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

const REDACTED: &str = "[redacted]";
const MIN_SECRET_LEN: usize = 6;

fn sensitive_key(key: &str) -> bool {
    let key = key.to_lowercase();
    ["pass", "secret", "token", "key", "credential", "auth"]
        .iter()
        .any(|needle| key.contains(needle))
}

fn flatten_yaml(
    value: &serde_yaml::Value,
    path: &str,
    sensitive: bool,
    out: &mut BTreeMap<String, (String, bool)>,
) {
    use serde_yaml::Value;
    match value {
        Value::Mapping(map) => {
            for (key, child) in map {
                let name = match key {
                    Value::String(s) => s.clone(),
                    other => serde_yaml::to_string(other)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                };
                let child_path = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}.{name}")
                };
                flatten_yaml(child, &child_path, sensitive || sensitive_key(&name), out);
            }
        }
        Value::Sequence(items) => {
            for (i, child) in items.iter().enumerate() {
                flatten_yaml(child, &format!("{path}[{i}]"), sensitive, out);
            }
        }
        Value::Null => {}
        Value::String(s) => {
            out.insert(path.to_string(), (s.clone(), sensitive));
        }
        scalar => {
            let text = serde_yaml::to_string(scalar)
                .unwrap_or_default()
                .trim()
                .to_string();
            out.insert(path.to_string(), (text, sensitive));
        }
    }
}

fn flatten_values(yaml: &str) -> std::result::Result<BTreeMap<String, (String, bool)>, String> {
    let value: serde_yaml::Value = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
    let mut out = BTreeMap::new();
    flatten_yaml(&value, "", false, &mut out);
    Ok(out)
}

/// Diff of redacted values between the previous and failed revision, plus the
/// raw secret values that were redacted (for scrubbing the rest of the report).
fn values_diff(previous: Option<&str>, failed: Option<&str>) -> (String, Vec<String>) {
    let mut out = String::from("=== values diff (previous -> failed) ===\n");
    let (Some(previous), Some(failed)) = (previous, failed) else {
        out.push_str("unavailable: revision values could not be read\n");
        return (out, Vec::new());
    };
    let (prev, fail) = match (flatten_values(previous), flatten_values(failed)) {
        (Ok(p), Ok(f)) => (p, f),
        (Err(e), _) | (_, Err(e)) => {
            let _ = writeln!(out, "unavailable: cannot parse values: {e}");
            return (out, Vec::new());
        }
    };
    let mut secrets = Vec::new();
    for (value, sensitive) in prev.values().chain(fail.values()) {
        if *sensitive && value.len() >= MIN_SECRET_LEN {
            secrets.push(value.clone());
        }
    }
    let show = |entry: &(String, bool)| {
        if entry.1 {
            REDACTED.to_string()
        } else {
            entry.0.clone()
        }
    };
    let keys: BTreeSet<_> = prev.keys().chain(fail.keys()).collect();
    let mut changed = false;
    for key in keys {
        match (prev.get(key), fail.get(key)) {
            (Some(p), Some(f)) if p.0 != f.0 => {
                changed = true;
                let _ = writeln!(out, "~ {key}: {} -> {}", show(p), show(f));
            }
            (None, Some(f)) => {
                changed = true;
                let _ = writeln!(out, "+ {key}: {}", show(f));
            }
            (Some(p), None) => {
                changed = true;
                let _ = writeln!(out, "- {key}: {}", show(p));
            }
            _ => {}
        }
    }
    if !changed {
        out.push_str("no differences\n");
    }
    (out, secrets)
}

/// Replace known secret values, then mask common inline secret patterns.
fn scrub(text: &str, secrets: &[String]) -> String {
    let mut sorted: Vec<&String> = secrets
        .iter()
        .filter(|s| s.len() >= MIN_SECRET_LEN)
        .collect();
    sorted.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let mut out = text.to_string();
    for secret in sorted {
        out = out.replace(secret.as_str(), REDACTED);
    }
    let inline =
        Regex::new(r#"(?i)(password|passwd|token|apikey|api_key|secret)("?\s*[=:]\s*)\S+"#);
    if let Ok(re) = inline {
        out = re.replace_all(&out, "${1}${2}[redacted]").into_owned();
    }
    if let Ok(re) = Regex::new(r"Bearer\s+\S+") {
        out = re.replace_all(&out, "Bearer [redacted]").into_owned();
    }
    // Credentials embedded in URLs: scheme://user:pass@host
    if let Ok(re) = Regex::new(r"([A-Za-z][A-Za-z0-9+.-]*://)[^\s/@:]+:[^\s/@]+@") {
        out = re.replace_all(&out, "${1}[redacted]@").into_owned();
    }
    out
}

/// Scrub text that did not come from `capture` (e.g. Helm error lines) with
/// the namespace's Secret values and the inline patterns.
pub async fn scrub_for_namespace(k8s: &K8sClient, ns: &str, text: &str) -> String {
    let secrets = namespace_secret_values(k8s, ns).await.unwrap_or_default();
    scrub(text, &secrets)
}

/// Inline-pattern scrub only, for callers without cluster access.
pub(crate) fn scrub_inline(text: &str) -> String {
    scrub(text, &[])
}

/// Decoded Secret values (>= 6 chars) in the namespace. `None` when listing fails.
async fn namespace_secret_values(k8s: &K8sClient, ns: &str) -> Option<Vec<String>> {
    let api: Api<Secret> = Api::namespaced(k8s.client().clone(), ns);
    let list = api.list(&ListParams::default()).await.ok()?;
    let mut values = Vec::new();
    for secret in list.items {
        for bytes in secret.data.iter().flat_map(|d| d.values()) {
            if let Ok(text) = String::from_utf8(bytes.0.clone()) {
                let text = text.trim().to_string();
                if text.len() >= MIN_SECRET_LEN {
                    values.push(text);
                }
            }
        }
    }
    Some(values)
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
    let read = |rev: Option<u32>| rev.and_then(|r| deploy.values_at(app, r).ok());
    let (diff, mut secrets) =
        values_diff(read(previous_rev).as_deref(), read(failed_rev).as_deref());
    report.push_str(&diff);

    match namespace_secret_values(k8s, ns).await {
        Some(values) => secrets.extend(values),
        None => report.push_str("\nsecret scrub: limited\n"),
    }

    truncate_report(scrub(&report, &secrets))
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

    #[test]
    fn test_values_redacted_and_diffed() {
        let prev = "image:\n  tag: 1.0\napiKey: oldsecretvalue\nauth:\n  user: bob\nsame: x\n";
        let fail =
            "image:\n  tag: 2.0\napiKey: newsecretvalue\nauth:\n  user: bob\nsame: x\nextra: 1\n";
        let (diff, secrets) = values_diff(Some(prev), Some(fail));
        assert!(diff.contains("~ image.tag: 1.0 -> 2.0"));
        assert!(diff.contains("~ apiKey: [redacted] -> [redacted]"));
        assert!(diff.contains("+ extra: 1"));
        assert!(!diff.contains("secretvalue"));
        assert!(!diff.contains("same"));
        assert!(!diff.contains("bob -> "));
        assert!(secrets.contains(&"oldsecretvalue".to_string()));
        assert!(secrets.contains(&"newsecretvalue".to_string()));
        assert!(values_diff(None, Some(fail)).0.contains("unavailable"));
    }

    #[test]
    fn test_logs_scrub_known_secret_values() {
        let out = scrub(
            "connecting with hunter2hunter2 ok; short abc",
            &["hunter2hunter2".to_string(), "abc".to_string()],
        );
        assert_eq!(out, "connecting with [redacted] ok; short abc");
    }

    #[test]
    fn test_inline_secret_patterns_masked() {
        let out = scrub(
            "password=abc123 token: xyz api_key=k1 Authorization: Bearer eyJ.a.b {\"secret\":\"s3\"}",
            &[],
        );
        assert!(out.contains("password=[redacted]"));
        assert!(out.contains("token: [redacted]"));
        assert!(out.contains("api_key=[redacted]"));
        assert!(out.contains("Bearer [redacted]"));
        assert!(!out.contains("abc123") && !out.contains("eyJ") && !out.contains("s3"));
    }

    #[test]
    fn test_url_credentials_masked() {
        let out = scrub(
            "pulling https://bot:Sup3rS3cret@registry.example/chart and postgres://app:pw@db:5432/x; plain https://example.com/a@b",
            &[],
        );
        assert!(
            out.contains("https://[redacted]@registry.example/chart"),
            "{out}"
        );
        assert!(out.contains("postgres://[redacted]@db:5432/x"), "{out}");
        assert!(out.contains("https://example.com/a@b"), "{out}");
        assert!(!out.contains("Sup3rS3cret") && !out.contains("app:pw"));
    }
}
