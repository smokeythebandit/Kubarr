//! Shaping Kubernetes objects into compact, redacted summaries for the
//! assistant.
//!
//! Everything here is a pure function over `k8s-openapi` types (or the JSON
//! of a dynamic object), so the redaction rules are tested without a cluster.
//! Shaping is a whitelist: only the fields named here are copied, so
//! `managedFields`, Secret data, env var values, `valueFrom` targets, command
//! lines and anything else not listed never reach the model. On top of that
//! the `kubectl.kubernetes.io/last-applied-configuration` annotation is
//! dropped and labels/annotations with secret-ish keys are redacted.

use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{
    ConfigMap, Container, ContainerState, ContainerStatus, Event, Namespace, Node,
    PersistentVolume, PersistentVolumeClaim, Pod, PodSpec, PodStatus, PodTemplateSpec, Probe,
    Service, Volume,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta, Time};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use serde::Serialize;
use serde_json::{json, Map, Value};

use super::tools::clip;
use crate::services::update_diagnostics::{scrub_inline, scrub_with};

pub const REDACTED: &str = "[redacted]";
/// Why Secrets are refused (the model sees this verbatim).
pub const SECRET_REFUSAL: &str = "Secret objects are never available to the assistant: they \
     are not listed or described, and their contents are never shown. Pod and workload \
     descriptions (k8s_describe) do show which Secrets they reference, by name only.";
pub const OMITTED: &str = "[omitted]";
/// A ConfigMap list line names at most this many keys.
const MAX_LINE_KEYS: usize = 20;
const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";
/// Free-text fields (condition and event messages, annotation values).
const MAX_TEXT_CHARS: usize = 500;
/// One log line is cut to this many characters (after scrubbing).
pub const MAX_LOG_LINE_CHARS: usize = 1_000;
/// Logs keep the newest lines that fit in this many bytes, so the tool
/// result stays under the 20 KB cap without cutting off the newest lines.
pub const LOG_BUDGET_BYTES: usize = 18 * 1024;
pub const LOGS_OMITTED_MARKER: &str = "[earlier lines omitted: the log was longer than 18 KB]";

/// The kinds the cluster tools read. Secrets are deliberately absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pod,
    Deployment,
    StatefulSet,
    DaemonSet,
    Job,
    CronJob,
    Service,
    HttpRoute,
    PersistentVolumeClaim,
    PersistentVolume,
    Node,
    Event,
    Namespace,
    ConfigMap,
}

impl Kind {
    pub const ALL: [Kind; 14] = [
        Kind::Pod,
        Kind::Deployment,
        Kind::StatefulSet,
        Kind::DaemonSet,
        Kind::Job,
        Kind::CronJob,
        Kind::Service,
        Kind::HttpRoute,
        Kind::PersistentVolumeClaim,
        Kind::PersistentVolume,
        Kind::Node,
        Kind::Event,
        Kind::Namespace,
        Kind::ConfigMap,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Pod => "Pod",
            Kind::Deployment => "Deployment",
            Kind::StatefulSet => "StatefulSet",
            Kind::DaemonSet => "DaemonSet",
            Kind::Job => "Job",
            Kind::CronJob => "CronJob",
            Kind::Service => "Service",
            Kind::HttpRoute => "HTTPRoute",
            Kind::PersistentVolumeClaim => "PersistentVolumeClaim",
            Kind::PersistentVolume => "PersistentVolume",
            Kind::Node => "Node",
            Kind::Event => "Event",
            Kind::Namespace => "Namespace",
            Kind::ConfigMap => "ConfigMap",
        }
    }

    /// For status lines: "pods", "HTTP routes".
    pub fn plural_label(self) -> &'static str {
        match self {
            Kind::Pod => "pods",
            Kind::Deployment => "deployments",
            Kind::StatefulSet => "stateful sets",
            Kind::DaemonSet => "daemon sets",
            Kind::Job => "jobs",
            Kind::CronJob => "cron jobs",
            Kind::Service => "services",
            Kind::HttpRoute => "HTTP routes",
            Kind::PersistentVolumeClaim => "volume claims",
            Kind::PersistentVolume => "persistent volumes",
            Kind::Node => "nodes",
            Kind::Event => "events",
            Kind::Namespace => "namespaces",
            Kind::ConfigMap => "config maps",
        }
    }

    /// Parse a kind name. Secrets are refused with [`SECRET_REFUSAL`].
    pub fn parse(name: &str) -> Result<Kind, String> {
        let lower = name.trim().to_ascii_lowercase();
        if matches!(lower.as_str(), "secret" | "secrets") {
            return Err(SECRET_REFUSAL.to_string());
        }
        Kind::ALL
            .into_iter()
            .find(|kind| kind.name().eq_ignore_ascii_case(&lower))
            .ok_or_else(|| format!("Unsupported kind `{}`.", clip(name, 40)))
    }

    /// Cluster-scoped kinds have no namespace.
    pub fn cluster_scoped(self) -> bool {
        matches!(self, Kind::PersistentVolume | Kind::Node | Kind::Namespace)
    }

    /// Only administrators may read these at all. (Namespaces are special:
    /// non-admins see the namespaces they are allowed to see.)
    pub fn admin_only(self) -> bool {
        matches!(self, Kind::PersistentVolume | Kind::Node)
    }
}

/// Whether a label/annotation key looks like it could carry a credential.
pub fn secretish_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "token",
        "secret",
        "pass",
        "key",
        "credential",
        "auth",
        "cert",
        "dsn",
        "private",
        "connection",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

fn text(value: Option<&str>) -> Value {
    value
        .filter(|v| !v.is_empty())
        .map(|v| Value::String(clip(&scrub_inline(v), MAX_TEXT_CHARS)))
        .unwrap_or(Value::Null)
}

fn time(t: Option<&Time>) -> Value {
    t.map(|t| Value::String(t.0.to_string()))
        .unwrap_or(Value::Null)
}

fn seconds(t: Option<&Time>) -> Option<i64> {
    t.map(|t| t.0.as_second())
}

/// "3d", "5h", "12m", "40s".
pub fn age(since: Option<i64>, now: i64) -> String {
    let Some(since) = since else {
        return "?".to_string();
    };
    let s = (now - since).max(0);
    match s {
        s if s >= 2 * 86_400 => format!("{}d", s / 86_400),
        s if s >= 2 * 3_600 => format!("{}h", s / 3_600),
        s if s >= 120 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

fn quantities(map: Option<&BTreeMap<String, Quantity>>) -> Value {
    match map {
        Some(map) if !map.is_empty() => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), Value::String(v.0.clone())))
                .collect(),
        ),
        _ => Value::Null,
    }
}

fn int_or_string(value: &IntOrString) -> String {
    match value {
        IntOrString::Int(i) => i.to_string(),
        IntOrString::String(s) => s.clone(),
    }
}

/// Drop nulls, empty arrays and empty objects so summaries stay compact.
fn compact(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, compact(v)))
                .filter(|(_, v)| match v {
                    Value::Null => false,
                    Value::Array(a) => !a.is_empty(),
                    Value::Object(o) => !o.is_empty(),
                    _ => true,
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(compact).collect()),
        other => other,
    }
}

/// A value that looks like an embedded JSON/YAML document.
fn blob(value: &str) -> bool {
    let trimmed = value.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[') || value.contains('\n')
}

/// Labels or annotations with secret-ish keys redacted, JSON/YAML blobs
/// omitted, the last-applied
/// configuration dropped and long values clipped.
pub fn shape_string_map(map: Option<&BTreeMap<String, String>>) -> Value {
    let Some(map) = map else {
        return Value::Null;
    };
    let out: Map<String, Value> = map
        .iter()
        .filter(|(k, _)| k.as_str() != LAST_APPLIED && !k.contains("last-applied"))
        .map(|(k, v)| {
            let value = if secretish_key(k) {
                REDACTED.to_string()
            } else if blob(v) {
                OMITTED.to_string()
            } else {
                clip(&scrub_inline(v), 300)
            };
            (k.clone(), Value::String(value))
        })
        .collect();
    Value::Object(out)
}

/// Object metadata: name, namespace, creation time, owners, labels and
/// annotations (redacted). Never `managedFields`.
pub fn shape_meta(meta: &ObjectMeta) -> Value {
    let owners: Vec<String> = meta
        .owner_references
        .iter()
        .flatten()
        .map(|o| format!("{}/{}", o.kind, o.name))
        .collect();
    json!({
        "name": meta.name,
        "namespace": meta.namespace,
        "created": time(meta.creation_timestamp.as_ref()),
        "deleting_since": time(meta.deletion_timestamp.as_ref()),
        "owners": owners,
        "labels": shape_string_map(meta.labels.as_ref()),
        "annotations": shape_string_map(meta.annotations.as_ref()),
    })
}

/// Conditions of any kind: type, status, reason, scrubbed message, time.
fn conditions<T: Serialize>(items: Option<&Vec<T>>) -> Value {
    let list: Vec<Value> = items
        .into_iter()
        .flatten()
        .filter_map(|c| serde_json::to_value(c).ok())
        .map(|c| {
            json!({
                "type": c["type"],
                "status": c["status"],
                "reason": c["reason"],
                "message": text(c["message"].as_str()),
                "since": c["lastTransitionTime"],
            })
        })
        .collect();
    Value::Array(list)
}

fn selector(selector: Option<&LabelSelector>) -> Value {
    let Some(selector) = selector else {
        return Value::Null;
    };
    let mut parts: Vec<String> = selector
        .match_labels
        .iter()
        .flatten()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    for expr in selector.match_expressions.iter().flatten() {
        let values = expr.values.clone().unwrap_or_default().join(",");
        parts.push(format!("{} {} ({values})", expr.key, expr.operator));
    }
    Value::String(parts.join(", "))
}

fn env_names(container: &Container) -> Vec<String> {
    container
        .env
        .iter()
        .flatten()
        .map(|env| {
            let from = env.value_from.as_ref().map(|from| {
                if from.secret_key_ref.is_some() {
                    "secret"
                } else if from.config_map_key_ref.is_some() {
                    "config map"
                } else if from.field_ref.is_some() {
                    "pod field"
                } else if from.resource_field_ref.is_some() {
                    "resource"
                } else {
                    "reference"
                }
            });
            match from {
                Some(from) => format!("{} (from {from})", env.name),
                None => env.name.clone(),
            }
        })
        .collect()
}

fn env_from(container: &Container) -> Vec<String> {
    container
        .env_from
        .iter()
        .flatten()
        .filter_map(|source| {
            if let Some(cm) = &source.config_map_ref {
                Some(format!("ConfigMap/{}", cm.name))
            } else {
                source
                    .secret_ref
                    .as_ref()
                    .map(|secret| format!("Secret/{}", secret.name))
            }
        })
        .collect()
}

fn probe(probe: Option<&Probe>) -> Value {
    let Some(p) = probe else {
        return Value::Null;
    };
    let action = if let Some(http) = &p.http_get {
        format!(
            "http GET {}:{}",
            http.path.as_deref().unwrap_or("/"),
            int_or_string(&http.port)
        )
    } else if let Some(tcp) = &p.tcp_socket {
        format!("tcp :{}", int_or_string(&tcp.port))
    } else if let Some(grpc) = &p.grpc {
        format!("grpc :{}", grpc.port)
    } else if p.exec.is_some() {
        "exec".to_string()
    } else {
        "unknown".to_string()
    };
    Value::String(format!(
        "{action} delay={}s period={}s timeout={}s failures={}",
        p.initial_delay_seconds.unwrap_or(0),
        p.period_seconds.unwrap_or(10),
        p.timeout_seconds.unwrap_or(1),
        p.failure_threshold.unwrap_or(3)
    ))
}

/// A container: name, image, ports, resources, probes, mounts and env var
/// *names* only. Command and args are left out (they can carry secrets).
pub fn shape_container(c: &Container) -> Value {
    let ports: Vec<String> = c
        .ports
        .iter()
        .flatten()
        .map(|p| {
            let name = p
                .name
                .as_deref()
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            format!(
                "{}/{}{name}",
                p.container_port,
                p.protocol.as_deref().unwrap_or("TCP")
            )
        })
        .collect();
    let mounts: Vec<String> = c
        .volume_mounts
        .iter()
        .flatten()
        .map(|m| {
            let ro = if m.read_only == Some(true) {
                " (ro)"
            } else {
                ""
            };
            format!("{} -> {}{ro}", m.name, m.mount_path)
        })
        .collect();
    let resources = c.resources.as_ref();
    compact(json!({
        "name": c.name,
        "image": c.image,
        "ports": ports,
        "requests": quantities(resources.and_then(|r| r.requests.as_ref())),
        "limits": quantities(resources.and_then(|r| r.limits.as_ref())),
        "env": env_names(c),
        "env_from": env_from(c),
        "mounts": mounts,
        "liveness": probe(c.liveness_probe.as_ref()),
        "readiness": probe(c.readiness_probe.as_ref()),
        "startup": probe(c.startup_probe.as_ref()),
    }))
}

/// A volume: its name and type (plus the claim, config map or path it
/// points at). Secret volumes show the type only.
pub fn shape_volume(v: &Volume) -> String {
    let detail = if let Some(pvc) = &v.persistent_volume_claim {
        format!("persistentVolumeClaim {}", pvc.claim_name)
    } else if let Some(cm) = &v.config_map {
        format!("configMap {}", cm.name)
    } else if v.secret.is_some() {
        "secret".to_string()
    } else if let Some(host) = &v.host_path {
        format!("hostPath {}", host.path)
    } else if let Some(nfs) = &v.nfs {
        format!("nfs {}:{}", nfs.server, nfs.path)
    } else if v.empty_dir.is_some() {
        "emptyDir".to_string()
    } else if v.projected.is_some() {
        "projected".to_string()
    } else if v.downward_api.is_some() {
        "downwardAPI".to_string()
    } else if let Some(csi) = &v.csi {
        format!("csi {}", csi.driver)
    } else if v.ephemeral.is_some() {
        "ephemeral".to_string()
    } else {
        "other".to_string()
    };
    format!("{}: {detail}", v.name)
}

/// A pod spec (also the template of workloads).
pub fn shape_pod_spec(spec: &PodSpec) -> Value {
    let containers: Vec<Value> = spec.containers.iter().map(shape_container).collect();
    let init: Vec<Value> = spec
        .init_containers
        .iter()
        .flatten()
        .map(shape_container)
        .collect();
    let volumes: Vec<String> = spec.volumes.iter().flatten().map(shape_volume).collect();
    compact(json!({
        "node": spec.node_name,
        "service_account": spec.service_account_name,
        "restart_policy": spec.restart_policy,
        "priority_class": spec.priority_class_name,
        "node_selector": shape_string_map(spec.node_selector.as_ref()),
        "host_network": spec.host_network,
        "init_containers": init,
        "containers": containers,
        "volumes": volumes,
    }))
}

fn template(t: &PodTemplateSpec) -> Value {
    let meta = t.metadata.as_ref();
    compact(json!({
        "labels": shape_string_map(meta.and_then(|m| m.labels.as_ref())),
        "annotations": shape_string_map(meta.and_then(|m| m.annotations.as_ref())),
        "spec": t.spec.as_ref().map(shape_pod_spec),
    }))
}

fn container_state(state: Option<&ContainerState>) -> Value {
    let Some(state) = state else {
        return Value::Null;
    };
    if let Some(w) = &state.waiting {
        let mut s = format!("waiting: {}", w.reason.as_deref().unwrap_or("unknown"));
        if let Value::String(m) = text(w.message.as_deref()) {
            s.push_str(&format!(" ({m})"));
        }
        Value::String(s)
    } else if let Some(t) = &state.terminated {
        let mut s = format!(
            "terminated: {} (exit code {})",
            t.reason.as_deref().unwrap_or("unknown"),
            t.exit_code
        );
        if let Some(at) = &t.finished_at {
            s.push_str(&format!(" at {}", at.0));
        }
        if let Value::String(m) = text(t.message.as_deref()) {
            s.push_str(&format!(": {m}"));
        }
        Value::String(s)
    } else if let Some(r) = &state.running {
        Value::String(match &r.started_at {
            Some(at) => format!("running since {}", at.0),
            None => "running".to_string(),
        })
    } else {
        Value::Null
    }
}

fn container_status(cs: &ContainerStatus) -> Value {
    compact(json!({
        "name": cs.name,
        "ready": cs.ready,
        "restarts": cs.restart_count,
        "image": cs.image,
        "state": container_state(cs.state.as_ref()),
        "last_state": container_state(cs.last_state.as_ref()),
    }))
}

fn pod_status(status: &PodStatus) -> Value {
    compact(json!({
        "phase": status.phase,
        "reason": status.reason,
        "message": text(status.message.as_deref()),
        "started": time(status.start_time.as_ref()),
        "pod_ip": status.pod_ip,
        "host_ip": status.host_ip,
        "qos": status.qos_class,
        "conditions": conditions(status.conditions.as_ref()),
        "init_containers": status.init_container_statuses.iter().flatten().map(container_status).collect::<Vec<_>>(),
        "containers": status.container_statuses.iter().flatten().map(container_status).collect::<Vec<_>>(),
    }))
}

/// What a kind needs for `k8s_list` (one line) and `k8s_describe`.
pub trait Shape {
    fn meta(&self) -> &ObjectMeta;
    /// One compact line, without the namespace.
    fn line(&self, now: i64) -> String;
    /// A compact, redacted summary.
    fn describe(&self) -> Value;
}

fn name_of(meta: &ObjectMeta) -> &str {
    meta.name.as_deref().unwrap_or("?")
}

fn created(meta: &ObjectMeta) -> Option<i64> {
    seconds(meta.creation_timestamp.as_ref())
}

/// Restarts and the most telling container problem of a pod.
fn pod_trouble(pod: &Pod) -> (i32, Option<String>) {
    let statuses: Vec<&ContainerStatus> = pod
        .status
        .iter()
        .flat_map(|s| {
            s.init_container_statuses
                .iter()
                .flatten()
                .chain(s.container_statuses.iter().flatten())
        })
        .collect();
    let restarts = statuses.iter().map(|s| s.restart_count).sum();
    let reason = statuses.iter().find_map(|s| {
        let state = s.state.as_ref()?;
        state
            .waiting
            .as_ref()
            .and_then(|w| w.reason.clone())
            .or_else(|| {
                state
                    .terminated
                    .as_ref()
                    .filter(|t| t.exit_code != 0)
                    .and_then(|t| t.reason.clone())
            })
    });
    (restarts, reason)
}

impl Shape for Pod {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn line(&self, now: i64) -> String {
        let status = self.status.as_ref();
        let containers = status
            .and_then(|s| s.container_statuses.as_ref())
            .map(Vec::as_slice)
            .unwrap_or_default();
        let ready = containers.iter().filter(|c| c.ready).count();
        let total = self
            .spec
            .as_ref()
            .map_or(containers.len(), |s| s.containers.len());
        let (restarts, reason) = pod_trouble(self);
        let phase = status.and_then(|s| s.phase.as_deref()).unwrap_or("Unknown");
        let mut line = format!(
            "{} {phase} ready {ready}/{total} restarts {restarts}",
            name_of(&self.metadata)
        );
        if let Some(reason) = reason {
            line.push_str(&format!(" ({reason})"));
        }
        if let Some(node) = self.spec.as_ref().and_then(|s| s.node_name.as_deref()) {
            line.push_str(&format!(" node {node}"));
        }
        line.push_str(&format!(" age {}", age(created(&self.metadata), now)));
        line
    }
    fn describe(&self) -> Value {
        compact(json!({
            "kind": "Pod",
            "metadata": shape_meta(&self.metadata),
            "spec": self.spec.as_ref().map(shape_pod_spec),
            "status": self.status.as_ref().map(pod_status),
        }))
    }
}

impl Shape for Deployment {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn line(&self, now: i64) -> String {
        let s = self.status.as_ref();
        format!(
            "{} ready {}/{} up-to-date {} available {} age {}",
            name_of(&self.metadata),
            s.and_then(|s| s.ready_replicas).unwrap_or(0),
            self.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1),
            s.and_then(|s| s.updated_replicas).unwrap_or(0),
            s.and_then(|s| s.available_replicas).unwrap_or(0),
            age(created(&self.metadata), now)
        )
    }
    fn describe(&self) -> Value {
        let spec = self.spec.as_ref();
        let status = self.status.as_ref();
        let strategy = spec.and_then(|s| s.strategy.as_ref()).map(|st| {
            let mut s = st.type_.clone().unwrap_or_else(|| "RollingUpdate".into());
            if let Some(ru) = &st.rolling_update {
                s.push_str(&format!(
                    " (maxSurge {}, maxUnavailable {})",
                    ru.max_surge
                        .as_ref()
                        .map(int_or_string)
                        .unwrap_or("25%".into()),
                    ru.max_unavailable
                        .as_ref()
                        .map(int_or_string)
                        .unwrap_or("25%".into())
                ));
            }
            s
        });
        compact(json!({
            "kind": "Deployment",
            "metadata": shape_meta(&self.metadata),
            "replicas": {
                "desired": spec.and_then(|s| s.replicas),
                "current": status.and_then(|s| s.replicas),
                "ready": status.and_then(|s| s.ready_replicas),
                "updated": status.and_then(|s| s.updated_replicas),
                "available": status.and_then(|s| s.available_replicas),
                "unavailable": status.and_then(|s| s.unavailable_replicas),
            },
            "paused": spec.and_then(|s| s.paused),
            "selector": selector(spec.map(|s| &s.selector)),
            "strategy": strategy,
            "conditions": conditions(status.and_then(|s| s.conditions.as_ref())),
            "template": spec.map(|s| template(&s.template)),
        }))
    }
}

impl Shape for StatefulSet {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn line(&self, now: i64) -> String {
        format!(
            "{} ready {}/{} age {}",
            name_of(&self.metadata),
            self.status
                .as_ref()
                .and_then(|s| s.ready_replicas)
                .unwrap_or(0),
            self.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1),
            age(created(&self.metadata), now)
        )
    }
    fn describe(&self) -> Value {
        let spec = self.spec.as_ref();
        let status = self.status.as_ref();
        let claims: Vec<String> = spec
            .and_then(|s| s.volume_claim_templates.as_ref())
            .into_iter()
            .flatten()
            .map(|pvc| {
                let size = pvc
                    .spec
                    .as_ref()
                    .and_then(|s| s.resources.as_ref())
                    .and_then(|r| r.requests.as_ref())
                    .and_then(|r| r.get("storage"))
                    .map(|q| q.0.clone())
                    .unwrap_or_default();
                format!("{} {size}", name_of(&pvc.metadata))
            })
            .collect();
        compact(json!({
            "kind": "StatefulSet",
            "metadata": shape_meta(&self.metadata),
            "replicas": {
                "desired": spec.and_then(|s| s.replicas),
                "current": status.map(|s| s.replicas),
                "ready": status.and_then(|s| s.ready_replicas),
                "updated": status.and_then(|s| s.updated_replicas),
                "available": status.and_then(|s| s.available_replicas),
            },
            "current_revision": status.and_then(|s| s.current_revision.clone()),
            "update_revision": status.and_then(|s| s.update_revision.clone()),
            "service_name": spec.and_then(|s| s.service_name.clone()),
            "selector": selector(spec.map(|s| &s.selector)),
            "update_strategy": spec.and_then(|s| s.update_strategy.as_ref()).and_then(|u| u.type_.clone()),
            "volume_claim_templates": claims,
            "conditions": conditions(status.and_then(|s| s.conditions.as_ref())),
            "template": spec.map(|s| template(&s.template)),
        }))
    }
}

impl Shape for DaemonSet {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn line(&self, now: i64) -> String {
        let s = self.status.as_ref();
        format!(
            "{} desired {} ready {} up-to-date {} available {} age {}",
            name_of(&self.metadata),
            s.map_or(0, |s| s.desired_number_scheduled),
            s.map_or(0, |s| s.number_ready),
            s.and_then(|s| s.updated_number_scheduled).unwrap_or(0),
            s.and_then(|s| s.number_available).unwrap_or(0),
            age(created(&self.metadata), now)
        )
    }
    fn describe(&self) -> Value {
        let spec = self.spec.as_ref();
        let s = self.status.as_ref();
        compact(json!({
            "kind": "DaemonSet",
            "metadata": shape_meta(&self.metadata),
            "pods": {
                "desired": s.map(|s| s.desired_number_scheduled),
                "current": s.map(|s| s.current_number_scheduled),
                "ready": s.map(|s| s.number_ready),
                "updated": s.and_then(|s| s.updated_number_scheduled),
                "available": s.and_then(|s| s.number_available),
                "unavailable": s.and_then(|s| s.number_unavailable),
                "misscheduled": s.map(|s| s.number_misscheduled),
            },
            "selector": selector(spec.map(|s| &s.selector)),
            "update_strategy": spec.and_then(|s| s.update_strategy.as_ref()).and_then(|u| u.type_.clone()),
            "conditions": conditions(s.and_then(|s| s.conditions.as_ref())),
            "template": spec.map(|s| template(&s.template)),
        }))
    }
}

fn job_state(job: &Job) -> &'static str {
    let finished = |kind: &str| {
        job.status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .into_iter()
            .flatten()
            .any(|c| c.type_ == kind && c.status == "True")
    };
    if finished("Complete") {
        "Complete"
    } else if finished("Failed") {
        "Failed"
    } else if job.spec.as_ref().and_then(|s| s.suspend) == Some(true) {
        "Suspended"
    } else {
        "Running"
    }
}

impl Shape for Job {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn line(&self, now: i64) -> String {
        let s = self.status.as_ref();
        format!(
            "{} {} succeeded {}/{} failed {} active {} age {}",
            name_of(&self.metadata),
            job_state(self),
            s.and_then(|s| s.succeeded).unwrap_or(0),
            self.spec.as_ref().and_then(|s| s.completions).unwrap_or(1),
            s.and_then(|s| s.failed).unwrap_or(0),
            s.and_then(|s| s.active).unwrap_or(0),
            age(created(&self.metadata), now)
        )
    }
    fn describe(&self) -> Value {
        let spec = self.spec.as_ref();
        let s = self.status.as_ref();
        compact(json!({
            "kind": "Job",
            "metadata": shape_meta(&self.metadata),
            "state": job_state(self),
            "completions": spec.and_then(|s| s.completions),
            "parallelism": spec.and_then(|s| s.parallelism),
            "backoff_limit": spec.and_then(|s| s.backoff_limit),
            "active": s.and_then(|s| s.active),
            "succeeded": s.and_then(|s| s.succeeded),
            "failed": s.and_then(|s| s.failed),
            "started": time(s.and_then(|s| s.start_time.as_ref())),
            "completed": time(s.and_then(|s| s.completion_time.as_ref())),
            "conditions": conditions(s.and_then(|s| s.conditions.as_ref())),
            "template": spec.map(|s| template(&s.template)),
        }))
    }
}

impl Shape for CronJob {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn line(&self, now: i64) -> String {
        let spec = self.spec.as_ref();
        let s = self.status.as_ref();
        let last = s
            .and_then(|s| s.last_schedule_time.as_ref())
            .map(|t| format!("{} ago", age(Some(t.0.as_second()), now)))
            .unwrap_or_else(|| "never".into());
        format!(
            "{} schedule \"{}\" suspended {} active {} last run {last}",
            name_of(&self.metadata),
            spec.map(|s| s.schedule.as_str()).unwrap_or("?"),
            spec.and_then(|s| s.suspend).unwrap_or(false),
            s.and_then(|s| s.active.as_ref()).map_or(0, Vec::len),
        )
    }
    fn describe(&self) -> Value {
        let spec = self.spec.as_ref();
        let s = self.status.as_ref();
        let active: Vec<String> = s
            .and_then(|s| s.active.as_ref())
            .into_iter()
            .flatten()
            .filter_map(|r| r.name.clone())
            .collect();
        let job_spec = spec.and_then(|s| s.job_template.spec.as_ref());
        compact(json!({
            "kind": "CronJob",
            "metadata": shape_meta(&self.metadata),
            "schedule": spec.map(|s| s.schedule.clone()),
            "time_zone": spec.and_then(|s| s.time_zone.clone()),
            "suspend": spec.and_then(|s| s.suspend),
            "concurrency_policy": spec.and_then(|s| s.concurrency_policy.clone()),
            "last_schedule": time(s.and_then(|s| s.last_schedule_time.as_ref())),
            "last_successful": time(s.and_then(|s| s.last_successful_time.as_ref())),
            "active_jobs": active,
            "backoff_limit": job_spec.and_then(|j| j.backoff_limit),
            "template": job_spec.map(|j| template(&j.template)),
        }))
    }
}

fn service_ports(svc: &Service) -> Vec<String> {
    svc.spec
        .as_ref()
        .and_then(|s| s.ports.as_ref())
        .into_iter()
        .flatten()
        .map(|p| {
            let mut s = format!("{}/{}", p.port, p.protocol.as_deref().unwrap_or("TCP"));
            if let Some(target) = &p.target_port {
                s.push_str(&format!("->{}", int_or_string(target)));
            }
            if let Some(node_port) = p.node_port {
                s.push_str(&format!(" nodePort {node_port}"));
            }
            if let Some(name) = &p.name {
                s.push_str(&format!(" ({name})"));
            }
            s
        })
        .collect()
}

impl Shape for Service {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn line(&self, _now: i64) -> String {
        let spec = self.spec.as_ref();
        format!(
            "{} {} {} ports {}",
            name_of(&self.metadata),
            spec.and_then(|s| s.type_.as_deref()).unwrap_or("ClusterIP"),
            spec.and_then(|s| s.cluster_ip.as_deref()).unwrap_or("-"),
            service_ports(self).join(", ")
        )
    }
    fn describe(&self) -> Value {
        let spec = self.spec.as_ref();
        let ingress: Vec<String> = self
            .status
            .as_ref()
            .and_then(|s| s.load_balancer.as_ref())
            .and_then(|lb| lb.ingress.as_ref())
            .into_iter()
            .flatten()
            .filter_map(|i| i.ip.clone().or_else(|| i.hostname.clone()))
            .collect();
        compact(json!({
            "kind": "Service",
            "metadata": shape_meta(&self.metadata),
            "type": spec.and_then(|s| s.type_.clone()),
            "cluster_ip": spec.and_then(|s| s.cluster_ip.clone()),
            "external_ips": spec.and_then(|s| s.external_ips.clone()),
            "load_balancer_ip": spec.and_then(|s| s.load_balancer_ip.clone()),
            "load_balancer_ingress": ingress,
            "ports": service_ports(self),
            "selector": shape_string_map(spec.and_then(|s| s.selector.as_ref())),
            "external_traffic_policy": spec.and_then(|s| s.external_traffic_policy.clone()),
        }))
    }
}

impl Shape for PersistentVolumeClaim {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn line(&self, now: i64) -> String {
        let spec = self.spec.as_ref();
        let status = self.status.as_ref();
        format!(
            "{} {} {} {} class {} volume {} age {}",
            name_of(&self.metadata),
            status.and_then(|s| s.phase.as_deref()).unwrap_or("?"),
            status
                .and_then(|s| s.capacity.as_ref())
                .and_then(|c| c.get("storage"))
                .map(|q| q.0.as_str())
                .unwrap_or("-"),
            spec.and_then(|s| s.access_modes.as_ref())
                .map(|m| m.join(","))
                .unwrap_or_default(),
            spec.and_then(|s| s.storage_class_name.as_deref())
                .unwrap_or("-"),
            spec.and_then(|s| s.volume_name.as_deref()).unwrap_or("-"),
            age(created(&self.metadata), now)
        )
    }
    fn describe(&self) -> Value {
        let spec = self.spec.as_ref();
        let status = self.status.as_ref();
        compact(json!({
            "kind": "PersistentVolumeClaim",
            "metadata": shape_meta(&self.metadata),
            "phase": status.and_then(|s| s.phase.clone()),
            "storage_class": spec.and_then(|s| s.storage_class_name.clone()),
            "access_modes": spec.and_then(|s| s.access_modes.clone()),
            "volume_mode": spec.and_then(|s| s.volume_mode.clone()),
            "requested": quantities(spec.and_then(|s| s.resources.as_ref()).and_then(|r| r.requests.as_ref())),
            "capacity": quantities(status.and_then(|s| s.capacity.as_ref())),
            "volume": spec.and_then(|s| s.volume_name.clone()),
            "conditions": conditions(status.and_then(|s| s.conditions.as_ref())),
        }))
    }
}

fn pv_source(pv: &PersistentVolume) -> String {
    let Some(spec) = pv.spec.as_ref() else {
        return "unknown".into();
    };
    if let Some(nfs) = &spec.nfs {
        format!("nfs {}:{}", nfs.server, nfs.path)
    } else if let Some(host) = &spec.host_path {
        format!("hostPath {}", host.path)
    } else if let Some(local) = &spec.local {
        format!("local {}", local.path)
    } else if let Some(csi) = &spec.csi {
        format!("csi {}", csi.driver)
    } else {
        "other".into()
    }
}

impl Shape for PersistentVolume {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn line(&self, now: i64) -> String {
        let spec = self.spec.as_ref();
        let claim = spec
            .and_then(|s| s.claim_ref.as_ref())
            .map(|c| {
                format!(
                    "{}/{}",
                    c.namespace.as_deref().unwrap_or("-"),
                    c.name.as_deref().unwrap_or("-")
                )
            })
            .unwrap_or_else(|| "-".into());
        format!(
            "{} {} {} {} {} claim {claim} class {} age {}",
            name_of(&self.metadata),
            spec.and_then(|s| s.capacity.as_ref())
                .and_then(|c| c.get("storage"))
                .map(|q| q.0.as_str())
                .unwrap_or("-"),
            spec.and_then(|s| s.access_modes.as_ref())
                .map(|m| m.join(","))
                .unwrap_or_default(),
            spec.and_then(|s| s.persistent_volume_reclaim_policy.as_deref())
                .unwrap_or("-"),
            self.status
                .as_ref()
                .and_then(|s| s.phase.as_deref())
                .unwrap_or("?"),
            spec.and_then(|s| s.storage_class_name.as_deref())
                .unwrap_or("-"),
            age(created(&self.metadata), now)
        )
    }
    fn describe(&self) -> Value {
        let spec = self.spec.as_ref();
        let status = self.status.as_ref();
        compact(json!({
            "kind": "PersistentVolume",
            "metadata": shape_meta(&self.metadata),
            "phase": status.and_then(|s| s.phase.clone()),
            "reason": status.and_then(|s| s.reason.clone()),
            "message": text(status.and_then(|s| s.message.as_deref())),
            "capacity": quantities(spec.and_then(|s| s.capacity.as_ref())),
            "access_modes": spec.and_then(|s| s.access_modes.clone()),
            "reclaim_policy": spec.and_then(|s| s.persistent_volume_reclaim_policy.clone()),
            "storage_class": spec.and_then(|s| s.storage_class_name.clone()),
            "claim": spec.and_then(|s| s.claim_ref.as_ref()).map(|c| format!(
                "{}/{}",
                c.namespace.as_deref().unwrap_or("-"),
                c.name.as_deref().unwrap_or("-")
            )),
            "source": pv_source(self),
        }))
    }
}

fn node_ready(node: &Node) -> &'static str {
    let ready = node
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .into_iter()
        .flatten()
        .find(|c| c.type_ == "Ready");
    match ready.map(|c| c.status.as_str()) {
        Some("True") => "Ready",
        Some(_) => "NotReady",
        None => "Unknown",
    }
}

impl Shape for Node {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn line(&self, now: i64) -> String {
        let mut state = node_ready(self).to_string();
        if self.spec.as_ref().and_then(|s| s.unschedulable) == Some(true) {
            state.push_str(",SchedulingDisabled");
        }
        format!(
            "{} {state} {} age {}",
            name_of(&self.metadata),
            self.status
                .as_ref()
                .and_then(|s| s.node_info.as_ref())
                .map(|i| i.kubelet_version.as_str())
                .unwrap_or("-"),
            age(created(&self.metadata), now)
        )
    }
    fn describe(&self) -> Value {
        let status = self.status.as_ref();
        let spec = self.spec.as_ref();
        let info = status.and_then(|s| s.node_info.as_ref()).map(|i| {
            json!({
                "kubelet": i.kubelet_version,
                "os_image": i.os_image,
                "kernel": i.kernel_version,
                "container_runtime": i.container_runtime_version,
                "architecture": i.architecture,
            })
        });
        let addresses: Vec<String> = status
            .and_then(|s| s.addresses.as_ref())
            .into_iter()
            .flatten()
            .map(|a| format!("{} {}", a.type_, a.address))
            .collect();
        let taints: Vec<String> = spec
            .and_then(|s| s.taints.as_ref())
            .into_iter()
            .flatten()
            .map(|t| {
                let value = match &t.value {
                    Some(_) if secretish_key(&t.key) => format!("={REDACTED}"),
                    Some(v) => format!("={v}"),
                    None => String::new(),
                };
                format!("{}{value}:{}", t.key, t.effect)
            })
            .collect();
        compact(json!({
            "kind": "Node",
            "metadata": shape_meta(&self.metadata),
            "ready": node_ready(self),
            "unschedulable": spec.and_then(|s| s.unschedulable),
            "taints": taints,
            "addresses": addresses,
            "capacity": quantities(status.and_then(|s| s.capacity.as_ref())),
            "allocatable": quantities(status.and_then(|s| s.allocatable.as_ref())),
            "info": info,
            "conditions": conditions(status.and_then(|s| s.conditions.as_ref())),
        }))
    }
}

/// When an event last happened, in Unix seconds.
pub fn event_seconds(event: &Event) -> Option<i64> {
    event
        .last_timestamp
        .as_ref()
        .map(|t| t.0.as_second())
        .or_else(|| event.event_time.as_ref().map(|t| t.0.as_second()))
        .or_else(|| event.first_timestamp.as_ref().map(|t| t.0.as_second()))
        .or_else(|| created(&event.metadata))
}

/// `<time> <type> <reason> <Kind>/<name>: <message> (xN)`.
pub fn event_line(event: &Event) -> String {
    let at = event_seconds(event)
        .and_then(|s| chrono::DateTime::<chrono::Utc>::from_timestamp(s, 0))
        .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| "?".into());
    let object = &event.involved_object;
    let message = match text(event.message.as_deref()) {
        Value::String(m) => m,
        _ => String::new(),
    };
    let mut line = format!(
        "{at} {} {} {}/{}: {message}",
        event.type_.as_deref().unwrap_or("Normal"),
        event.reason.as_deref().unwrap_or("-"),
        object.kind.as_deref().unwrap_or("?"),
        object.name.as_deref().unwrap_or("?"),
    );
    if let Some(count) = event.count.filter(|c| *c > 1) {
        line.push_str(&format!(" (x{count})"));
    }
    line
}

impl Shape for Event {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn line(&self, _now: i64) -> String {
        event_line(self)
    }
    fn describe(&self) -> Value {
        let object = &self.involved_object;
        compact(json!({
            "kind": "Event",
            "metadata": shape_meta(&self.metadata),
            "type": self.type_,
            "reason": self.reason,
            "message": text(self.message.as_deref()),
            "object": format!(
                "{}/{}",
                object.kind.as_deref().unwrap_or("?"),
                object.name.as_deref().unwrap_or("?")
            ),
            "count": self.count,
            "first_seen": time(self.first_timestamp.as_ref()),
            "last_seen": time(self.last_timestamp.as_ref()),
            "source": self.source.as_ref().and_then(|s| s.component.clone()).or_else(|| self.reporting_component.clone()),
        }))
    }
}

impl Shape for Namespace {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn line(&self, now: i64) -> String {
        format!(
            "{} {} age {}",
            name_of(&self.metadata),
            self.status
                .as_ref()
                .and_then(|s| s.phase.as_deref())
                .unwrap_or("?"),
            age(created(&self.metadata), now)
        )
    }
    fn describe(&self) -> Value {
        let status = self.status.as_ref();
        compact(json!({
            "kind": "Namespace",
            "metadata": shape_meta(&self.metadata),
            "phase": status.and_then(|s| s.phase.clone()),
            "conditions": conditions(status.and_then(|s| s.conditions.as_ref())),
        }))
    }
}

/// The names of a ConfigMap's data keys; never their values.
pub fn config_map_keys(cm: &ConfigMap) -> Vec<String> {
    cm.data
        .iter()
        .flat_map(|d| d.keys())
        .chain(cm.binary_data.iter().flat_map(|d| d.keys()))
        .cloned()
        .collect()
}

impl Shape for ConfigMap {
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn line(&self, _now: i64) -> String {
        let keys = config_map_keys(self);
        let mut line = format!(
            "{} keys: {}",
            name_of(&self.metadata),
            keys.iter()
                .take(MAX_LINE_KEYS)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
        if keys.len() > MAX_LINE_KEYS {
            line.push_str(&format!(" …and {} more", keys.len() - MAX_LINE_KEYS));
        }
        line
    }
    fn describe(&self) -> Value {
        compact(json!({
            "kind": "ConfigMap",
            "metadata": shape_meta(&self.metadata),
            "keys": config_map_keys(self),
            "note": "Values are not shown, only key names.",
        }))
    }
}

/// HTTPRoute (Gateway API, read dynamically): hosts and backends from its
/// JSON `spec`.
fn route_backends(spec: &Value) -> Vec<String> {
    spec["rules"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|rule| rule["backendRefs"].as_array().into_iter().flatten())
        .map(|b| {
            let mut s = b["name"].as_str().unwrap_or("?").to_string();
            if let Some(ns) = b["namespace"].as_str() {
                s = format!("{ns}/{s}");
            }
            if let Some(port) = b["port"].as_i64() {
                s.push_str(&format!(":{port}"));
            }
            s
        })
        .collect()
}

fn route_hosts(spec: &Value) -> Vec<String> {
    spec["hostnames"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| h.as_str().map(str::to_string))
        .collect()
}

pub fn httproute_line(meta: &ObjectMeta, data: &Value) -> String {
    let spec = &data["spec"];
    format!(
        "{} hosts {} -> {}",
        name_of(meta),
        route_hosts(spec).join(","),
        route_backends(spec).join(", ")
    )
}

pub fn httproute_describe(meta: &ObjectMeta, data: &Value) -> Value {
    let spec = &data["spec"];
    let parents: Vec<String> = spec["parentRefs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let mut s = p["name"].as_str().unwrap_or("?").to_string();
            if let Some(ns) = p["namespace"].as_str() {
                s = format!("{ns}/{s}");
            }
            if let Some(section) = p["sectionName"].as_str() {
                s.push_str(&format!(" ({section})"));
            }
            s
        })
        .collect();
    let rules: Vec<Value> = spec["rules"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|rule| {
            let matches: Vec<String> = rule["matches"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| {
                    let path = &m["path"];
                    path["value"]
                        .as_str()
                        .map(|v| format!("{} {v}", path["type"].as_str().unwrap_or("PathPrefix")))
                })
                .collect();
            let filters: Vec<String> = rule["filters"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| f["type"].as_str().map(str::to_string))
                .collect();
            compact(json!({
                "matches": matches,
                "filters": filters,
                "backends": route_backends(&json!({"rules": [rule]})),
            }))
        })
        .collect();
    let status: Vec<Value> = data["status"]["parents"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let conditions: Vec<Value> = p["conditions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| {
                    json!({
                        "type": c["type"],
                        "status": c["status"],
                        "reason": c["reason"],
                        "message": text(c["message"].as_str()),
                    })
                })
                .collect();
            json!({"parent": p["parentRef"]["name"], "conditions": conditions})
        })
        .collect();
    compact(json!({
        "kind": "HTTPRoute",
        "metadata": shape_meta(meta),
        "hostnames": route_hosts(spec),
        "parents": parents,
        "rules": rules,
        "status": status,
    }))
}

/// Scrub every string in a shaped result with the namespace's known Secret
/// values (plus the inline patterns), e.g. termination messages that hold a
/// log tail (`FallbackToLogsOnError`), event and condition messages.
pub fn scrub_value(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(s) => *s = scrub_with(s, secrets),
        Value::Array(items) => items.iter_mut().for_each(|v| scrub_value(v, secrets)),
        Value::Object(map) => map.values_mut().for_each(|v| scrub_value(v, secrets)),
        _ => {}
    }
}

/// Pod logs for the model: every line scrubbed (known Secret values, inline
/// patterns, URL credentials) *before* it is clipped, so a clip can never
/// leave a recognisable prefix of a secret. Keeps the newest lines that fit
/// in [`LOG_BUDGET_BYTES`]. Returns the text and the number of lines kept.
pub fn shape_logs(raw: &str, secrets: &[String]) -> (String, usize) {
    let clean = scrub_with(raw, secrets);
    let lines: Vec<String> = clean
        .lines()
        .map(|line| clip(line, MAX_LOG_LINE_CHARS))
        .collect();
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 0;
    let mut omitted = false;
    for line in lines.iter().rev() {
        if used + line.len() + 1 > LOG_BUDGET_BYTES {
            omitted = true;
            break;
        }
        used += line.len() + 1;
        kept.push(line);
    }
    kept.reverse();
    let count = kept.len();
    let mut text = kept.join("\n");
    if omitted {
        text = format!("{LOGS_OMITTED_MARKER}\n{text}");
    }
    (text, count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        ConfigMapEnvSource, ContainerStateWaiting, ContainerStatus, EnvFromSource, EnvVar,
        EnvVarSource, ObjectFieldSelector, SecretEnvSource, SecretKeySelector, SecretVolumeSource,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{FieldsV1, ManagedFieldsEntry};

    const SECRET: &str = "hunter2-very-secret-value";

    fn leaky_container() -> Container {
        Container {
            name: "app".into(),
            image: Some("ghcr.io/example/app:1.2.3".into()),
            command: Some(vec!["/app".into(), format!("--password={SECRET}")]),
            args: Some(vec![format!("--token {SECRET}")]),
            env: Some(vec![
                EnvVar {
                    name: "PLAIN".into(),
                    value: Some(SECRET.into()),
                    value_from: None,
                },
                EnvVar {
                    name: "DB_PASSWORD".into(),
                    value: None,
                    value_from: Some(EnvVarSource {
                        secret_key_ref: Some(SecretKeySelector {
                            name: "db-credentials-target".into(),
                            key: "password-key-target".into(),
                            optional: None,
                        }),
                        ..Default::default()
                    }),
                },
                EnvVar {
                    name: "POD_IP".into(),
                    value: None,
                    value_from: Some(EnvVarSource {
                        field_ref: Some(ObjectFieldSelector {
                            field_path: "status.podIP".into(),
                            api_version: None,
                        }),
                        ..Default::default()
                    }),
                },
            ]),
            env_from: Some(vec![
                EnvFromSource {
                    secret_ref: Some(SecretEnvSource {
                        name: "app-secrets".into(),
                        optional: None,
                    }),
                    prefix: None,
                    config_map_ref: None,
                },
                EnvFromSource {
                    config_map_ref: Some(ConfigMapEnvSource {
                        name: "app-config".into(),
                        optional: None,
                    }),
                    prefix: None,
                    secret_ref: None,
                },
            ]),
            ..Default::default()
        }
    }

    fn leaky_meta() -> ObjectMeta {
        ObjectMeta {
            name: Some("app-0".into()),
            namespace: Some("jellyfin".into()),
            annotations: Some(BTreeMap::from([
                (
                    LAST_APPLIED.to_string(),
                    format!(r#"{{"env":[{{"value":"{SECRET}"}}]}}"#),
                ),
                ("example.com/api-token".to_string(), SECRET.to_string()),
                ("checksum/config".to_string(), "abc123".to_string()),
                ("example.com/basic-auth".to_string(), SECRET.to_string()),
            ])),
            labels: Some(BTreeMap::from([
                ("app".to_string(), "jellyfin".to_string()),
                ("cert-hash".to_string(), SECRET.to_string()),
            ])),
            managed_fields: Some(vec![ManagedFieldsEntry {
                manager: Some(format!("manager-{SECRET}")),
                fields_v1: Some(FieldsV1(json!({"f:data": {SECRET: {}}}))),
                ..Default::default()
            }]),
            ..Default::default()
        }
    }

    fn leaky_pod() -> Pod {
        Pod {
            metadata: leaky_meta(),
            spec: Some(PodSpec {
                containers: vec![leaky_container()],
                init_containers: Some(vec![leaky_container()]),
                volumes: Some(vec![
                    Volume {
                        name: "creds".into(),
                        secret: Some(SecretVolumeSource {
                            secret_name: Some("secret-volume-target".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Volume {
                        name: "config".into(),
                        persistent_volume_claim: Some(
                            k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                                claim_name: "jellyfin-config".into(),
                                read_only: None,
                            },
                        ),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".into()),
                container_statuses: Some(vec![ContainerStatus {
                    name: "app".into(),
                    ready: false,
                    restart_count: 4,
                    image: "ghcr.io/example/app:1.2.3".into(),
                    image_id: String::new(),
                    state: Some(ContainerState {
                        waiting: Some(ContainerStateWaiting {
                            reason: Some("CrashLoopBackOff".into()),
                            message: Some(format!("back-off; password={SECRET}")),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn pod_describe_leaks_no_values() {
        let pod = leaky_pod();
        let out = pod.describe().to_string();
        assert!(!out.contains(SECRET), "{out}");
        assert!(!out.contains("hunter2"), "{out}");
        // valueFrom targets are stripped; the source kind remains.
        assert!(!out.contains("db-credentials-target"), "{out}");
        assert!(!out.contains("password-key-target"), "{out}");
        assert!(!out.contains("status.podIP"), "{out}");
        assert!(!out.contains("secret-volume-target"), "{out}");
        assert!(out.contains("DB_PASSWORD (from secret)"), "{out}");
        assert!(out.contains("POD_IP (from pod field)"), "{out}");
        assert!(out.contains("\"PLAIN\""), "{out}");
        // envFrom shows the source kind/name only.
        assert!(out.contains("Secret/app-secrets"), "{out}");
        assert!(out.contains("ConfigMap/app-config"), "{out}");
        // No managedFields, no last-applied, secret-ish keys redacted.
        assert!(!out.contains("managed"), "{out}");
        assert!(!out.contains("last-applied"), "{out}");
        assert!(
            out.contains(r#""example.com/api-token":"[redacted]""#),
            "{out}"
        );
        assert!(
            out.contains(r#""example.com/basic-auth":"[redacted]""#),
            "{out}"
        );
        assert!(out.contains(r#""cert-hash":"[redacted]""#), "{out}");
        assert!(out.contains(r#""checksum/config":"abc123""#), "{out}");
        // Useful parts survive.
        assert!(out.contains("ghcr.io/example/app:1.2.3"), "{out}");
        assert!(out.contains("creds: secret"), "{out}");
        assert!(
            out.contains("config: persistentVolumeClaim jellyfin-config"),
            "{out}"
        );
        assert!(out.contains("CrashLoopBackOff"), "{out}");
        assert!(out.contains("\"restarts\":4"), "{out}");
    }

    #[test]
    fn pod_line_is_compact() {
        let line = leaky_pod().line(0);
        assert!(
            line.starts_with("app-0 Running ready 0/1 restarts 4"),
            "{line}"
        );
        assert!(line.contains("(CrashLoopBackOff)"), "{line}");
        assert!(!line.contains(SECRET));
    }

    #[test]
    fn deployment_template_is_shaped_like_a_pod() {
        let deployment = Deployment {
            metadata: leaky_meta(),
            spec: Some(k8s_openapi::api::apps::v1::DeploymentSpec {
                replicas: Some(2),
                selector: LabelSelector {
                    match_labels: Some(BTreeMap::from([("app".into(), "jellyfin".into())])),
                    match_expressions: None,
                },
                template: PodTemplateSpec {
                    metadata: Some(leaky_meta()),
                    spec: leaky_pod().spec,
                },
                ..Default::default()
            }),
            status: None,
        };
        let out = deployment.describe().to_string();
        assert!(!out.contains(SECRET), "{out}");
        assert!(!out.contains("db-credentials-target"), "{out}");
        assert!(out.contains("\"desired\":2"), "{out}");
        assert!(out.contains("app=jellyfin"), "{out}");
        assert!(out.contains("DB_PASSWORD (from secret)"), "{out}");
        assert!(deployment.line(0).starts_with("app-0 ready 0/2"));
    }

    #[test]
    fn config_maps_show_keys_only() {
        let cm = ConfigMap {
            metadata: leaky_meta(),
            data: Some(BTreeMap::from([
                ("settings.xml".into(), format!("<pw>{SECRET}</pw>")),
                ("API_URL".into(), "http://x".into()),
            ])),
            ..Default::default()
        };
        let out = cm.describe().to_string();
        assert!(!out.contains(SECRET), "{out}");
        assert!(!out.contains("http://x"), "{out}");
        assert!(
            out.contains("settings.xml") && out.contains("API_URL"),
            "{out}"
        );
        assert_eq!(cm.line(0), "app-0 keys: API_URL, settings.xml");
    }

    #[test]
    fn secrets_are_refused_and_kinds_parse() {
        for name in ["Secret", "secret", "Secrets", " SECRET "] {
            assert_eq!(Kind::parse(name), Err(SECRET_REFUSAL.to_string()));
        }
        assert_eq!(Kind::parse("pod"), Ok(Kind::Pod));
        assert_eq!(Kind::parse("HTTPRoute"), Ok(Kind::HttpRoute));
        assert!(Kind::parse("ClusterRole").is_err());
        assert!(Kind::Node.admin_only() && Kind::PersistentVolume.admin_only());
        assert!(!Kind::Namespace.admin_only() && Kind::Namespace.cluster_scoped());
        assert!(!Kind::Pod.cluster_scoped());
    }

    #[test]
    fn secretish_keys() {
        for key in [
            "api-token",
            "SECRET_NAME",
            "x/password",
            "passwd",
            "sshKey",
            "credentials",
            "oauth",
            "tls-cert",
        ] {
            assert!(secretish_key(key), "{key}");
        }
        for key in ["app", "checksum/config", "helm.sh/chart"] {
            assert!(!secretish_key(key), "{key}");
        }
    }

    #[test]
    fn logs_are_scrubbed_before_clipping_and_keep_the_newest_lines() {
        // A secret straddling the clip point must not leave a prefix behind.
        let padding = "x".repeat(MAX_LOG_LINE_CHARS - 5);
        let raw = format!(
            "{padding}{SECRET}\nconnecting to postgres://user:pw123@db/x\nBearer abc.def\nok"
        );
        let (text, count) = shape_logs(&raw, &[SECRET.to_string()]);
        assert_eq!(count, 4);
        assert!(!text.contains("hunter"), "{text}");
        assert!(!text.contains("pw123"), "{text}");
        assert!(!text.contains("abc.def"), "{text}");
        assert!(text.ends_with("ok"));

        let many: String = (0..2_000).map(|i| format!("line {i}\n")).collect();
        let (text, count) = shape_logs(&many, &[]);
        assert!(text.len() <= LOG_BUDGET_BYTES + LOGS_OMITTED_MARKER.len() + 1);
        assert!(text.starts_with(LOGS_OMITTED_MARKER));
        assert!(text.ends_with("line 1999"));
        assert!(count < 2_000);
    }

    #[test]
    fn httproute_is_summarised() {
        let meta = ObjectMeta {
            name: Some("jellyfin".into()),
            ..Default::default()
        };
        let data = json!({
            "spec": {
                "hostnames": ["jellyfin.example.com"],
                "parentRefs": [{"name": "kubarr-gateway", "namespace": "kubarr", "sectionName": "https"}],
                "rules": [{"matches": [{"path": {"type": "PathPrefix", "value": "/"}}],
                           "backendRefs": [{"name": "jellyfin", "port": 8096}]}]
            },
            "status": {"parents": [{"parentRef": {"name": "kubarr-gateway"},
                "conditions": [{"type": "Accepted", "status": "True", "reason": "Accepted", "message": "ok"}]}]}
        });
        assert_eq!(
            httproute_line(&meta, &data),
            "jellyfin hosts jellyfin.example.com -> jellyfin:8096"
        );
        let out = httproute_describe(&meta, &data).to_string();
        assert!(out.contains("kubarr/kubarr-gateway (https)"), "{out}");
        assert!(out.contains("PathPrefix /"), "{out}");
    }

    #[test]
    fn events_render_as_lines() {
        let event = Event {
            metadata: ObjectMeta::default(),
            involved_object: k8s_openapi::api::core::v1::ObjectReference {
                kind: Some("Pod".into()),
                name: Some("app-0".into()),
                ..Default::default()
            },
            type_: Some("Warning".into()),
            reason: Some("BackOff".into()),
            message: Some(format!("restarting; token={SECRET}")),
            count: Some(3),
            last_timestamp: Some(Time(jiff::Timestamp::from_second(1_700_000_000).unwrap())),
            ..Default::default()
        };
        let line = event_line(&event);
        assert_eq!(
            line,
            "2023-11-14T22:13:20Z Warning BackOff Pod/app-0: restarting; token=[redacted] (x3)"
        );
    }

    #[test]
    fn known_secret_values_are_scrubbed_from_shaped_output() {
        let mut pod = leaky_pod();
        let db = "Zq8-database-pass-77";
        if let Some(status) = pod.status.as_mut() {
            if let Some(cs) = status.container_statuses.as_mut() {
                cs[0].last_state = Some(ContainerState {
                    terminated: Some(k8s_openapi::api::core::v1::ContainerStateTerminated {
                        exit_code: 1,
                        reason: Some("Error".into()),
                        message: Some(format!("fatal: cannot connect with {db}")),
                        ..Default::default()
                    }),
                    ..Default::default()
                });
            }
        }
        let mut out = pod.describe();
        assert!(out.to_string().contains(db));
        scrub_value(&mut out, &[db.to_string()]);
        let text = out.to_string();
        assert!(!text.contains(db), "{text}");
        assert!(
            text.contains("fatal: cannot connect with [redacted]"),
            "{text}"
        );

        let mut lines = json!({"events": [format!("Warning Failed Pod/x: {db}")]});
        scrub_value(&mut lines, &[db.to_string()]);
        assert!(!lines.to_string().contains(db));
    }

    #[test]
    fn annotation_blobs_are_omitted_and_config_map_lines_capped() {
        let map = BTreeMap::from([
            ("example.com/config".to_string(), r#"{"a": 1}"#.to_string()),
            ("example.com/list".to_string(), "[1, 2]".to_string()),
            ("example.com/yaml".to_string(), "a: 1\nb: 2".to_string()),
            ("example.com/db-dsn".to_string(), "x".to_string()),
            ("example.com/connection-string".to_string(), "x".to_string()),
            ("example.com/private".to_string(), "x".to_string()),
            ("example.com/plain".to_string(), "fine".to_string()),
        ]);
        let out = shape_string_map(Some(&map)).to_string();
        assert!(out.contains(r#""example.com/config":"[omitted]""#), "{out}");
        assert!(out.contains(r#""example.com/list":"[omitted]""#), "{out}");
        assert!(out.contains(r#""example.com/yaml":"[omitted]""#), "{out}");
        assert!(
            out.contains(r#""example.com/db-dsn":"[redacted]""#),
            "{out}"
        );
        assert!(
            out.contains(r#""example.com/connection-string":"[redacted]""#),
            "{out}"
        );
        assert!(
            out.contains(r#""example.com/private":"[redacted]""#),
            "{out}"
        );
        assert!(out.contains(r#""example.com/plain":"fine""#), "{out}");

        let cm = ConfigMap {
            metadata: leaky_meta(),
            data: Some(
                (0..25)
                    .map(|i| (format!("k{i:02}"), String::new()))
                    .collect(),
            ),
            ..Default::default()
        };
        let line = cm.line(0);
        assert!(line.contains("k19") && !line.contains("k20"), "{line}");
        assert!(line.ends_with("…and 5 more"), "{line}");
    }

    #[test]
    fn ages_are_short() {
        assert_eq!(age(Some(0), 30), "30s");
        assert_eq!(age(Some(0), 600), "10m");
        assert_eq!(age(Some(0), 3 * 3600), "3h");
        assert_eq!(age(Some(0), 5 * 86_400), "5d");
        assert_eq!(age(None, 5), "?");
    }
}
