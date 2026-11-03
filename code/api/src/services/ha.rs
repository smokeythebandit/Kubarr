//! Scales Kubarr's HA system releases with the number of usable cluster nodes.
//!
//! Every HA chart reads `kubarr.ha.nodeCount` and derives its own replica
//! count from it. The worker leader recounts nodes periodically and upgrades
//! any installed HA release whose applied value differs from the desired one.
//!
//! Scale-up waits until a node has been Ready for [`READY_DEBOUNCE`]; scale-down
//! only happens once a node is deleted, or has stayed cordoned or tainted away
//! for [`WITHHELD_GRACE`] — never merely because it went NotReady or is being
//! drained for maintenance.
//!
//! The same loop releases a database instance whose node-local volume
//! belonged to a deleted node, so CloudNativePG re-creates it on a live one.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::{Node, PersistentVolume, PersistentVolumeClaim, Pod};
use kube::api::{Api, DeleteParams, ListParams};
use sea_orm::sea_query::OnConflict;
use sea_orm::{DatabaseConnection, EntityTrait, Set};
use tokio_util::sync::CancellationToken;

use crate::config::CONFIG;
use crate::error::{AppError, Result};
use crate::models::system_setting;
use crate::services::app_manager::AppManager;
use crate::services::catalog::lifecycle_for_app_name;
use crate::services::deployment::DeploymentManager;
use crate::services::helm_process;

pub const HA_NODE_COUNT_SETTING: &str = "ha_node_count";

pub const HA_RELEASES: &[&str] = &[
    "openresty",
    "kubarr-backend",
    "kubarr-frontend",
    "kubarr-worker",
    "postgresql",
    "cert-manager",
    "kubarr-dns-webhook",
    "cloudflared",
];

/// A node counts toward scale-up only after being Ready this long.
pub const READY_DEBOUNCE: chrono::Duration = chrono::Duration::seconds(120);

/// A cordoned or tainted node leaves the pool only after staying so this long;
/// a drain for maintenance is shorter and must not resize anything.
pub const WITHHELD_GRACE: chrono::Duration = chrono::Duration::minutes(30);

const HA_VALUE: &str = "kubarr.ha.nodeCount";
const CERT_MANAGER_MAX_REPLICAS: u32 = 2;

/// Taints the node lifecycle controller adds while a node is unhealthy. They
/// mark a node as temporarily unusable, not as removed from Kubarr's pool.
const CONDITION_TAINTS: &[&str] = &[
    "node.kubernetes.io/not-ready",
    "node.kubernetes.io/unreachable",
    "node.kubernetes.io/memory-pressure",
    "node.kubernetes.io/disk-pressure",
    "node.kubernetes.io/pid-pressure",
    "node.kubernetes.io/network-unavailable",
];

/// Nodes ready to receive new replicas right now: Ready for at least
/// [`READY_DEBOUNCE`], not cordoned, and carrying no repelling taint.
pub fn schedulable_node_count(nodes: &[Node], now: DateTime<Utc>) -> u32 {
    let ready_before = (now - READY_DEBOUNCE).timestamp();
    count(nodes, |node| {
        !deleting(node)
            && !withheld(node, false)
            && ready_condition(node).is_some_and(|condition| {
                condition.status == "True"
                    && condition
                        .last_transition_time
                        .as_ref()
                        .is_some_and(|since| since.0.as_second() <= ready_before)
            })
    })
}

/// Nodes that still belong to the pool whatever their health. A NotReady node
/// is retained; a deleted node is not, nor is one in `removed` (cordoned or
/// tainted for at least [`WITHHELD_GRACE`], see [`WithheldTracker`]).
pub fn retained_node_count(nodes: &[Node], removed: &HashSet<String>) -> u32 {
    count(nodes, |node| {
        !deleting(node) && !(withheld(node, true) && removed.contains(node_name(node)))
    })
}

/// Remembers since when each node has been cordoned or carried a repelling
/// taint. Kept in the leader's memory only: a new leader starts over, which
/// delays a scale-down rather than causing one.
#[derive(Debug, Default)]
pub struct WithheldTracker {
    since: HashMap<String, DateTime<Utc>>,
}

impl WithheldTracker {
    /// Record the current node list and return the nodes withheld for at
    /// least [`WITHHELD_GRACE`] without interruption.
    pub fn observe(&mut self, nodes: &[Node], now: DateTime<Utc>) -> HashSet<String> {
        let withheld: HashSet<&str> = nodes
            .iter()
            .filter(|node| withheld(node, true))
            .map(node_name)
            .collect();
        self.since
            .retain(|name, _| withheld.contains(name.as_str()));
        for name in withheld {
            self.since.entry(name.to_string()).or_insert(now);
        }
        self.since
            .iter()
            .filter(|(_, since)| now - **since >= WITHHELD_GRACE)
            .map(|(name, _)| name.clone())
            .collect()
    }
}

/// The node count HA releases should be sized for, given the last applied one.
pub fn desired_node_count(applied: u32, schedulable: u32, retained: u32) -> u32 {
    let desired = if schedulable > applied {
        schedulable
    } else if retained < applied {
        retained
    } else {
        applied
    };
    desired.max(1)
}

/// Namespace and CloudNativePG cluster of the Kubarr database.
const DB_NAMESPACE: &str = "kubarr-database";
const DB_CLUSTER: &str = "kubarr-db";
const HOSTNAME_LABEL: &str = "kubernetes.io/hostname";

/// Database instance PVCs, and the Pods mounting them, to delete because
/// their volume is pinned to a node that no longer exists.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LostInstances {
    pub claims: Vec<String>,
    pub pods: Vec<String>,
}

impl LostInstances {
    pub fn is_empty(&self) -> bool {
        self.claims.is_empty() && self.pods.is_empty()
    }
}

/// Instance PVCs whose bound volume is node-local (required node affinity on
/// a hostname) and names only nodes absent from `nodes`, plus the Pods that
/// mount them. CloudNativePG re-creates such an instance on a live node once
/// both are gone; left alone it stays Pending forever.
///
/// Only deletion counts: a node that still exists, however unhealthy, keeps
/// its volume. Nothing is released when the node list is empty or when no
/// instance would survive, since then there is no replica to re-clone from
/// and the claims hold the only copies of the data.
pub fn lost_database_instances(
    claims: &[PersistentVolumeClaim],
    volumes: &[PersistentVolume],
    pods: &[Pod],
    nodes: &[Node],
) -> LostInstances {
    let live: HashSet<&str> = nodes
        .iter()
        .flat_map(|node| {
            let label = node
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get(HOSTNAME_LABEL))
                .map(String::as_str);
            std::iter::once(node_name(node)).chain(label)
        })
        .filter(|name| !name.is_empty())
        .collect();
    if live.is_empty() {
        return LostInstances::default();
    }
    let volumes: HashMap<&str, &PersistentVolume> = volumes
        .iter()
        .filter_map(|volume| Some((volume.metadata.name.as_deref()?, volume)))
        .collect();

    let mut lost = Vec::new();
    let mut surviving = 0;
    for claim in claims {
        let Some(name) = claim.metadata.name.as_deref() else {
            continue;
        };
        // An unbound claim, or one whose volume is gone, holds no data and
        // proves nothing either way.
        let Some(volume) = claim
            .spec
            .as_ref()
            .and_then(|spec| spec.volume_name.as_deref())
            .and_then(|volume| volumes.get(volume))
        else {
            continue;
        };
        let hosts = pinned_hosts(volume);
        if !hosts.is_empty() && hosts.iter().all(|host| !live.contains(host)) {
            lost.push(name.to_string());
        } else {
            surviving += 1;
        }
    }
    if surviving == 0 {
        return LostInstances::default();
    }

    let pods = pods
        .iter()
        .filter(|pod| {
            pod.spec
                .iter()
                .flat_map(|spec| spec.volumes.iter().flatten())
                .filter_map(|volume| volume.persistent_volume_claim.as_ref())
                .any(|source| lost.contains(&source.claim_name))
        })
        .filter_map(|pod| pod.metadata.name.clone())
        .collect();
    LostInstances { claims: lost, pods }
}

/// Hostnames a volume's required node affinity pins it to; empty when the
/// volume is not node-local.
fn pinned_hosts(volume: &PersistentVolume) -> Vec<&str> {
    let terms = volume
        .spec
        .as_ref()
        .and_then(|spec| spec.node_affinity.as_ref())
        .and_then(|affinity| affinity.required.as_ref())
        .map(|required| required.node_selector_terms.as_slice())
        .unwrap_or_default();
    terms
        .iter()
        .flat_map(|term| {
            let expressions = term
                .match_expressions
                .iter()
                .flatten()
                .filter(|req| req.key == HOSTNAME_LABEL);
            let fields = term
                .match_fields
                .iter()
                .flatten()
                .filter(|req| req.key == "metadata.name");
            expressions.chain(fields)
        })
        .filter(|req| req.operator == "In")
        .flat_map(|req| req.values.iter().flatten().map(String::as_str))
        .collect()
}

/// Delete the database instances [`lost_database_instances`] finds: Pods
/// first, so the PVC protection finalizer lets the claims go.
async fn release_lost_database_instances(client: kube::Client, nodes: &[Node]) -> Result<()> {
    let selector = ListParams::default().labels(&format!("cnpg.io/cluster={DB_CLUSTER}"));
    let claims_api = Api::<PersistentVolumeClaim>::namespaced(client.clone(), DB_NAMESPACE);
    let claims = claims_api.list(&selector).await?.items;
    if claims.is_empty() {
        return Ok(());
    }
    let volumes_api = Api::<PersistentVolume>::all(client.clone());
    let mut volumes = Vec::new();
    for name in claims
        .iter()
        .filter_map(|claim| claim.spec.as_ref()?.volume_name.as_deref())
    {
        if let Some(volume) = volumes_api.get_opt(name).await? {
            volumes.push(volume);
        }
    }
    let pods_api = Api::<Pod>::namespaced(client, DB_NAMESPACE);
    let pods = pods_api.list(&selector).await?.items;

    let lost = lost_database_instances(&claims, &volumes, &pods, nodes);
    for pod in &lost.pods {
        tracing::warn!(pod, "Deleting database instance Pod whose node was deleted");
        delete_ignoring_absent(pods_api.delete(pod, &DeleteParams::default()).await)?;
    }
    for claim in &lost.claims {
        tracing::warn!(
            claim,
            "Deleting database PVC pinned to a deleted node; CloudNativePG re-creates the instance"
        );
        delete_ignoring_absent(claims_api.delete(claim, &DeleteParams::default()).await)?;
    }
    Ok(())
}

fn delete_ignoring_absent<T>(result: kube::Result<T>) -> Result<()> {
    match result {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(status)) if status.code == 404 => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Values a release needs beyond `kubarr.ha.nodeCount` because its chart
/// cannot derive them itself. Only the upstream cert-manager subchart does.
pub fn extra_ha_values(release: &str, n: u32) -> Vec<String> {
    if release != "cert-manager" {
        return Vec::new();
    }
    let ha = n > 1;
    let replicas = if ha {
        n.min(CERT_MANAGER_MAX_REPLICAS)
    } else {
        1
    };
    let mut values = Vec::new();
    for (prefix, component) in [
        ("cert-manager.", "controller"),
        ("cert-manager.webhook.", "webhook"),
        ("cert-manager.cainjector.", "cainjector"),
    ] {
        values.push(format!("{prefix}replicaCount={replicas}"));
        // Explicit in both directions: --reuse-values would otherwise keep a
        // PDB that blocks every drain once the release is back to one replica.
        values.push(format!("{prefix}podDisruptionBudget.enabled={ha}"));
        if ha {
            let spread = format!("{prefix}topologySpreadConstraints[0]");
            values.extend([
                format!("{spread}.maxSkew=1"),
                format!("{spread}.topologyKey=kubernetes.io/hostname"),
                format!("{spread}.whenUnsatisfiable=DoNotSchedule"),
                // Tainted nodes (control planes, unreachable) are not spread
                // domains; as empty ones they would hold rollouts Pending.
                format!("{spread}.nodeTaintsPolicy=Honor"),
                format!(
                    "{spread}.labelSelector.matchLabels.app\\.kubernetes\\.io/instance={release}"
                ),
                format!(
                    "{spread}.labelSelector.matchLabels.app\\.kubernetes\\.io/component={component}"
                ),
            ]);
        } else {
            // null drops the reused list so the chart default ([]) applies.
            values.push(format!("{prefix}topologySpreadConstraints=null"));
        }
    }
    values
}

/// Helm arguments that change only the HA sizing of an installed release:
/// same chart revision, every previous value kept.
fn ha_upgrade_args(
    release: &str,
    release_namespace: &str,
    chart_ref: &str,
    version: &str,
    n: u32,
    plain_http: bool,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "upgrade",
        // Releases are installed with client-side apply; keep it under Helm 4.
        "--server-side=false",
        release,
        chart_ref,
        "-n",
        release_namespace,
        "--version",
        version,
        // Safe here, unlike on chart updates: the revision is unchanged, so
        // there are no new chart defaults that reuse could hide.
        "--reuse-values",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    if plain_http {
        args.push("--plain-http".to_string());
    }
    // No --wait: the worker itself is one of these releases, and a rescale
    // is complete once Kubernetes has the new spec (same rule as its
    // self-update).
    for value in std::iter::once(format!("{HA_VALUE}={n}")).chain(extra_ha_values(release, n)) {
        args.push("--set".to_string());
        args.push(value);
    }
    args
}

/// `kubarr.ha.nodeCount` from `helm get values -o json` output. A release
/// installed without it runs single-node sizing, so absence means 1. A value
/// that is present but unusable maps to 0 so it never matches a desired count
/// and the release is rewritten.
fn applied_node_count_from_values(json: &[u8]) -> Result<u32> {
    let values: serde_json::Value = serde_json::from_slice(json)
        .map_err(|error| AppError::Internal(format!("Invalid Helm values: {error}")))?;
    let Some(value) = values.pointer("/kubarr/ha/nodeCount") else {
        return Ok(1);
    };
    Ok(match value {
        serde_json::Value::Number(number) => number
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
        serde_json::Value::String(text) => text.trim().parse().unwrap_or(0),
        serde_json::Value::Null => 1,
        _ => 0,
    })
}

/// Recount nodes and bring every installed HA release to the desired size.
///
/// Each release is compared against its own applied value, so a release
/// installed after the last rescale (cert-manager from the catalog) still
/// converges. The setting is the memory of the last fully applied count and
/// is only advanced once every release matches. Returns the desired count
/// when anything was changed.
pub async fn reconcile_ha(
    manager: &AppManager,
    tracker: &mut WithheldTracker,
    cancellation: &CancellationToken,
) -> Result<Option<u32>> {
    let client = match manager.k8s_client.read().await.as_ref() {
        Some(client) => client.client().clone(),
        None => return Ok(None),
    };
    let nodes = Api::<Node>::all(client.clone())
        .list(&ListParams::default())
        .await?
        .items;
    if let Err(error) = release_lost_database_instances(client, &nodes).await {
        tracing::warn!(%error, "Failed to release database instances of deleted nodes");
    }

    let now = Utc::now();
    let removed = tracker.observe(&nodes, now);
    let applied = read_applied_count(&manager.db).await?;
    let desired = desired_node_count(
        applied,
        schedulable_node_count(&nodes, now),
        retained_node_count(&nodes, &removed),
    );

    // The worker upgrading its own release goes last so a restart cannot cut
    // the others short.
    let ordered = HA_RELEASES
        .iter()
        .filter(|release| **release != "kubarr-worker")
        .chain(std::iter::once(&"kubarr-worker"));

    let mut changed = false;
    let mut failures = Vec::new();
    for &release in ordered {
        if cancellation.is_cancelled() {
            return Ok(changed.then_some(desired));
        }
        match rescale_release(manager, release, desired).await {
            Ok(upgraded) => changed |= upgraded,
            Err(error) => {
                tracing::warn!(release, %error, "Failed to rescale HA release");
                failures.push(format!("{release}: {error}"));
            }
        }
    }

    if !failures.is_empty() {
        return Err(AppError::Internal(format!(
            "HA rescale to {desired} nodes incomplete: {}",
            failures.join("; ")
        )));
    }
    if desired != applied {
        write_applied_count(&manager.db, desired).await?;
        tracing::info!(from = applied, to = desired, "HA node count applied");
        changed = true;
    }
    Ok(changed.then_some(desired))
}

/// Bring one release to `desired`; true when it was upgraded.
///
/// The Helm lock covers reading the installed revision and values as well as
/// the upgrade, so a queued update cannot change the release in between (an
/// upgrade pinned to a stale revision would silently downgrade it). The read
/// guards are taken before the lock, matching `execute_operation`, and all of
/// it is released before the next release so queued operations interleave.
async fn rescale_release(manager: &AppManager, release: &str, desired: u32) -> Result<bool> {
    let k8s_guard = manager.k8s_client.read().await;
    let client = k8s_guard
        .as_ref()
        .ok_or_else(|| AppError::Internal("Kubernetes client not available".into()))?;
    let catalog = manager.catalog.read().await;
    let _helm = manager.helm_lock.lock().await;

    let deployment = DeploymentManager::new(client, &catalog);
    let Some(version) = deployment.installed_chart_version(release) else {
        return Ok(false);
    };
    let lifecycle = lifecycle_for_app_name(release);
    let current = release_node_count(&lifecycle.release_name, &lifecycle.release_namespace)?;
    if current == desired {
        return Ok(false);
    }
    let args = ha_upgrade_args(
        &lifecycle.release_name,
        &lifecycle.release_namespace,
        &deployment.chart_ref(release),
        &version,
        desired,
        CONFIG.charts.plain_http,
    );
    run_helm(&args, Duration::from_secs(5 * 60))?;
    tracing::info!(release, from = current, to = desired, "Rescaled HA release");
    Ok(true)
}

fn count(nodes: &[Node], keep: impl Fn(&Node) -> bool) -> u32 {
    u32::try_from(nodes.iter().filter(|node| keep(node)).count()).unwrap_or(u32::MAX)
}

fn deleting(node: &Node) -> bool {
    node.metadata.deletion_timestamp.is_some()
}

fn node_name(node: &Node) -> &str {
    node.metadata.name.as_deref().unwrap_or_default()
}

/// Cordoned, or carrying a NoSchedule/NoExecute taint (Kubarr pods tolerate
/// none). `ignore_condition_taints` skips the taints Kubernetes adds to an
/// unhealthy node, which would otherwise turn NotReady into removal.
fn withheld(node: &Node, ignore_condition_taints: bool) -> bool {
    let Some(spec) = node.spec.as_ref() else {
        return false;
    };
    spec.unschedulable == Some(true)
        || spec.taints.iter().flatten().any(|taint| {
            matches!(taint.effect.as_str(), "NoSchedule" | "NoExecute")
                && !(ignore_condition_taints && CONDITION_TAINTS.contains(&taint.key.as_str()))
        })
}

fn ready_condition(node: &Node) -> Option<&k8s_openapi::api::core::v1::NodeCondition> {
    node.status
        .as_ref()?
        .conditions
        .as_ref()?
        .iter()
        .find(|condition| condition.type_ == "Ready")
}

fn release_node_count(release: &str, namespace: &str) -> Result<u32> {
    let stdout = run_helm(
        &["get", "values", release, "-n", namespace, "-o", "json"].map(str::to_string),
        Duration::from_secs(30),
    )?;
    applied_node_count_from_values(&stdout)
}

fn run_helm(args: &[String], timeout: Duration) -> Result<Vec<u8>> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = tokio::task::block_in_place(|| helm_process::run(&args, timeout))?;
    if !output.status.success() {
        return Err(AppError::Internal(format!(
            "Helm command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output.stdout)
}

async fn read_applied_count(db: &DatabaseConnection) -> Result<u32> {
    let stored = system_setting::Entity::find_by_id(HA_NODE_COUNT_SETTING)
        .one(db)
        .await?;
    Ok(match stored {
        None => 1,
        Some(row) => row.value.trim().parse::<u32>().unwrap_or_else(|_| {
            tracing::warn!(value = %row.value, "Ignoring invalid {HA_NODE_COUNT_SETTING}");
            1
        }),
    }
    .max(1))
}

async fn write_applied_count(db: &DatabaseConnection, n: u32) -> Result<()> {
    let row = system_setting::ActiveModel {
        key: Set(HA_NODE_COUNT_SETTING.to_string()),
        value: Set(n.to_string()),
        description: Set(Some(
            "Node count the HA system releases are currently sized for".to_string(),
        )),
        updated_at: Set(Utc::now()),
    };
    system_setting::Entity::insert(row)
        .on_conflict(
            OnConflict::column(system_setting::Column::Key)
                .update_columns([
                    system_setting::Column::Value,
                    system_setting::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{NodeCondition, NodeSpec, NodeStatus, Taint};
    use k8s_openapi::api::core::v1::{PersistentVolume, PersistentVolumeClaim, Pod};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-03T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn node(ready: &str, ready_for_secs: i64) -> Node {
        let since = jiff::Timestamp::from_second((now().timestamp()) - ready_for_secs).unwrap();
        Node {
            spec: Some(NodeSpec::default()),
            status: Some(NodeStatus {
                conditions: Some(vec![NodeCondition {
                    type_: "Ready".into(),
                    status: ready.into(),
                    last_transition_time: Some(Time(since)),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn named(mut node: Node, name: &str) -> Node {
        node.metadata.name = Some(name.to_string());
        node
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    fn ready() -> Node {
        node("True", 600)
    }

    fn tainted(mut node: Node, key: &str, effect: &str) -> Node {
        node.spec.get_or_insert_with(Default::default).taints = Some(vec![Taint {
            key: key.into(),
            effect: effect.into(),
            ..Default::default()
        }]);
        node
    }

    #[test]
    fn counts_ready_untainted_nodes() {
        let nodes = [ready(), ready(), ready()];
        assert_eq!(schedulable_node_count(&nodes, now()), 3);
        assert_eq!(retained_node_count(&nodes, &none()), 3);
    }

    #[test]
    fn skips_cordoned_and_noschedule_tainted() {
        let mut cordoned = ready();
        cordoned.spec.as_mut().unwrap().unschedulable = Some(true);
        let nodes = [
            ready(),
            named(cordoned, "cordoned"),
            named(tainted(ready(), "dedicated", "NoSchedule"), "no-schedule"),
            named(tainted(ready(), "dedicated", "NoExecute"), "no-execute"),
            named(tainted(ready(), "soft", "PreferNoSchedule"), "soft"),
        ];
        assert_eq!(schedulable_node_count(&nodes, now()), 2);
        // Withheld nodes stay in the pool until the tracker reports them.
        assert_eq!(retained_node_count(&nodes, &none()), 5);
        let all: HashSet<String> = ["cordoned", "no-schedule", "no-execute", "soft"]
            .map(str::to_string)
            .into();
        assert_eq!(retained_node_count(&nodes, &all), 2);
    }

    #[test]
    fn debounces_recently_ready() {
        let nodes = [ready(), node("True", 60)];
        assert_eq!(schedulable_node_count(&nodes, now()), 1);
        assert_eq!(schedulable_node_count(&[node("True", 120)], now()), 1);
    }

    #[test]
    fn notready_node_is_retained() {
        let nodes = [ready(), node("False", 600), node("Unknown", 600)];
        assert_eq!(schedulable_node_count(&nodes, now()), 1);
        assert_eq!(retained_node_count(&nodes, &none()), 3);
    }

    #[test]
    fn notready_node_keeps_its_lifecycle_taints_but_is_retained() {
        let unreachable = tainted(
            node("Unknown", 600),
            "node.kubernetes.io/unreachable",
            "NoExecute",
        );
        let not_ready = tainted(
            node("False", 600),
            "node.kubernetes.io/not-ready",
            "NoSchedule",
        );
        let nodes = [
            ready(),
            named(unreachable, "unreachable"),
            named(not_ready, "not-ready"),
        ];
        assert_eq!(schedulable_node_count(&nodes, now()), 1);
        assert_eq!(retained_node_count(&nodes, &none()), 3);
        // Lifecycle taints are not tracked, however long they last.
        let mut tracker = WithheldTracker::default();
        tracker.observe(&nodes, now());
        let removed = tracker.observe(&nodes, now() + chrono::Duration::hours(5));
        assert!(removed.is_empty());
        assert_eq!(retained_node_count(&nodes, &removed), 3);
    }

    #[test]
    fn deleting_node_is_not_retained() {
        let mut leaving = ready();
        leaving.metadata.deletion_timestamp = Some(Time(jiff::Timestamp::now()));
        assert_eq!(retained_node_count(&[ready(), leaving], &none()), 1);
    }

    fn cordoned(name: &str) -> Node {
        let mut node = named(ready(), name);
        node.spec.as_mut().unwrap().unschedulable = Some(true);
        node
    }

    fn minutes(n: i64) -> DateTime<Utc> {
        now() + chrono::Duration::minutes(n)
    }

    #[test]
    fn freshly_cordoned_node_is_retained() {
        let nodes = [named(ready(), "a"), cordoned("b")];
        let mut tracker = WithheldTracker::default();
        let removed = tracker.observe(&nodes, now());
        assert_eq!(retained_node_count(&nodes, &removed), 2);
        let removed = tracker.observe(&nodes, minutes(29));
        assert_eq!(retained_node_count(&nodes, &removed), 2);
    }

    #[test]
    fn node_cordoned_past_grace_is_not_retained() {
        let nodes = [named(ready(), "a"), cordoned("b")];
        let mut tracker = WithheldTracker::default();
        tracker.observe(&nodes, now());
        let removed = tracker.observe(&nodes, minutes(31));
        assert_eq!(removed, HashSet::from(["b".to_string()]));
        assert_eq!(retained_node_count(&nodes, &removed), 1);
    }

    #[test]
    fn uncordon_resets_the_grace_period() {
        let mut tracker = WithheldTracker::default();
        tracker.observe(&[cordoned("b")], now());
        tracker.observe(&[named(ready(), "b")], minutes(20));
        let removed = tracker.observe(&[cordoned("b")], minutes(31));
        assert!(removed.is_empty());
        let removed = tracker.observe(&[cordoned("b")], minutes(61));
        assert!(removed.contains("b"));
    }

    #[test]
    fn deleted_node_is_forgotten_by_the_tracker() {
        let mut tracker = WithheldTracker::default();
        tracker.observe(&[cordoned("b")], now());
        tracker.observe(&[], minutes(10));
        assert!(tracker.observe(&[cordoned("b")], minutes(31)).is_empty());
    }

    #[test]
    fn draining_one_of_three_nodes_keeps_desired() {
        let mut tracker = WithheldTracker::default();
        let mut desired = 3;
        for minute in 0..=10 {
            let nodes = [named(ready(), "a"), named(ready(), "b"), cordoned("c")];
            let removed = tracker.observe(&nodes, minutes(minute));
            desired = desired_node_count(
                desired,
                schedulable_node_count(&nodes, minutes(minute)),
                retained_node_count(&nodes, &removed),
            );
            assert_eq!(desired, 3, "minute {minute}");
        }
    }

    #[test]
    fn desired_scales_up() {
        assert_eq!(desired_node_count(1, 3, 3), 3);
    }

    #[test]
    fn desired_holds_on_notready() {
        assert_eq!(desired_node_count(3, 2, 3), 3);
    }

    #[test]
    fn desired_scales_down_on_deletion() {
        assert_eq!(desired_node_count(3, 2, 2), 2);
    }

    #[test]
    fn desired_never_below_one() {
        assert_eq!(desired_node_count(1, 0, 0), 1);
        assert_eq!(desired_node_count(3, 0, 0), 1);
        assert_eq!(desired_node_count(0, 0, 0), 1);
    }

    #[test]
    fn ha_upgrade_args_pin_version_and_reuse_values() {
        let args = ha_upgrade_args(
            "kubarr-backend",
            "default",
            "oci://registry/kubarr-backend",
            "0.4.2+1",
            3,
            false,
        );
        assert_eq!(
            args,
            [
                "upgrade",
                "--server-side=false",
                "kubarr-backend",
                "oci://registry/kubarr-backend",
                "-n",
                "default",
                "--version",
                "0.4.2+1",
                "--reuse-values",
                "--set",
                "kubarr.ha.nodeCount=3",
            ]
        );
        assert!(!args.iter().any(|arg| arg.starts_with("--wait")));
        assert!(!args.iter().any(|arg| arg.contains("reset")));
        let plain = ha_upgrade_args("openresty", "default", "r/openresty", "1.0.0", 1, true);
        assert!(plain.contains(&"--plain-http".to_string()));
    }

    #[test]
    fn ha_upgrade_args_carry_cert_manager_extras() {
        let args = ha_upgrade_args(
            "cert-manager",
            "default",
            "r/cert-manager",
            "1.21.3+1",
            2,
            false,
        );
        assert!(args.contains(&"kubarr.ha.nodeCount=2".to_string()));
        assert!(args.contains(&"cert-manager.replicaCount=2".to_string()));
    }

    #[test]
    fn cert_manager_gets_subchart_replica_values() {
        let three = extra_ha_values("cert-manager", 3);
        for prefix in [
            "cert-manager.",
            "cert-manager.webhook.",
            "cert-manager.cainjector.",
        ] {
            assert!(three.contains(&format!("{prefix}replicaCount=2")));
            assert!(three.contains(&format!("{prefix}podDisruptionBudget.enabled=true")));
            assert!(three.contains(&format!(
                "{prefix}topologySpreadConstraints[0].topologyKey=kubernetes.io/hostname"
            )));
            assert!(three.contains(&format!(
                "{prefix}topologySpreadConstraints[0].whenUnsatisfiable=DoNotSchedule"
            )));
            assert!(three.contains(&format!(
                "{prefix}topologySpreadConstraints[0].nodeTaintsPolicy=Honor"
            )));
        }
        assert!(three.contains(
            &"cert-manager.webhook.topologySpreadConstraints[0].labelSelector.matchLabels.app\\.kubernetes\\.io/component=webhook"
                .to_string()
        ));
        assert!(
            extra_ha_values("cert-manager", 2).contains(&"cert-manager.replicaCount=2".to_string())
        );

        let one = extra_ha_values("cert-manager", 1);
        for prefix in [
            "cert-manager.",
            "cert-manager.webhook.",
            "cert-manager.cainjector.",
        ] {
            assert!(one.contains(&format!("{prefix}replicaCount=1")));
            assert!(one.contains(&format!("{prefix}podDisruptionBudget.enabled=false")));
            assert!(one.contains(&format!("{prefix}topologySpreadConstraints=null")));
        }

        for release in HA_RELEASES.iter().filter(|r| **r != "cert-manager") {
            assert!(extra_ha_values(release, 3).is_empty(), "{release}");
        }
    }

    fn live(name: &str) -> Node {
        let mut node = named(ready(), name);
        node.metadata.labels =
            Some([("kubernetes.io/hostname".to_string(), name.to_string())].into());
        node
    }

    fn claim(name: &str, volume: Option<&str>) -> PersistentVolumeClaim {
        serde_json::from_value(serde_json::json!({
            "metadata": {"name": name, "labels": {"cnpg.io/cluster": DB_CLUSTER}},
            "spec": {"volumeName": volume}
        }))
        .unwrap()
    }

    fn volume_on(name: &str, host: &str) -> PersistentVolume {
        serde_json::from_value(serde_json::json!({
            "metadata": {"name": name},
            "spec": {"nodeAffinity": {"required": {"nodeSelectorTerms": [{
                "matchExpressions": [{
                    "key": "kubernetes.io/hostname", "operator": "In", "values": [host]
                }]
            }]}}}
        }))
        .unwrap()
    }

    fn instance_pod(name: &str, claim: &str) -> Pod {
        serde_json::from_value(serde_json::json!({
            "metadata": {"name": name},
            "spec": {"containers": [], "volumes": [
                {"name": "pgdata", "persistentVolumeClaim": {"claimName": claim}},
                {"name": "scratch", "emptyDir": {}}
            ]}
        }))
        .unwrap()
    }

    fn three_instances() -> (Vec<PersistentVolumeClaim>, Vec<PersistentVolume>, Vec<Pod>) {
        (
            vec![
                claim("kubarr-db-1", Some("pv-a")),
                claim("kubarr-db-2", Some("pv-b")),
                claim("kubarr-db-3", Some("pv-c")),
            ],
            vec![
                volume_on("pv-a", "a"),
                volume_on("pv-b", "b"),
                volume_on("pv-c", "c"),
            ],
            vec![
                instance_pod("kubarr-db-1", "kubarr-db-1"),
                instance_pod("kubarr-db-2", "kubarr-db-2"),
                instance_pod("kubarr-db-3", "kubarr-db-3"),
            ],
        )
    }

    #[test]
    fn instance_on_a_deleted_node_is_released() {
        let (claims, volumes, pods) = three_instances();
        let lost = lost_database_instances(&claims, &volumes, &pods, &[live("a"), live("b")]);
        assert_eq!(lost.claims, ["kubarr-db-3"]);
        assert_eq!(lost.pods, ["kubarr-db-3"]);
    }

    #[test]
    fn instance_on_an_existing_node_is_kept_whatever_its_health() {
        let (claims, volumes, pods) = three_instances();
        let nodes = [
            live("a"),
            live("b"),
            tainted(
                named(node("Unknown", 600), "c"),
                "node.kubernetes.io/unreachable",
                "NoExecute",
            ),
        ];
        // The tainted node keeps its hostname label in real clusters; drop it
        // here to prove the node name alone keeps the volume.
        let lost = lost_database_instances(&claims, &volumes, &pods, &nodes);
        assert!(lost.is_empty(), "{lost:?}");
        let mut leaving = live("c");
        leaving.metadata.deletion_timestamp = Some(Time(jiff::Timestamp::now()));
        let lost =
            lost_database_instances(&claims, &volumes, &pods, &[live("a"), live("b"), leaving]);
        assert!(
            lost.is_empty(),
            "a node still being deleted exists: {lost:?}"
        );
    }

    #[test]
    fn hostname_label_also_identifies_a_node() {
        let (claims, volumes, pods) = three_instances();
        let mut renamed = named(ready(), "node-c.example");
        renamed.metadata.labels =
            Some([("kubernetes.io/hostname".to_string(), "c".to_string())].into());
        let lost =
            lost_database_instances(&claims, &volumes, &pods, &[live("a"), live("b"), renamed]);
        assert!(lost.is_empty(), "{lost:?}");
    }

    #[test]
    fn nothing_is_released_without_a_surviving_instance() {
        let (claims, volumes, pods) = three_instances();
        // Every instance's node is gone: deleting all PVCs would destroy the
        // only copies of the data, so wait for a human instead.
        assert!(lost_database_instances(&claims, &volumes, &pods, &[live("z")]).is_empty());
        // A fresh, still unbound claim holds no data and is no survivor.
        let mut with_new = claims.clone();
        with_new.push(claim("kubarr-db-4", None));
        assert!(lost_database_instances(&with_new, &volumes, &pods, &[live("z")]).is_empty());
        // An empty node list is never evidence of deletion.
        assert!(lost_database_instances(&claims, &volumes, &pods, &[]).is_empty());
    }

    #[test]
    fn claims_without_a_node_pinned_volume_are_left_alone() {
        let claims = vec![
            claim("kubarr-db-1", Some("pv-a")),
            claim("kubarr-db-2", None),
            claim("kubarr-db-3", Some("pv-missing")),
            claim("kubarr-db-4", Some("pv-nfs")),
        ];
        let nfs: PersistentVolume =
            serde_json::from_value(serde_json::json!({"metadata": {"name": "pv-nfs"}})).unwrap();
        let volumes = vec![volume_on("pv-a", "a"), nfs];
        let lost = lost_database_instances(&claims, &volumes, &[], &[live("a")]);
        assert!(lost.is_empty(), "{lost:?}");
    }

    #[test]
    fn wal_claim_and_its_pod_go_together() {
        let (mut claims, mut volumes, mut pods) = three_instances();
        claims.push(claim("kubarr-db-3-wal", Some("pv-c-wal")));
        volumes.push(volume_on("pv-c-wal", "c"));
        pods[2] = serde_json::from_value(serde_json::json!({
            "metadata": {"name": "kubarr-db-3"},
            "spec": {"containers": [], "volumes": [
                {"name": "pgdata", "persistentVolumeClaim": {"claimName": "kubarr-db-3"}},
                {"name": "pg-wal", "persistentVolumeClaim": {"claimName": "kubarr-db-3-wal"}}
            ]}
        }))
        .unwrap();
        let lost = lost_database_instances(&claims, &volumes, &pods, &[live("a"), live("b")]);
        assert_eq!(lost.claims, ["kubarr-db-3", "kubarr-db-3-wal"]);
        assert_eq!(lost.pods, ["kubarr-db-3"]);
    }

    #[test]
    fn applied_count_reads_helm_values() {
        let parse = |json: &str| applied_node_count_from_values(json.as_bytes()).unwrap();
        assert_eq!(parse("null"), 1);
        assert_eq!(parse("{}"), 1);
        assert_eq!(parse(r#"{"kubarr":{"storage":{}}}"#), 1);
        assert_eq!(parse(r#"{"kubarr":{"ha":{"nodeCount":3}}}"#), 3);
        assert_eq!(parse(r#"{"kubarr":{"ha":{"nodeCount":"2"}}}"#), 2);
        assert_eq!(parse(r#"{"kubarr":{"ha":{"nodeCount":"many"}}}"#), 0);
        assert_eq!(parse(r#"{"kubarr":{"ha":{"nodeCount":-1}}}"#), 0);
        assert!(applied_node_count_from_values(b"not json").is_err());
    }
}
