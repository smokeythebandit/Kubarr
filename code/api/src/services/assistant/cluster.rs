//! Filtered, read-only cluster tools: `k8s_list`, `k8s_describe`,
//! `k8s_events`, `pod_logs` and `helm_status`.
//!
//! Scope: administrators (`settings.manage`) see every namespace; everyone
//! else only the namespaces of their allowed apps that are owned by exactly
//! one app and are neither system namespaces nor of a catalog system app
//! (the same rule as metrics and logs). Cluster-scoped kinds (nodes,
//! persistent volumes) are admin-only; non-admins listing namespaces get
//! their allowed ones. Secrets are never read for the model: not listed, not
//! described. Output is shaped by [`super::k8s_shape`].
//!
//! `helm_status` runs `helm get metadata` and `helm history`, whose output
//! carries chart, version, revision and status but never values or
//! manifests; the result is whitelisted again before it is returned. (Helm
//! reads its release Secret itself, server-side; nothing from that Secret
//! but the metadata fields reaches the model.)

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::time::Duration;

use chrono::Utc;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{
    ConfigMap, Event, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod, Service,
};
use k8s_openapi::{ClusterResourceScope, NamespaceResourceScope};
use kube::api::{Api, ApiResource, DynamicObject, GroupVersionKind, ListParams, LogParams};
use kube::{Client, Resource};
use sea_orm::EntityTrait;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use super::k8s_shape::{
    event_line, event_seconds, httproute_describe, httproute_line, scrub_value, shape_logs, Kind,
    Shape,
};
use super::tools::{
    clip, is_app_name, scoped_namespace, system_namespace, ToolContext, ToolError, ToolResult,
};
use crate::models::app_state;
use crate::services::catalog::lifecycle_for_app_name;
use crate::services::helm_process;
use crate::services::update_diagnostics::{scrub_inline, scrub_with, try_secret_values};

/// At most this many objects per list call (first page only).
const LIST_LIMIT: u32 = 200;
const MAX_EVENTS: usize = 100;
const MAX_TAIL: u64 = 500;
const DEFAULT_TAIL: u64 = 100;
const MAX_SINCE_MINUTES: u64 = 24 * 60;
/// Raw pod logs fetched at most (before scrubbing and the 18 KB budget).
const LOG_FETCH_BYTES: i64 = 512 * 1024;
const HELM_TIMEOUT: Duration = Duration::from_secs(30);
/// Every Kubernetes API call is cut off after this long.
const KUBE_TIMEOUT: Duration = Duration::from_secs(15);

const UNREACHABLE: &str = "The Kubernetes cluster could not be read right now.";
const KUBE_SLOW: &str = "The Kubernetes API did not answer in time.";
const UNSCRUBBABLE: &str = "This is not shown because Kubarr could not read the namespace's \
     Secrets to redact their values from it.";
const NAMESPACE_HIDDEN: &str = "is not visible to this user (system and shared namespaces are \
     only available to administrators)";

/// A DNS-1123 label: 1-63 of `[a-z0-9-]`, starting and ending alphanumeric.
/// Namespaces, Helm releases and container names.
pub fn is_k8s_label(value: &str) -> bool {
    let alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let bytes = value.as_bytes();
    match (bytes.first(), bytes.last()) {
        (Some(&first), Some(&last)) => {
            bytes.len() <= 63
                && alnum(first)
                && alnum(last)
                && bytes.iter().all(|&b| alnum(b) || b == b'-')
        }
        _ => false,
    }
}

/// A DNS-1123 subdomain (object names): at most 253 characters, dot-separated
/// labels, so no leading `-`/`.`, no `..` and nothing but `[a-z0-9-.]`.
pub fn is_k8s_name(value: &str) -> bool {
    value.len() <= 253 && value.split('.').all(is_k8s_label)
}

/// Whether a non-admin may read `namespace`: owned by exactly one app, that
/// app is visible to them and not a catalog system app, and the namespace is
/// not a system namespace.
pub fn open_to_non_admin(
    namespace: &str,
    owners: &[&str],
    sees_app: impl Fn(&str) -> bool,
    system_app: impl Fn(&str) -> bool,
) -> bool {
    match owners {
        [owner] => {
            is_app_name(namespace)
                && !system_namespace(namespace)
                && sees_app(owner)
                && !system_app(owner)
        }
        _ => false,
    }
}

/// The namespaces a non-admin may read, from `(app, namespace)` pairs.
pub fn allowed_namespaces(
    apps: &[(String, String)],
    sees_app: impl Fn(&str) -> bool,
    system_app: impl Fn(&str) -> bool,
) -> BTreeSet<String> {
    let mut owners: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (app, namespace) in apps {
        owners.entry(namespace).or_default().push(app);
    }
    owners
        .into_iter()
        .filter(|(namespace, owners)| open_to_non_admin(namespace, owners, &sees_app, &system_app))
        .map(|(namespace, _)| namespace.to_string())
        .collect()
}

/// Which namespaces this user may read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    Only(BTreeSet<String>),
}

impl Scope {
    pub(super) fn check(&self, namespace: &str) -> Result<(), ToolError> {
        match self {
            Scope::All if is_k8s_label(namespace) => Ok(()),
            Scope::Only(set) if set.contains(namespace) => Ok(()),
            _ => Err(ToolError::new(format!(
                "The namespace \"{}\" {NAMESPACE_HIDDEN}.",
                clip(namespace, 64)
            ))),
        }
    }
}

pub(super) async fn scope(ctx: &ToolContext) -> Result<Scope, ToolError> {
    if ctx.is_admin() {
        return Ok(Scope::All);
    }
    let apps: Vec<(String, String)> = app_state::Entity::find()
        .all(&ctx.db)
        .await?
        .into_iter()
        .map(|state| (state.app_name, state.namespace))
        .collect();
    let catalog = ctx.state.catalog.read().await;
    Ok(Scope::Only(allowed_namespaces(
        &apps,
        |app| ctx.sees_app(app),
        |app| catalog.get_app(app).is_some_and(|config| config.is_system),
    )))
}

/// Whether the cluster tools are offered: the cluster is reachable and the
/// user may read at least one namespace.
pub async fn available(ctx: &ToolContext) -> bool {
    if ctx.state.k8s_client.read().await.is_none() {
        return false;
    }
    match scope(ctx).await {
        Ok(Scope::All) => true,
        Ok(Scope::Only(set)) => !set.is_empty(),
        Err(_) => false,
    }
}

async fn client(ctx: &ToolContext) -> Result<Client, ToolError> {
    ctx.state
        .k8s_client
        .read()
        .await
        .as_ref()
        .map(|k8s| k8s.client().clone())
        .ok_or_else(|| ToolError::new(UNREACHABLE))
}

fn kube_error(kind: Kind, what: &str, e: kube::Error) -> ToolError {
    match &e {
        kube::Error::Api(status) if status.code == 404 => {
            if kind == Kind::HttpRoute && what.is_empty() {
                ToolError::new("HTTPRoutes are not available: the Gateway API is not installed.")
            } else {
                ToolError::new(format!("No {} named \"{what}\" was found.", kind.name()))
            }
        }
        kube::Error::Api(status) if status.code == 403 => ToolError::new(format!(
            "Kubarr is not allowed to read {} in this cluster.",
            kind.plural_label()
        )),
        _ => {
            tracing::debug!(error = %e, "Assistant cluster read failed");
            ToolError::new(UNREACHABLE)
        }
    }
}

/// Await a Kubernetes call with [`KUBE_TIMEOUT`].
async fn call<T>(
    kind: Kind,
    what: &str,
    fut: impl std::future::Future<Output = Result<T, kube::Error>>,
) -> Result<T, ToolError> {
    match tokio::time::timeout(KUBE_TIMEOUT, fut).await {
        Ok(result) => result.map_err(|e| kube_error(kind, what, e)),
        Err(_) => Err(ToolError::new(KUBE_SLOW)),
    }
}

/// Known Secret values of the namespaces a result covers, for scrubbing.
/// Fails (so the result is withheld) when they cannot be read.
async fn secrets_for(ctx: &ToolContext, targets: &Targets) -> Result<Vec<String>, ToolError> {
    let guard = ctx.state.k8s_client.read().await;
    let k8s = guard.as_ref().ok_or_else(|| ToolError::new(UNREACHABLE))?;
    let read = async {
        match targets {
            Targets::All => try_secret_values(k8s, None).await,
            Targets::Some(namespaces) => {
                let mut all = Vec::new();
                for ns in namespaces {
                    all.extend(try_secret_values(k8s, Some(ns)).await?);
                }
                Some(all)
            }
        }
    };
    match tokio::time::timeout(KUBE_TIMEOUT, read).await {
        Ok(Some(values)) => Ok(values),
        _ => Err(ToolError::new(UNSCRUBBABLE)),
    }
}

fn kind_of(input: &Value) -> Result<Kind, ToolError> {
    Kind::parse(input["kind"].as_str().unwrap_or_default()).map_err(ToolError::new)
}

/// Namespaces, releases and containers are DNS labels; object names are
/// DNS subdomains.
fn label_key(key: &str) -> bool {
    matches!(key, "namespace" | "release" | "container")
}

fn name_input(input: &Value, key: &str) -> Result<Option<String>, ToolError> {
    let valid = |v: &str| {
        if label_key(key) {
            is_k8s_label(v)
        } else {
            is_k8s_name(v)
        }
    };
    match input[key].as_str() {
        None => Ok(None),
        Some(value) if valid(value) => Ok(Some(value.to_string())),
        Some(_) => Err(ToolError::new(format!(
            "`{key}` must be a valid Kubernetes name (lowercase letters, digits and '-'{}, \
             starting and ending with a letter or digit).",
            if label_key(key) {
                ""
            } else {
                ", '.' between parts"
            }
        ))),
    }
}

fn required_name(input: &Value, key: &str) -> Result<String, ToolError> {
    name_input(input, key)?.ok_or_else(|| ToolError::new(format!("`{key}` is required.")))
}

/// Namespaces a namespaced list or event query reads.
enum Targets {
    All,
    Some(Vec<String>),
}

impl Targets {
    fn resolve(scope: &Scope, namespace: Option<&str>) -> Result<Targets, ToolError> {
        match (namespace, scope) {
            (Some(ns), _) => {
                scope.check(ns)?;
                Ok(Targets::Some(vec![ns.to_string()]))
            }
            (None, Scope::All) => Ok(Targets::All),
            (None, Scope::Only(set)) => Ok(Targets::Some(set.iter().cloned().collect())),
        }
    }

    fn several(&self) -> bool {
        match self {
            Targets::All => true,
            Targets::Some(list) => list.len() > 1,
        }
    }
}

fn admin_only_error(kind: Kind) -> ToolError {
    ToolError::new(format!(
        "{} are cluster-wide and only available to administrators.",
        kind.plural_label()
    ))
}

/// `(items, more_not_shown)`.
async fn fetch_namespaced<K>(
    client: &Client,
    targets: &Targets,
    kind: Kind,
) -> Result<(Vec<K>, bool), ToolError>
where
    K: Resource<Scope = NamespaceResourceScope, DynamicType = ()>
        + Clone
        + DeserializeOwned
        + Debug,
{
    let params = ListParams::default().limit(LIST_LIMIT);
    let mut items = Vec::new();
    let mut more = false;
    let namespaces = match targets {
        Targets::All => {
            let list = call(kind, "", Api::<K>::all(client.clone()).list(&params)).await?;
            more = list.metadata.continue_.is_some_and(|c| !c.is_empty());
            return Ok((list.items, more));
        }
        Targets::Some(namespaces) => namespaces,
    };
    for ns in namespaces {
        let list = call(
            kind,
            "",
            Api::<K>::namespaced(client.clone(), ns).list(&params),
        )
        .await?;
        more |= list.metadata.continue_.is_some_and(|c| !c.is_empty());
        items.extend(list.items);
        if items.len() >= LIST_LIMIT as usize {
            more = true;
            items.truncate(LIST_LIMIT as usize);
            break;
        }
    }
    Ok((items, more))
}

async fn fetch_cluster<K>(client: &Client, kind: Kind) -> Result<(Vec<K>, bool), ToolError>
where
    K: Resource<Scope = ClusterResourceScope, DynamicType = ()> + Clone + DeserializeOwned + Debug,
{
    let list = call(
        kind,
        "",
        Api::<K>::all(client.clone()).list(&ListParams::default().limit(LIST_LIMIT)),
    )
    .await?;
    let more = list.metadata.continue_.is_some_and(|c| !c.is_empty());
    Ok((list.items, more))
}

fn lines_of<K: Shape>(items: &[K], prefix: bool, now: i64) -> Vec<String> {
    items
        .iter()
        .map(|item| {
            let line = item.line(now);
            match (prefix, item.meta().namespace.as_deref()) {
                (true, Some(ns)) => format!("{ns}/{line}"),
                _ => line,
            }
        })
        .collect()
}

async fn list_lines<K>(
    client: &Client,
    targets: &Targets,
    kind: Kind,
    now: i64,
) -> Result<(Vec<String>, bool), ToolError>
where
    K: Shape
        + Resource<Scope = NamespaceResourceScope, DynamicType = ()>
        + Clone
        + DeserializeOwned
        + Debug,
{
    let (items, more) = fetch_namespaced::<K>(client, targets, kind).await?;
    Ok((lines_of(&items, targets.several(), now), more))
}

fn httproute_resource() -> ApiResource {
    ApiResource::from_gvk_with_plural(
        &GroupVersionKind::gvk("gateway.networking.k8s.io", "v1", "HTTPRoute"),
        "httproutes",
    )
}

async fn list_httproutes(
    client: &Client,
    targets: &Targets,
) -> Result<(Vec<String>, bool), ToolError> {
    let resource = httproute_resource();
    let params = ListParams::default().limit(LIST_LIMIT);
    let apis: Vec<Api<DynamicObject>> = match targets {
        Targets::All => vec![Api::all_with(client.clone(), &resource)],
        Targets::Some(namespaces) => namespaces
            .iter()
            .map(|ns| Api::namespaced_with(client.clone(), ns, &resource))
            .collect(),
    };
    let mut lines = Vec::new();
    let mut more = false;
    for api in apis {
        let list = call(Kind::HttpRoute, "", api.list(&params)).await?;
        more |= list.metadata.continue_.is_some_and(|c| !c.is_empty());
        for route in &list.items {
            let line = httproute_line(&route.metadata, &route.data);
            lines.push(
                match (targets.several(), route.metadata.namespace.as_deref()) {
                    (true, Some(ns)) => format!("{ns}/{line}"),
                    _ => line,
                },
            );
        }
    }
    Ok((lines, more))
}

pub(super) async fn k8s_list(ctx: &ToolContext, input: &Value) -> ToolResult {
    let kind = kind_of(input)?;
    let namespace = name_input(input, "namespace")?;
    let scope = scope(ctx).await?;
    if kind.admin_only() && scope != Scope::All {
        return Err(admin_only_error(kind));
    }
    let client = client(ctx).await?;
    let now = Utc::now().timestamp();
    let mut scrub_targets = None;
    let (mut lines, more) = match kind {
        Kind::Namespace => match &scope {
            Scope::All => {
                let (items, more) = fetch_cluster::<Namespace>(&client, kind).await?;
                (lines_of(&items, false, now), more)
            }
            Scope::Only(set) => {
                let api = Api::<Namespace>::all(client.clone());
                let mut lines = Vec::new();
                for ns in set {
                    match call(kind, ns, api.get(ns)).await {
                        Ok(object) => lines.push(object.line(now)),
                        Err(ToolError(e)) => {
                            tracing::debug!(error = %e, "Assistant namespace read failed");
                        }
                    }
                }
                (lines, false)
            }
        },
        Kind::Node => {
            let (items, more) = fetch_cluster::<Node>(&client, kind).await?;
            (lines_of(&items, false, now), more)
        }
        Kind::PersistentVolume => {
            let (items, more) = fetch_cluster::<PersistentVolume>(&client, kind).await?;
            (lines_of(&items, false, now), more)
        }
        _ => {
            let targets = Targets::resolve(&scope, namespace.as_deref())?;
            let found = match kind {
                Kind::Pod => list_lines::<Pod>(&client, &targets, kind, now).await?,
                Kind::Deployment => list_lines::<Deployment>(&client, &targets, kind, now).await?,
                Kind::StatefulSet => {
                    list_lines::<StatefulSet>(&client, &targets, kind, now).await?
                }
                Kind::DaemonSet => list_lines::<DaemonSet>(&client, &targets, kind, now).await?,
                Kind::Job => list_lines::<Job>(&client, &targets, kind, now).await?,
                Kind::CronJob => list_lines::<CronJob>(&client, &targets, kind, now).await?,
                Kind::Service => list_lines::<Service>(&client, &targets, kind, now).await?,
                Kind::PersistentVolumeClaim => {
                    list_lines::<PersistentVolumeClaim>(&client, &targets, kind, now).await?
                }
                Kind::ConfigMap => list_lines::<ConfigMap>(&client, &targets, kind, now).await?,
                Kind::HttpRoute => list_httproutes(&client, &targets).await?,
                Kind::Event => {
                    let (mut items, more) =
                        fetch_namespaced::<Event>(&client, &targets, kind).await?;
                    items.sort_by_key(|e| std::cmp::Reverse(event_seconds(e)));
                    (lines_of(&items, targets.several(), now), more)
                }
                Kind::Namespace | Kind::Node | Kind::PersistentVolume => (Vec::new(), false),
            };
            scrub_targets = Some(targets);
            found
        }
    };
    lines.truncate(LIST_LIMIT as usize);
    let mut result = json!({
        "kind": kind.name(),
        "namespace": if kind.cluster_scoped() { Value::Null } else {
            json!(namespace.unwrap_or_else(|| "all visible namespaces".into()))
        },
        "count": lines.len(),
        "items": lines,
    });
    if more {
        result["note"] = json!("More objects exist than are shown; narrow by namespace.");
    }
    if let Some(targets) = scrub_targets {
        scrub_value(&mut result, &secrets_for(ctx, &targets).await?);
    }
    Ok(result)
}

async fn get_namespaced<K>(client: &Client, ns: &str, name: &str, kind: Kind) -> ToolResult
where
    K: Shape
        + Resource<Scope = NamespaceResourceScope, DynamicType = ()>
        + Clone
        + DeserializeOwned
        + Debug,
{
    let object = call(
        kind,
        &format!("{ns}/{name}"),
        Api::<K>::namespaced(client.clone(), ns).get(name),
    )
    .await?;
    Ok(object.describe())
}

async fn get_cluster<K>(client: &Client, name: &str, kind: Kind) -> ToolResult
where
    K: Shape
        + Resource<Scope = ClusterResourceScope, DynamicType = ()>
        + Clone
        + DeserializeOwned
        + Debug,
{
    let object = call(kind, name, Api::<K>::all(client.clone()).get(name)).await?;
    Ok(object.describe())
}

pub(super) async fn k8s_describe(ctx: &ToolContext, input: &Value) -> ToolResult {
    let kind = kind_of(input)?;
    let name = required_name(input, "name")?;
    let namespace = name_input(input, "namespace")?;
    let scope = scope(ctx).await?;
    if kind.admin_only() && scope != Scope::All {
        return Err(admin_only_error(kind));
    }
    let client = client(ctx).await?;
    match kind {
        Kind::Namespace => {
            scope.check(&name)?;
            let mut result = get_cluster::<Namespace>(&client, &name, kind).await?;
            let secrets = secrets_for(ctx, &Targets::Some(vec![name.clone()])).await?;
            scrub_value(&mut result, &secrets);
            Ok(result)
        }
        // Admin-only: scrubbed with every namespace's Secret values.
        Kind::Node | Kind::PersistentVolume => {
            let mut result = if kind == Kind::Node {
                get_cluster::<Node>(&client, &name, kind).await?
            } else {
                get_cluster::<PersistentVolume>(&client, &name, kind).await?
            };
            scrub_value(&mut result, &secrets_for(ctx, &Targets::All).await?);
            Ok(result)
        }
        _ => {
            let ns = namespace.ok_or_else(|| {
                ToolError::new(format!("`namespace` is required for a {}.", kind.name()))
            })?;
            scope.check(&ns)?;
            let (ns, name) = (ns.as_str(), name.as_str());
            let mut result = match kind {
                Kind::Pod => get_namespaced::<Pod>(&client, ns, name, kind).await,
                Kind::Deployment => get_namespaced::<Deployment>(&client, ns, name, kind).await,
                Kind::StatefulSet => get_namespaced::<StatefulSet>(&client, ns, name, kind).await,
                Kind::DaemonSet => get_namespaced::<DaemonSet>(&client, ns, name, kind).await,
                Kind::Job => get_namespaced::<Job>(&client, ns, name, kind).await,
                Kind::CronJob => get_namespaced::<CronJob>(&client, ns, name, kind).await,
                Kind::Service => get_namespaced::<Service>(&client, ns, name, kind).await,
                Kind::PersistentVolumeClaim => {
                    get_namespaced::<PersistentVolumeClaim>(&client, ns, name, kind).await
                }
                Kind::Event => get_namespaced::<Event>(&client, ns, name, kind).await,
                Kind::ConfigMap => get_namespaced::<ConfigMap>(&client, ns, name, kind).await,
                Kind::HttpRoute => {
                    let resource = httproute_resource();
                    let api = Api::<DynamicObject>::namespaced_with(client.clone(), ns, &resource);
                    let route = call(kind, &format!("{ns}/{name}"), api.get(name)).await?;
                    Ok(httproute_describe(&route.metadata, &route.data))
                }
                Kind::Namespace | Kind::Node | Kind::PersistentVolume => {
                    Err(ToolError::new("Unsupported kind."))
                }
            }?;
            let secrets = secrets_for(ctx, &Targets::Some(vec![ns.to_string()])).await?;
            scrub_value(&mut result, &secrets);
            Ok(result)
        }
    }
}

pub(super) async fn k8s_events(ctx: &ToolContext, input: &Value) -> ToolResult {
    let namespace = name_input(input, "namespace")?;
    let minutes = input["since_minutes"]
        .as_u64()
        .unwrap_or(60)
        .clamp(1, MAX_SINCE_MINUTES);
    let scope = scope(ctx).await?;
    let targets = Targets::resolve(&scope, namespace.as_deref())?;
    let client = client(ctx).await?;
    let (mut events, _) = fetch_namespaced::<Event>(&client, &targets, Kind::Event).await?;
    let cutoff = Utc::now().timestamp() - (minutes as i64) * 60;
    events.retain(|e| event_seconds(e).is_some_and(|s| s >= cutoff));
    events.sort_by_key(|e| std::cmp::Reverse(event_seconds(e)));
    let total = events.len();
    events.truncate(MAX_EVENTS);
    let several = targets.several();
    let lines: Vec<String> = events
        .iter()
        .map(|e| match (several, e.metadata.namespace.as_deref()) {
            (true, Some(ns)) => format!("{ns} {}", event_line(e)),
            _ => event_line(e),
        })
        .collect();
    let mut result = json!({
        "namespace": namespace.unwrap_or_else(|| "all visible namespaces".into()),
        "since_minutes": minutes,
        "count": lines.len(),
        "events": lines,
    });
    if total > MAX_EVENTS {
        result["note"] = json!(format!(
            "{total} events matched; the newest {MAX_EVENTS} are shown."
        ));
    }
    scrub_value(&mut result, &secrets_for(ctx, &targets).await?);
    Ok(result)
}

pub(super) async fn pod_logs(ctx: &ToolContext, input: &Value) -> ToolResult {
    let namespace = required_name(input, "namespace")?;
    let pod = required_name(input, "pod")?;
    let container = name_input(input, "container")?;
    let previous = input["previous"].as_bool().unwrap_or(false);
    let tail = input["tail"]
        .as_u64()
        .unwrap_or(DEFAULT_TAIL)
        .clamp(1, MAX_TAIL);
    scope(ctx).await?.check(&namespace)?;
    let client = client(ctx).await?;
    let params = LogParams {
        tail_lines: Some(tail as i64),
        container: container.clone(),
        previous,
        limit_bytes: Some(LOG_FETCH_BYTES),
        ..Default::default()
    };
    // Refuse up front when the Secrets to scrub with cannot be read.
    let secrets = secrets_for(ctx, &Targets::Some(vec![namespace.clone()])).await?;
    let api = Api::<Pod>::namespaced(client, &namespace);
    let raw = match tokio::time::timeout(KUBE_TIMEOUT, api.logs(&pod, &params)).await {
        Err(_) => return Err(ToolError::new(KUBE_SLOW)),
        Ok(Ok(raw)) => raw,
        Ok(Err(kube::Error::Api(status))) if status.code == 400 => {
            return Err(ToolError::new(format!(
                "Kubernetes could not return these logs: {}",
                clip(&scrub_with(&status.message, &secrets), 300)
            )))
        }
        Ok(Err(e)) => return Err(kube_error(Kind::Pod, &format!("{namespace}/{pod}"), e)),
    };
    let (log, lines) = shape_logs(&raw, &secrets);
    Ok(json!({
        "namespace": namespace,
        "pod": pod,
        "container": container,
        "previous": previous,
        "lines": lines,
        "log": log,
    }))
}

/// The whitelisted fields of `helm get metadata -o json`.
#[derive(Debug, Deserialize)]
struct HelmMetadata {
    name: Option<String>,
    namespace: Option<String>,
    chart: Option<String>,
    version: Option<String>,
    #[serde(rename = "appVersion")]
    app_version: Option<String>,
    revision: Option<u32>,
    status: Option<String>,
    #[serde(rename = "deployedAt")]
    deployed_at: Option<String>,
}

/// The whitelisted fields of one `helm history -o json` entry.
#[derive(Debug, Deserialize)]
struct HelmRevision {
    revision: u32,
    updated: Option<String>,
    status: Option<String>,
    chart: Option<String>,
    app_version: Option<String>,
    description: Option<String>,
}

/// Release metadata without values, manifests or chart annotations.
pub fn release_metadata_json(bytes: &[u8]) -> Option<Value> {
    let m: HelmMetadata = serde_json::from_slice(bytes).ok()?;
    Some(json!({
        "release": m.name,
        "namespace": m.namespace,
        "chart": m.chart,
        "version": m.version,
        "app_version": m.app_version,
        "revision": m.revision,
        "status": m.status,
        "deployed_at": m.deployed_at,
    }))
}

/// Recent revisions, newest first, descriptions scrubbed and clipped.
pub fn release_history_json(bytes: &[u8], max: usize) -> Vec<Value> {
    let mut entries: Vec<HelmRevision> = serde_json::from_slice(bytes).unwrap_or_default();
    entries.sort_by_key(|e| std::cmp::Reverse(e.revision));
    entries
        .into_iter()
        .take(max)
        .map(|e| {
            json!({
                "revision": e.revision,
                "updated": e.updated,
                "status": e.status,
                "chart": e.chart,
                "app_version": e.app_version,
                "description": e.description.map(|d| clip(&scrub_inline(&d), 300)),
            })
        })
        .collect()
}

/// `helm <command…> -n <ns> -o json -- <release>`: the release goes after
/// `--` so it can never be read as a flag (names are validated as well).
pub fn helm_args(command: &[&str], release: &str, namespace: &str) -> Vec<String> {
    command
        .iter()
        .copied()
        .chain(["-n", namespace, "-o", "json", "--", release])
        .map(str::to_string)
        .collect()
}

async fn helm(args: Vec<String>) -> Result<helm_process::HelmOutput, ToolError> {
    tokio::task::spawn_blocking(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        helm_process::run(&args, HELM_TIMEOUT)
    })
    .await
    .map_err(|_| ToolError::new("Helm could not be run right now."))?
    .map_err(|e| {
        tracing::debug!(error = %e, "Assistant helm call failed");
        ToolError::new("Helm could not be run right now.")
    })
}

pub(super) async fn helm_status(ctx: &ToolContext, input: &Value) -> ToolResult {
    let release = required_name(input, "release")?;
    let namespace = name_input(input, "namespace")?;
    let hidden = || {
        ToolError::new(format!(
            "No Helm release named \"{release}\" is visible to this user."
        ))
    };
    let owner = app_state::Entity::find()
        .all(&ctx.db)
        .await?
        .into_iter()
        .find(|state| lifecycle_for_app_name(&state.app_name).release_name == release);
    let release_namespace = match (&owner, ctx.is_admin()) {
        (Some(state), false) => {
            // Same rule as metrics and logs: the app is visible, owns its
            // namespace alone and is not a system app.
            scoped_namespace(ctx, &state.app_name)
                .await
                .map_err(|_| hidden())?;
            lifecycle_for_app_name(&state.app_name).release_namespace
        }
        (None, false) => return Err(hidden()),
        (Some(state), true) => {
            namespace.unwrap_or_else(|| lifecycle_for_app_name(&state.app_name).release_namespace)
        }
        (None, true) => namespace.unwrap_or_else(|| release.clone()),
    };
    let metadata = helm(helm_args(
        &["get", "metadata"],
        &release,
        &release_namespace,
    ))
    .await?;
    if !metadata.status.success() {
        let stderr = String::from_utf8_lossy(&metadata.stderr).to_ascii_lowercase();
        if stderr.contains("not found") {
            return Err(ToolError::new(format!(
                "No Helm release named \"{release}\" in namespace {release_namespace}."
            )));
        }
        tracing::debug!(%stderr, "Assistant helm metadata failed");
        return Err(ToolError::new(
            "Helm could not read this release right now.",
        ));
    }
    let mut result = release_metadata_json(&metadata.stdout)
        .ok_or_else(|| ToolError::new("Helm returned metadata Kubarr could not read."))?;
    let history = helm(helm_args(
        &["history", "--max", "5"],
        &release,
        &release_namespace,
    ))
    .await?;
    if history.status.success() {
        result["recent_revisions"] = json!(release_history_json(&history.stdout, 5));
    }
    if let Some(state) = owner {
        result["kubarr_app"] = json!(state.app_name);
    }
    result["note"] = json!("Chart values and manifests are never shown.");
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps() -> Vec<(String, String)> {
        [
            ("jellyfin", "jellyfin"),
            ("sonarr", "sonarr"),
            ("radarr", "media"),
            ("lidarr", "media"),
            ("victoriametrics", "victoriametrics"),
            ("dashboard", "dashboard"),
        ]
        .into_iter()
        .map(|(a, n)| (a.to_string(), n.to_string()))
        .collect()
    }

    #[test]
    fn non_admins_only_get_their_own_unshared_non_system_namespaces() {
        let visible = ["jellyfin", "radarr", "victoriametrics", "dashboard"];
        let set = allowed_namespaces(
            &apps(),
            |app| visible.contains(&app),
            |app| app == "dashboard",
        );
        // sonarr: not allowed; media: shared by two apps; victoriametrics:
        // system namespace; dashboard: catalog system app.
        assert_eq!(set, BTreeSet::from(["jellyfin".to_string()]));

        let scope = Scope::Only(set);
        assert!(scope.check("jellyfin").is_ok());
        for ns in [
            "sonarr",
            "media",
            "kube-system",
            "victoriametrics",
            "kubarr",
        ] {
            let err = scope.check(ns).err().map(|e| e.0).unwrap_or_default();
            assert!(err.contains("not visible"), "{ns}: {err}");
        }
        assert!(Scope::All.check("kube-system").is_ok());
        assert!(Scope::All.check("Bad Name").is_err());
    }

    #[test]
    fn k8s_names_are_validated() {
        assert!(is_k8s_name("jellyfin-7d9f.abc"));
        assert!(is_k8s_label("jellyfin-0"));
        let long_label = "a".repeat(64);
        let long_name = "a.".repeat(127);
        for bad in [
            "",
            "Jellyfin",
            "a/b",
            "a b",
            "x\"",
            "--kube-context",
            "-x",
            "..",
            ".",
            "a..b",
            ".a",
            "a.",
            "a-",
            &long_name,
        ] {
            assert!(!is_k8s_name(bad), "{bad}");
            assert!(!is_k8s_label(bad), "{bad}");
        }
        assert!(!is_k8s_label(&long_label));
        assert!(!is_k8s_label("a.b"), "labels have no dots");
        // Validation happens at the tool input.
        for bad in ["--kube-context", "..", "-x"] {
            for key in ["namespace", "release", "name", "pod", "container"] {
                assert!(
                    name_input(&json!({ key: bad }), key).is_err(),
                    "{key}={bad}"
                );
            }
        }
        assert!(name_input(&json!({"namespace": "a.b"}), "namespace").is_err());
        assert!(name_input(&json!({"name": "a.b"}), "name").is_ok());
    }

    #[test]
    fn helm_release_goes_after_double_dash() {
        let args = helm_args(&["get", "metadata"], "jellyfin", "jellyfin");
        assert_eq!(
            args,
            ["get", "metadata", "-n", "jellyfin", "-o", "json", "--", "jellyfin"]
        );
    }

    #[test]
    fn helm_output_is_whitelisted() {
        let metadata = br#"{"name":"jellyfin","chart":"jellyfin","version":"1.2.3",
            "appVersion":"10.10.0","annotations":{"secret":"s3cr3t-annotation"},
            "namespace":"jellyfin","revision":7,"status":"deployed",
            "deployedAt":"2026-11-20T10:00:00Z","values":{"password":"s3cr3t-value"},
            "manifest":"kind: Secret"}"#;
        let out = release_metadata_json(metadata)
            .unwrap_or_default()
            .to_string();
        assert!(out.contains(r#""revision":7"#), "{out}");
        assert!(out.contains(r#""status":"deployed""#), "{out}");
        assert!(out.contains("10.10.0"), "{out}");
        assert!(!out.contains("s3cr3t"), "{out}");
        assert!(
            !out.contains("manifest") && !out.contains("kind: Secret"),
            "{out}"
        );

        let history = br#"[
            {"revision":5,"updated":"a","status":"superseded","chart":"jellyfin-1.2.1","app_version":"10.9","description":"Upgrade complete"},
            {"revision":7,"updated":"c","status":"failed","chart":"jellyfin-1.2.3","app_version":"10.10","description":"Upgrade failed: token=abcdef123"},
            {"revision":6,"updated":"b","status":"superseded","chart":"jellyfin-1.2.2","app_version":"10.9","description":"Upgrade complete"}]"#;
        let revisions = release_history_json(history, 2);
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0]["revision"], 7);
        assert_eq!(revisions[1]["revision"], 6);
        let text = json!(revisions).to_string();
        assert!(!text.contains("abcdef123"), "{text}");
        assert!(release_metadata_json(b"not json").is_none());
    }
}
