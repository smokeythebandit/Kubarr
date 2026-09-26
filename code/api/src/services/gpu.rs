use std::collections::{BTreeMap, HashMap};

use k8s_openapi::api::core::v1::Node;
use kube::api::{Api, ListParams};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::services::k8s::K8sClient;

pub const QUEUED_GPU_KEY: &str = "__kubarr_managed_gpu";

pub fn gpu_field<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Option<GpuSelection>>, D::Error> {
    Option::<GpuSelection>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GpuSelection {
    pub vendor: GpuVendor,
    pub node_name: String,
    /// Omission preserves the legacy Intel i915 / NVIDIA gpu selection.
    #[serde(default)]
    pub resource_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum GpuVendor {
    Intel,
    Nvidia,
    Amd,
}

impl GpuVendor {
    pub fn default_resource(self) -> Option<&'static str> {
        match self {
            Self::Intel => Some("gpu.intel.com/i915"),
            Self::Nvidia => Some("nvidia.com/gpu"),
            Self::Amd => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intel => "intel",
            Self::Nvidia => "nvidia",
            Self::Amd => "amd",
        }
    }
}

const GPU_RESOURCES: &[(&str, GpuVendor)] = &[
    ("gpu.intel.com/i915", GpuVendor::Intel),
    ("gpu.intel.com/xe", GpuVendor::Intel),
    ("nvidia.com/gpu", GpuVendor::Nvidia),
    ("nvidia.com/gpu.shared", GpuVendor::Nvidia),
    // Only a prepared shared DRM device plugin exposing this resource is suitable
    // for media containers. amd.com/gpu (ROCm compute) does not imply /dev/dri.
    ("amd.com/dri", GpuVendor::Amd),
];

fn selected_resource(selection: &GpuSelection) -> Result<&str> {
    let name = selection
        .resource_name
        .as_deref()
        .or_else(|| selection.vendor.default_resource())
        .ok_or_else(|| AppError::BadRequest("AMD requires an explicit advertised amd.com/dri resource from a prepared shared DRM device plugin".into()))?;
    if !GPU_RESOURCES.contains(&(name, selection.vendor)) {
        return Err(AppError::BadRequest(format!(
            "Unsupported GPU resource '{name}' for {} (AMD requires a prepared shared DRM device plugin, not a ROCm compute resource)",
            selection.vendor.as_str()
        )));
    }
    Ok(name)
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct GpuNode {
    pub name: String,
    pub ready: bool,
    pub schedulable: bool,
    /// Kubernetes advertised resource counts, not physical devices or transcoding capability.
    pub capacity: BTreeMap<String, i64>,
    pub allocatable: BTreeMap<String, i64>,
}

fn resource_counts(
    map: Option<&BTreeMap<String, k8s_openapi::apimachinery::pkg::api::resource::Quantity>>,
) -> BTreeMap<String, i64> {
    GPU_RESOURCES
        .into_iter()
        .filter_map(|(name, _)| {
            map.and_then(|map| map.get(*name))
                .and_then(|quantity| quantity.0.parse::<i64>().ok())
                .map(|count| ((*name).to_owned(), count))
        })
        .collect()
}

fn summarize(node: &Node) -> GpuNode {
    let status = node.status.as_ref();
    GpuNode {
        name: node.metadata.name.clone().unwrap_or_default(),
        ready: status
            .and_then(|s| s.conditions.as_ref())
            .is_some_and(|conditions| {
                conditions
                    .iter()
                    .any(|condition| condition.type_ == "Ready" && condition.status == "True")
            }),
        schedulable: !node
            .spec
            .as_ref()
            .and_then(|s| s.unschedulable)
            .unwrap_or(false),
        capacity: resource_counts(status.and_then(|s| s.capacity.as_ref())),
        allocatable: resource_counts(status.and_then(|s| s.allocatable.as_ref())),
    }
}

pub async fn list_gpu_nodes(client: &K8sClient) -> Result<Vec<GpuNode>> {
    let nodes: Api<Node> = Api::all(client.client().clone());
    let mut result: Vec<_> = nodes
        .list(&ListParams::default())
        .await?
        .items
        .iter()
        .map(summarize)
        .collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

pub async fn validate_selection(client: &K8sClient, selection: &GpuSelection) -> Result<String> {
    let resource = selected_resource(selection)?;
    // Node names are DNS subdomains. Reject Helm --set metacharacters even when the
    // Kubernetes API is unavailable or returns a malformed object.
    let name = &selection.node_name;
    if name.is_empty()
        || name.len() > 253
        || !name.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && part
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && part
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        })
    {
        return Err(AppError::BadRequest("Invalid GPU node name".into()));
    }
    let nodes: Api<Node> = Api::all(client.client().clone());
    let node = nodes.get(name).await.map_err(|error| match error {
        kube::Error::Api(ref response) if response.code == 404 => {
            AppError::BadRequest("GPU node not found".into())
        }
        other => AppError::Internal(format!("Failed to inspect GPU node: {other}")),
    })?;
    let status = summarize(&node);
    if !status.ready
        || !status.schedulable
        || status.allocatable.get(resource).copied().unwrap_or(0) <= 0
    {
        return Err(AppError::BadRequest(format!(
            "GPU node is not ready/schedulable with advertised allocatable {}",
            resource
        )));
    }
    node.metadata
        .labels
        .as_ref()
        .and_then(|labels| labels.get("kubernetes.io/hostname"))
        .filter(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        })
        .cloned()
        .ok_or_else(|| {
            AppError::BadRequest("GPU node has no usable kubernetes.io/hostname label".into())
        })
}

pub fn validate_custom_gpu_keys(custom_config: &HashMap<String, String>) -> Result<()> {
    let is_managed = |part: &str| {
        let part = part.trim_start().trim_start_matches('{').trim_start();
        // Helm --set permits backslash-escaped dots in extended resource keys.
        let normalized = part.replace("\\.", ".");
        part == "gpu"
            || part.starts_with("gpu.")
            || part.starts_with("gpu[")
            || GPU_RESOURCES
                .iter()
                .map(|(name, _)| name)
                .any(|resource| normalized.contains("resources.") && normalized.contains(resource))
    };
    if let Some(key) = custom_config
        .keys()
        .find(|key| *key == QUEUED_GPU_KEY || key.split(',').any(is_managed))
    {
        return Err(AppError::BadRequest(format!(
            "custom_config key '{key}' is managed by Kubarr; use gpu instead"
        )));
    }
    if let Some((key, _)) = custom_config.iter().find(|(_, value)| {
        value.split(',').skip(1).any(|segment| {
            let segment = segment.trim_start().trim_start_matches('{').trim_start();
            segment
                .split_once('=')
                .is_some_and(|(path, _)| is_managed(path.trim()))
        })
    }) {
        return Err(AppError::BadRequest(format!(
            "custom_config key '{key}' contains a managed GPU assignment; use gpu instead"
        )));
    }
    Ok(())
}

pub fn validate_gpu_scheduling_config(custom_config: &HashMap<String, String>) -> Result<()> {
    if custom_config.iter().any(|(key, value)| {
        let scheduling = |part: &str| {
            let part = part.trim_start().trim_start_matches('{').trim_start();
            part == "nodeSelector"
                || part.starts_with("nodeSelector.")
                || part.starts_with("nodeSelector[")
                || part == "affinity"
                || part.starts_with("affinity.")
                || part.starts_with("affinity[")
        };
        key.split(',').any(scheduling)
            || value.split(',').skip(1).any(|segment| {
                segment
                    .split_once('=')
                    .is_some_and(|(path, _)| scheduling(path))
            })
    }) {
        return Err(AppError::BadRequest("GPU selection manages node scheduling; remove nodeSelector/affinity from custom_config".into()));
    }
    Ok(())
}

/// None means preserve on upgrade; Some(None) explicitly disables and clears reused values.
pub fn helm_values(selection: Option<(&GpuSelection, &str)>) -> Vec<String> {
    let mut values = vec![
        "gpu.enabled=false".into(),
        "gpu.provider=intel".into(),
        "gpu.resourceName=".into(),
        "gpu.runtimeClassName=".into(),
        "nodeSelector.kubernetes\\.io/hostname=null".into(),
    ];
    // Helm --reuse-values retains old app resources. The chart helper deep-copies
    // these and merges GPU into the chart's default CPU/memory resource map.
    // Null removes only managed GPU keys, for both disabling and switching GPUs.
    for (resource, _) in GPU_RESOURCES {
        let escaped = resource.replace('.', "\\.");
        for app in ["plex", "jellyfin"] {
            for kind in ["requests", "limits"] {
                values.push(format!("{app}.resources.{kind}.{escaped}=null"));
            }
        }
    }
    if let Some((selection, hostname)) = selection {
        // Validation occurs at enqueue and again immediately before deployment.
        let resource = selected_resource(selection).expect("validated GPU selection");
        values[0] = "gpu.enabled=true".into();
        values[1] = format!("gpu.provider={}", selection.vendor.as_str());
        values[2] = format!("gpu.resourceName={resource}");
        values[4] = format!("nodeSelector.kubernetes\\.io/hostname={hostname}");
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{Request, Response};
    use k8s_openapi::api::core::v1::{NodeCondition, NodeSpec, NodeStatus};
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use kube::{client::Body, Client};
    use tower::service_fn;

    #[tokio::test]
    async fn discovers_advertised_resources_and_validates_readiness() {
        let node = Node {
            metadata: ObjectMeta {
                name: Some("gpu-node".into()),
                labels: Some(BTreeMap::from([(
                    "kubernetes.io/hostname".into(),
                    "host-a".into(),
                )])),
                ..Default::default()
            },
            spec: Some(NodeSpec::default()),
            status: Some(NodeStatus {
                conditions: Some(vec![NodeCondition {
                    type_: "Ready".into(),
                    status: "True".into(),
                    ..Default::default()
                }]),
                capacity: Some(BTreeMap::from([(
                    "gpu.intel.com/i915".into(),
                    Quantity("4".into()),
                )])),
                allocatable: Some(BTreeMap::from([(
                    "gpu.intel.com/i915".into(),
                    Quantity("2".into()),
                )])),
                ..Default::default()
            }),
        };
        let service = service_fn(move |request: Request<Body>| {
            let node = node.clone();
            async move {
                let payload = if request.uri().path() == "/api/v1/nodes" {
                    serde_json::json!({"apiVersion":"v1", "kind":"NodeList", "items":[node]})
                } else {
                    assert_eq!(request.uri().path(), "/api/v1/nodes/gpu-node");
                    serde_json::to_value(node).unwrap()
                };
                Ok::<_, std::convert::Infallible>(Response::new(Body::from(
                    serde_json::to_vec(&payload).unwrap(),
                )))
            }
        });
        let client = K8sClient::from_client(Client::new(service, "default"));
        let discovered = list_gpu_nodes(&client).await.unwrap();
        assert_eq!(discovered[0].capacity["gpu.intel.com/i915"], 4);
        assert_eq!(discovered[0].allocatable["gpu.intel.com/i915"], 2);
        assert!(discovered[0].ready);
        assert_eq!(
            validate_selection(
                &client,
                &GpuSelection {
                    vendor: GpuVendor::Intel,
                    node_name: "gpu-node".into(),
                    resource_name: None,
                },
            )
            .await
            .unwrap(),
            "host-a"
        );
        assert!(validate_selection(
            &client,
            &GpuSelection {
                vendor: GpuVendor::Nvidia,
                node_name: "gpu-node".into(),
                resource_name: None,
            }
        )
        .await
        .is_err());
        assert!(validate_selection(
            &client,
            &GpuSelection {
                vendor: GpuVendor::Intel,
                node_name: "gpu-node,gpu.enabled=true".into(),
                resource_name: None,
            }
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn aliases_are_discovered_and_selection_requires_matching_advertisement() {
        let resources = BTreeMap::from([
            ("gpu.intel.com/xe".into(), Quantity("2".into())),
            ("nvidia.com/gpu.shared".into(), Quantity("3".into())),
            ("amd.com/dri".into(), Quantity("1".into())),
            ("amd.com/gpu".into(), Quantity("8".into())),
        ]);
        let node = Node {
            metadata: ObjectMeta {
                name: Some("gpu-node".into()),
                labels: Some(BTreeMap::from([(
                    "kubernetes.io/hostname".into(),
                    "host-a".into(),
                )])),
                ..Default::default()
            },
            status: Some(NodeStatus {
                conditions: Some(vec![NodeCondition {
                    type_: "Ready".into(),
                    status: "True".into(),
                    ..Default::default()
                }]),
                capacity: Some(resources.clone()),
                allocatable: Some(resources),
                ..Default::default()
            }),
            ..Default::default()
        };
        let service = service_fn(move |request: Request<Body>| {
            let node = node.clone();
            async move {
                let payload = if request.uri().path() == "/api/v1/nodes" {
                    serde_json::json!({"apiVersion":"v1", "kind":"NodeList", "items":[node]})
                } else {
                    assert_eq!(request.uri().path(), "/api/v1/nodes/gpu-node");
                    serde_json::to_value(node).unwrap()
                };
                Ok::<_, std::convert::Infallible>(Response::new(Body::from(
                    serde_json::to_vec(&payload).unwrap(),
                )))
            }
        });
        let client = K8sClient::from_client(Client::new(service, "default"));
        let discovered = list_gpu_nodes(&client).await.unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].allocatable.len(), 3);
        assert!(!discovered[0].allocatable.contains_key("amd.com/gpu"));
        for (vendor, resource) in [
            (GpuVendor::Intel, "gpu.intel.com/xe"),
            (GpuVendor::Nvidia, "nvidia.com/gpu.shared"),
            (GpuVendor::Amd, "amd.com/dri"),
        ] {
            let selection = GpuSelection {
                vendor,
                node_name: "gpu-node".into(),
                resource_name: Some(resource.into()),
            };
            assert_eq!(
                validate_selection(&client, &selection).await.unwrap(),
                "host-a"
            );
            let values = helm_values(Some((&selection, "host-a")));
            assert!(values.contains(&format!("gpu.resourceName={resource}")));
        }
        for (vendor, resource) in [
            (GpuVendor::Intel, None),
            (GpuVendor::Nvidia, None),
            (GpuVendor::Amd, None),
            (GpuVendor::Amd, Some("amd.com/gpu")),
            (GpuVendor::Intel, Some("nvidia.com/gpu.shared")),
            (GpuVendor::Nvidia, Some("nvidia.com/gpu,other=true")),
        ] {
            let selection = GpuSelection {
                vendor,
                node_name: "gpu-node".into(),
                resource_name: resource.map(str::to_string),
            };
            assert!(
                validate_selection(&client, &selection).await.is_err(),
                "{selection:?}"
            );
        }
    }

    #[test]
    fn rejects_managed_overrides_and_cleans_reused_values() {
        for key in [
            "gpu",
            "gpu.enabled",
            "gpu[0].enabled",
            "image.tag,gpu.enabled",
            "__kubarr_managed_gpu",
            "plex.resources.limits.nvidia.com/gpu",
            "plex.resources.requests.nvidia\\.com/gpu\\.shared",
            "jellyfin.resources.limits.amd\\.com/dri",
        ] {
            assert!(
                validate_custom_gpu_keys(&HashMap::from([(key.into(), "1".into())])).is_err(),
                "{key}"
            );
        }
        assert!(validate_custom_gpu_keys(&HashMap::from([(
            "image.tag".into(),
            "stable,gpu.enabled=true".into()
        )]))
        .is_err());
        let cleared = helm_values(None);
        assert_eq!(
            &cleared[..5],
            [
                "gpu.enabled=false",
                "gpu.provider=intel",
                "gpu.resourceName=",
                "gpu.runtimeClassName=",
                "nodeSelector.kubernetes\\.io/hostname=null"
            ]
        );
        for resource in [
            "gpu\\.intel\\.com/i915",
            "gpu\\.intel\\.com/xe",
            "nvidia\\.com/gpu",
            "nvidia\\.com/gpu\\.shared",
            "amd\\.com/dri",
        ] {
            assert!(cleared.contains(&format!("plex.resources.requests.{resource}=null")));
            assert!(cleared.contains(&format!("jellyfin.resources.limits.{resource}=null")));
        }
        assert!(!cleared
            .iter()
            .any(|value| value.contains("cpu") || value.contains("memory")));
        assert!(validate_gpu_scheduling_config(&HashMap::from([(
            "nodeSelector.kubernetes.io/hostname".into(),
            "other-node".into()
        )]))
        .is_err());
        assert!(validate_gpu_scheduling_config(&HashMap::from([(
            "image.tag".into(),
            "latest,affinity.nodeAffinity=other".into()
        )]))
        .is_err());
    }
}
