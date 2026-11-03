//! Scales Kubarr's HA system releases with the number of usable cluster nodes.
//!
//! Every HA chart reads `kubarr.ha.nodeCount` and derives its own replica
//! count from it. The worker leader recounts nodes periodically and upgrades
//! any installed HA release whose applied value differs from the desired one.
//!
//! Scale-up waits until a node has been Ready for [`READY_DEBOUNCE`]; scale-down
//! only happens once a node is deleted, cordoned or tainted away — never
//! merely because it went NotReady.

use std::time::Duration;

use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::Node;
use kube::api::{Api, ListParams};
use sea_orm::sea_query::OnConflict;
use sea_orm::{DatabaseConnection, EntityTrait, Set};

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
        accepts_kubarr_pods(node, false)
            && ready_condition(node).is_some_and(|condition| {
                condition.status == "True"
                    && condition
                        .last_transition_time
                        .as_ref()
                        .is_some_and(|since| since.0.as_second() <= ready_before)
            })
    })
}

/// Nodes that still belong to the pool whatever their health: a NotReady node
/// is retained, a deleted, cordoned or permanently tainted one is not.
pub fn retained_node_count(nodes: &[Node]) -> u32 {
    count(nodes, |node| accepts_kubarr_pods(node, true))
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
pub async fn reconcile_ha(manager: &AppManager) -> Result<Option<u32>> {
    let k8s_guard = manager.k8s_client.read().await;
    let Some(client) = k8s_guard.as_ref() else {
        return Ok(None);
    };
    let nodes: Api<Node> = Api::all(client.client().clone());
    let nodes = nodes.list(&ListParams::default()).await?.items;

    let applied = read_applied_count(&manager.db).await?;
    let desired = desired_node_count(
        applied,
        schedulable_node_count(&nodes, Utc::now()),
        retained_node_count(&nodes),
    );

    let catalog = manager.catalog.read().await;
    let deployment = DeploymentManager::new(client, &catalog);
    // The worker upgrading its own release goes last so a restart cannot cut
    // the others short.
    let ordered = HA_RELEASES
        .iter()
        .filter(|release| **release != "kubarr-worker")
        .chain(std::iter::once(&"kubarr-worker"));

    let mut changed = false;
    let mut failures = Vec::new();
    for &release in ordered {
        let Some(version) = deployment.installed_chart_version(release) else {
            continue;
        };
        let lifecycle = lifecycle_for_app_name(release);
        let outcome = async {
            let current =
                release_node_count(&lifecycle.release_name, &lifecycle.release_namespace)?;
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
            let _helm = manager.helm_lock.lock().await;
            run_helm(&args, Duration::from_secs(5 * 60))?;
            tracing::info!(release, from = current, to = desired, "Rescaled HA release");
            Ok::<_, AppError>(true)
        }
        .await;
        match outcome {
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

fn count(nodes: &[Node], keep: impl Fn(&Node) -> bool) -> u32 {
    u32::try_from(nodes.iter().filter(|node| keep(node)).count()).unwrap_or(u32::MAX)
}

/// Not being deleted, not cordoned, and no NoSchedule/NoExecute taint (Kubarr
/// pods tolerate none). `allow_condition_taints` ignores the taints Kubernetes
/// adds to an unhealthy node, which would otherwise turn NotReady into removal.
fn accepts_kubarr_pods(node: &Node, allow_condition_taints: bool) -> bool {
    if node.metadata.deletion_timestamp.is_some() {
        return false;
    }
    let Some(spec) = node.spec.as_ref() else {
        return true;
    };
    if spec.unschedulable == Some(true) {
        return false;
    }
    !spec.taints.iter().flatten().any(|taint| {
        matches!(taint.effect.as_str(), "NoSchedule" | "NoExecute")
            && !(allow_condition_taints && CONDITION_TAINTS.contains(&taint.key.as_str()))
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
        assert_eq!(retained_node_count(&nodes), 3);
    }

    #[test]
    fn skips_cordoned_and_noschedule_tainted() {
        let mut cordoned = ready();
        cordoned.spec.as_mut().unwrap().unschedulable = Some(true);
        let nodes = [
            ready(),
            cordoned,
            tainted(ready(), "dedicated", "NoSchedule"),
            tainted(ready(), "dedicated", "NoExecute"),
            tainted(ready(), "soft", "PreferNoSchedule"),
        ];
        assert_eq!(schedulable_node_count(&nodes, now()), 2);
        assert_eq!(retained_node_count(&nodes), 2);
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
        assert_eq!(retained_node_count(&nodes), 3);
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
        let nodes = [ready(), unreachable, not_ready];
        assert_eq!(schedulable_node_count(&nodes, now()), 1);
        assert_eq!(retained_node_count(&nodes), 3);
    }

    #[test]
    fn deleting_node_is_not_retained() {
        let mut leaving = ready();
        leaving.metadata.deletion_timestamp = Some(Time(jiff::Timestamp::now()));
        assert_eq!(retained_node_count(&[ready(), leaving]), 1);
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
