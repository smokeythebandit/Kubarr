use std::collections::BTreeMap;
use std::fmt;

use chrono::Utc;
use k8s_openapi::api::core::v1::{
    Container, NFSVolumeSource, Namespace, PersistentVolume, PersistentVolumeClaim,
    PersistentVolumeClaimSpec, PersistentVolumeClaimVolumeSource, PersistentVolumeSpec, Pod,
    PodSecurityContext, PodSpec, Secret, Volume, VolumeMount, VolumeResourceRequirements,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::ByteString;
use kube::api::{Api, DeleteParams, ListParams, LogParams, PostParams};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;
use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::models::prelude::*;
use crate::models::storage_config;
use crate::services::k8s::K8sClient;

pub const STORAGE_SECRET_NAME: &str = "kubarr-storage-config";
pub const MEDIA_PVC_NAME: &str = "media-data";
pub const STORAGE_MOUNT_PATH: &str = "/data";
pub const DEFAULT_STORAGE_UID: i32 = 1000;
pub const DEFAULT_STORAGE_GID: i32 = 1000;
pub const DEFAULT_STORAGE_FS_GROUP: i32 = 1000;

const SECRET_CONFIG_KEY: &str = "config.json";
const MANAGED_NFS_NAMESPACE: &str = "kubarr-storage";
const MANAGED_NFS_NAME: &str = "kubarr-managed-nfs";
const MANAGED_NFS_EXPORT_PATH: &str = "/";
const MANAGED_NFS_RELEASE: &str = "managed-nfs";
const BOOTSTRAP_RELEASE_NAMESPACE: &str = "default";
const MANAGED_NFS_CHART_REF: &str = "oci://ghcr.io/smokeythebandit/kubarr-charts/managed-nfs";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StorageMode {
    ExternalNfs,
    ManagedNfs,
}

impl StorageMode {
    pub fn as_str(self) -> &'static str {
        match self {
            StorageMode::ExternalNfs => "external_nfs",
            StorageMode::ManagedNfs => "managed_nfs",
        }
    }
}

impl fmt::Display for StorageMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for StorageMode {
    type Error = AppError;

    fn try_from(value: &str) -> Result<Self> {
        match value {
            "external_nfs" => Ok(StorageMode::ExternalNfs),
            "managed_nfs" => Ok(StorageMode::ManagedNfs),
            _ => Err(AppError::BadRequest(format!(
                "Unsupported storage mode: {}",
                value
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct ExternalNfsConfig {
    pub server: String,
    pub export_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct ManagedNfsConfig {
    pub storage_class: Option<String>,
    #[serde(default = "default_managed_nfs_size")]
    pub size: String,
}

fn default_managed_nfs_size() -> String {
    "1Ti".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StorageConfigRequest {
    pub mode: StorageMode,
    pub external_nfs: Option<ExternalNfsConfig>,
    pub managed_nfs: Option<ManagedNfsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StorageValidationCheck {
    pub name: String,
    pub passed: bool,
    pub warning: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StorageBenchmark {
    pub bytes: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StorageValidationResponse {
    pub valid: bool,
    pub message: String,
    pub checks: Vec<StorageValidationCheck>,
    pub warnings: Vec<String>,
    pub benchmark: Option<StorageBenchmark>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersistedStorageConfig {
    pub mode: StorageMode,
    pub mount_path: String,
    pub uid: i32,
    pub gid: i32,
    pub fs_group: i32,
    pub config_json: Value,
    pub validation_json: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StorageConfigResponse {
    pub mode: StorageMode,
    pub mount_path: String,
    pub uid: i32,
    pub gid: i32,
    pub fs_group: i32,
    pub config_json: Value,
    pub validation_json: Option<Value>,
    pub validated: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl PersistedStorageConfig {
    pub fn validated(&self) -> bool {
        self.validation_json
            .as_ref()
            .and_then(|v| v.get("valid"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn without_validation(mut self) -> Self {
        self.validation_json = None;
        self
    }
}

pub fn kubarr_namespace() -> String {
    std::env::var("KUBARR_NAMESPACE")
        .or_else(|_| std::env::var("POD_NAMESPACE"))
        .unwrap_or_else(|_| "kubarr".to_string())
}

pub fn build_persisted_config(request: &StorageConfigRequest) -> Result<PersistedStorageConfig> {
    let config_json = match request.mode {
        StorageMode::ExternalNfs => {
            let cfg = request.external_nfs.as_ref().ok_or_else(|| {
                AppError::BadRequest("External NFS configuration is required".to_string())
            })?;
            validate_non_empty("NFS server", &cfg.server)?;
            validate_non_empty("NFS export path", &cfg.export_path)?;
            serde_json::to_value(cfg)?
        }
        StorageMode::ManagedNfs => {
            let cfg = request
                .managed_nfs
                .clone()
                .unwrap_or_else(|| ManagedNfsConfig {
                    storage_class: None,
                    size: default_managed_nfs_size(),
                });
            validate_non_empty("Managed NFS size", &cfg.size)?;
            serde_json::to_value(cfg)?
        }
    };

    Ok(PersistedStorageConfig {
        mode: request.mode,
        mount_path: STORAGE_MOUNT_PATH.to_string(),
        uid: DEFAULT_STORAGE_UID,
        gid: DEFAULT_STORAGE_GID,
        fs_group: DEFAULT_STORAGE_FS_GROUP,
        config_json,
        validation_json: None,
    })
}

fn validate_non_empty(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(AppError::BadRequest(format!("{} is required", label)));
    }
    Ok(())
}

pub async fn save_storage_config_to_db(
    db: &DatabaseConnection,
    config: &PersistedStorageConfig,
) -> Result<storage_config::Model> {
    let now = Utc::now();
    let config_json = serde_json::to_string(&config.config_json)?;
    let validation_json = config
        .validation_json
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    let existing = StorageConfig::find().one(db).await?;

    if let Some(existing) = existing {
        let mut active: storage_config::ActiveModel = existing.into();
        active.mode = Set(config.mode.to_string());
        active.mount_path = Set(config.mount_path.clone());
        active.uid = Set(config.uid);
        active.gid = Set(config.gid);
        active.fs_group = Set(config.fs_group);
        active.config_json = Set(config_json);
        active.validation_json = Set(validation_json);
        active.updated_at = Set(now);
        Ok(active.update(db).await?)
    } else {
        let active = storage_config::ActiveModel {
            mode: Set(config.mode.to_string()),
            mount_path: Set(config.mount_path.clone()),
            uid: Set(config.uid),
            gid: Set(config.gid),
            fs_group: Set(config.fs_group),
            config_json: Set(config_json),
            validation_json: Set(validation_json),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        Ok(active.insert(db).await?)
    }
}

pub async fn get_storage_config_from_db(
    db: &DatabaseConnection,
) -> Result<Option<(PersistedStorageConfig, storage_config::Model)>> {
    let Some(model) = StorageConfig::find().one(db).await? else {
        return Ok(None);
    };
    let persisted = persisted_from_model(&model)?;
    Ok(Some((persisted, model)))
}

pub fn persisted_from_model(model: &storage_config::Model) -> Result<PersistedStorageConfig> {
    Ok(PersistedStorageConfig {
        mode: StorageMode::try_from(model.mode.as_str())?,
        mount_path: model.mount_path.clone(),
        uid: model.uid,
        gid: model.gid,
        fs_group: model.fs_group,
        config_json: serde_json::from_str(&model.config_json)?,
        validation_json: model
            .validation_json
            .as_ref()
            .map(|v| serde_json::from_str(v))
            .transpose()?,
    })
}

pub fn response_from_persisted(
    config: PersistedStorageConfig,
    model: Option<&storage_config::Model>,
) -> StorageConfigResponse {
    StorageConfigResponse {
        validated: config.validated(),
        mode: config.mode,
        mount_path: config.mount_path,
        uid: config.uid,
        gid: config.gid,
        fs_group: config.fs_group,
        config_json: config.config_json,
        validation_json: config.validation_json,
        created_at: model.map(|m| m.created_at.to_rfc3339()),
        updated_at: model.map(|m| m.updated_at.to_rfc3339()),
    }
}

pub async fn save_pre_db_secret(k8s: &K8sClient, config: &PersistedStorageConfig) -> Result<()> {
    let config_bytes = serde_json::to_vec(config)?;
    upsert_secret(
        k8s,
        &kubarr_namespace(),
        STORAGE_SECRET_NAME,
        BTreeMap::from([(SECRET_CONFIG_KEY.to_string(), config_bytes)]),
    )
    .await
}

pub async fn load_pre_db_secret(k8s: &K8sClient) -> Result<Option<PersistedStorageConfig>> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client().clone(), &kubarr_namespace());
    let secret = match secrets.get(STORAGE_SECRET_NAME).await {
        Ok(secret) => secret,
        Err(kube::Error::Api(ae)) if ae.code == 404 => return Ok(None),
        Err(e) => return Err(AppError::Kubernetes(e)),
    };

    let Some(data) = secret.data else {
        return Ok(None);
    };
    let Some(bytes) = data.get(SECRET_CONFIG_KEY) else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_slice(&bytes.0)?))
}

pub async fn get_storage_config(
    db: Option<&DatabaseConnection>,
    k8s: Option<&K8sClient>,
) -> Result<Option<StorageConfigResponse>> {
    if let Some(db) = db {
        if let Some((config, model)) = get_storage_config_from_db(db).await? {
            return Ok(Some(response_from_persisted(config, Some(&model))));
        }
    }

    if let Some(k8s) = k8s {
        if let Some(config) = load_pre_db_secret(k8s).await? {
            return Ok(Some(response_from_persisted(config, None)));
        }
    }

    Ok(None)
}

pub async fn storage_ready(
    db: Option<&DatabaseConnection>,
    k8s: Option<&K8sClient>,
) -> Result<bool> {
    if let Some(db) = db {
        if let Some((config, _)) = get_storage_config_from_db(db).await? {
            return Ok(config.validated());
        }
    }

    if let Some(k8s) = k8s {
        if let Some(config) = load_pre_db_secret(k8s).await? {
            return Ok(config.validated());
        }
    }

    Ok(false)
}

pub async fn migrate_pre_db_secret_to_db(
    db: &DatabaseConnection,
    k8s: &K8sClient,
) -> Result<Option<storage_config::Model>> {
    let Some(config) = load_pre_db_secret(k8s).await? else {
        return Ok(None);
    };
    Ok(Some(save_storage_config_to_db(db, &config).await?))
}

async fn upsert_secret(
    k8s: &K8sClient,
    namespace: &str,
    name: &str,
    data: BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    ensure_namespace(k8s, namespace).await?;

    let secrets: Api<Secret> = Api::namespaced(k8s.client().clone(), namespace);
    let secret_data = data
        .into_iter()
        .map(|(key, value)| (key, ByteString(value)))
        .collect();
    let mut secret = Secret {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        type_: Some("Opaque".to_string()),
        data: Some(secret_data),
        ..Default::default()
    };

    match secrets.get(name).await {
        Ok(existing) => {
            secret.metadata.resource_version = existing.metadata.resource_version;
            secrets
                .replace(name, &PostParams::default(), &secret)
                .await?;
        }
        Err(kube::Error::Api(ae)) if ae.code == 404 => {
            secrets.create(&PostParams::default(), &secret).await?;
        }
        Err(e) => return Err(AppError::Kubernetes(e)),
    }

    Ok(())
}

pub async fn provision_storage(k8s: &K8sClient, config: &PersistedStorageConfig) -> Result<()> {
    match config.mode {
        StorageMode::ManagedNfs => ensure_managed_nfs_server(k8s, config).await?,
        StorageMode::ExternalNfs => {}
    }

    ensure_media_pvc_for_namespace(k8s, &kubarr_namespace(), config).await
}

pub async fn ensure_media_pvc_for_namespace(
    k8s: &K8sClient,
    namespace: &str,
    config: &PersistedStorageConfig,
) -> Result<()> {
    ensure_namespace(k8s, namespace).await?;

    match config.mode {
        StorageMode::ExternalNfs => {
            let nfs: ExternalNfsConfig = serde_json::from_value(config.config_json.clone())?;
            ensure_nfs_media_pvc(k8s, namespace, &nfs.server, &nfs.export_path).await
        }
        StorageMode::ManagedNfs => {
            ensure_managed_nfs_server(k8s, config).await?;
            let server = managed_nfs_node_ip(k8s).await?;
            ensure_nfs_media_pvc(k8s, namespace, &server, MANAGED_NFS_EXPORT_PATH).await
        }
    }
}

async fn managed_nfs_node_ip(k8s: &K8sClient) -> Result<String> {
    let pods: Api<Pod> = Api::namespaced(k8s.client().clone(), MANAGED_NFS_NAMESPACE);
    for _ in 0..30 {
        let pod_list = pods
            .list(&ListParams::default().labels(&format!("app={}", MANAGED_NFS_NAME)))
            .await?;
        for pod in pod_list.items {
            if let Some(status) = pod.status {
                if let Some(host_ip) = status.host_ip {
                    return Ok(host_ip);
                }
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    Err(AppError::Internal(
        "Managed NFS pod did not report a node IP".to_string(),
    ))
}

async fn ensure_namespace(k8s: &K8sClient, namespace: &str) -> Result<()> {
    let namespaces: Api<Namespace> = Api::all(k8s.client().clone());
    match namespaces.get(namespace).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(ae)) if ae.code == 404 => {
            let ns = Namespace {
                metadata: ObjectMeta {
                    name: Some(namespace.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            };
            namespaces.create(&PostParams::default(), &ns).await?;
            Ok(())
        }
        Err(e) => Err(AppError::Kubernetes(e)),
    }
}

fn media_pv_name(namespace: &str) -> String {
    format!("media-data-{}", namespace)
}

fn rwx_capacity() -> BTreeMap<String, Quantity> {
    BTreeMap::from([("storage".to_string(), Quantity("1Ti".to_string()))])
}

async fn ensure_nfs_media_pvc(
    k8s: &K8sClient,
    namespace: &str,
    nfs_server: &str,
    nfs_path: &str,
) -> Result<()> {
    let pv_name = media_pv_name(namespace);
    let capacity = rwx_capacity();

    let pvs: Api<PersistentVolume> = Api::all(k8s.client().clone());
    match pvs.get(&pv_name).await {
        Ok(mut existing) => {
            if existing
                .status
                .as_ref()
                .and_then(|status| status.phase.as_deref())
                == Some("Released")
            {
                if let Some(spec) = existing.spec.as_mut() {
                    spec.claim_ref = None;
                }
                pvs.replace(&pv_name, &PostParams::default(), &existing)
                    .await?;
            }
        }
        Err(kube::Error::Api(ae)) if ae.code == 404 => {
            let pv = PersistentVolume {
                metadata: ObjectMeta {
                    name: Some(pv_name.clone()),
                    ..Default::default()
                },
                spec: Some(PersistentVolumeSpec {
                    capacity: Some(capacity.clone()),
                    access_modes: Some(vec!["ReadWriteMany".to_string()]),
                    persistent_volume_reclaim_policy: Some("Retain".to_string()),
                    storage_class_name: Some("".to_string()),
                    nfs: Some(NFSVolumeSource {
                        server: nfs_server.to_string(),
                        path: nfs_path.to_string(),
                        read_only: Some(false),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            };
            pvs.create(&PostParams::default(), &pv).await?;
        }
        Err(e) => return Err(AppError::Kubernetes(e)),
    }

    ensure_static_media_pvc(k8s, namespace, &pv_name, capacity).await
}

async fn ensure_static_media_pvc(
    k8s: &K8sClient,
    namespace: &str,
    pv_name: &str,
    capacity: BTreeMap<String, Quantity>,
) -> Result<()> {
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(k8s.client().clone(), namespace);
    if pvcs.get(MEDIA_PVC_NAME).await.is_err() {
        let pvc = PersistentVolumeClaim {
            metadata: ObjectMeta {
                name: Some(MEDIA_PVC_NAME.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            spec: Some(PersistentVolumeClaimSpec {
                access_modes: Some(vec!["ReadWriteMany".to_string()]),
                storage_class_name: Some("".to_string()),
                volume_name: Some(pv_name.to_string()),
                resources: Some(VolumeResourceRequirements {
                    requests: Some(capacity),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        pvcs.create(&PostParams::default(), &pvc).await?;
    }

    Ok(())
}

async fn ensure_managed_nfs_server(
    _k8s: &K8sClient,
    config: &PersistedStorageConfig,
) -> Result<()> {
    let managed: ManagedNfsConfig = serde_json::from_value(config.config_json.clone())?;

    let mut args = vec![
        "upgrade".to_string(),
        "--install".to_string(),
        MANAGED_NFS_RELEASE.to_string(),
        MANAGED_NFS_CHART_REF.to_string(),
        "-n".to_string(),
        BOOTSTRAP_RELEASE_NAMESPACE.to_string(),
        "--wait".to_string(),
        "--timeout".to_string(),
        "5m".to_string(),
        "--set".to_string(),
        format!("namespace.name={}", MANAGED_NFS_NAMESPACE),
        "--set".to_string(),
        "namespace.create=true".to_string(),
        "--set".to_string(),
        format!("persistence.size={}", managed.size),
    ];

    if let Some(storage_class) = managed.storage_class.filter(|s| !s.trim().is_empty()) {
        args.push("--set-string".to_string());
        args.push(format!("persistence.storageClassName={}", storage_class));
    }

    let output = Command::new("helm")
        .args(&args)
        .output()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to run helm: {}", e)))?;

    if !output.status.success() {
        return Err(AppError::Internal(format!(
            "Managed NFS chart install failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(())
}

pub async fn validate_storage(
    k8s: &K8sClient,
    config: &PersistedStorageConfig,
) -> Result<StorageValidationResponse> {
    let id = Uuid::new_v4().simple().to_string();
    let ns_a = format!("kubarr-storage-validation-a-{}", &id[..8]);
    let ns_b = format!("kubarr-storage-validation-b-{}", &id[..8]);

    let result = validate_storage_inner(k8s, config, &id, &ns_a, &ns_b).await;

    cleanup_validation_namespace(k8s, &ns_a).await;
    cleanup_validation_namespace(k8s, &ns_b).await;

    result
}

async fn validate_storage_inner(
    k8s: &K8sClient,
    config: &PersistedStorageConfig,
    id: &str,
    ns_a: &str,
    ns_b: &str,
) -> Result<StorageValidationResponse> {
    ensure_media_pvc_for_namespace(k8s, ns_a, config).await?;
    ensure_media_pvc_for_namespace(k8s, ns_b, config).await?;

    let writer_script = format!(
        r#"set -eu
mkdir -p /data/kubarr-validation
rm -f /data/kubarr-validation/shared.txt /data/kubarr-validation/shared-renamed.txt /data/kubarr-validation/hardlink.txt /data/kubarr-validation/bench.bin
echo "kubarr-{id}" > /data/kubarr-validation/shared.txt
grep -q "kubarr-{id}" /data/kubarr-validation/shared.txt
mv /data/kubarr-validation/shared.txt /data/kubarr-validation/shared-renamed.txt
grep -q "kubarr-{id}" /data/kubarr-validation/shared-renamed.txt
if ln /data/kubarr-validation/shared-renamed.txt /data/kubarr-validation/hardlink.txt; then echo HARDLINK_OK; else echo HARDLINK_WARNING; fi
rm -f /data/kubarr-validation/hardlink.txt
dd if=/dev/zero of=/data/kubarr-validation/bench.bin bs=1024 count=256 >/tmp/bench.log 2>&1
cat /data/kubarr-validation/bench.bin >/dev/null
cat /tmp/bench.log || true
rm -f /data/kubarr-validation/bench.bin
echo "kubarr-{id}" > /data/kubarr-validation/shared.txt
rm -f /data/kubarr-validation/shared-renamed.txt
echo VALIDATION_OK"#,
    );
    let writer_logs = run_validation_pod(k8s, ns_a, config, "storage-writer", &writer_script)
        .await
        .map_err(|e| AppError::BadRequest(format!("Storage validation failed: {}", e)))?;

    let reader_script = format!(
        r#"set -eu
test -f /data/kubarr-validation/shared.txt
grep -q "kubarr-{id}" /data/kubarr-validation/shared.txt
rm -f /data/kubarr-validation/shared.txt
rmdir /data/kubarr-validation || true
echo CROSS_NAMESPACE_OK"#,
    );
    run_validation_pod(k8s, ns_b, config, "storage-reader", &reader_script)
        .await
        .map_err(|e| AppError::BadRequest(format!("Cross-namespace validation failed: {}", e)))?;

    let hardlink_warning = writer_logs.contains("HARDLINK_WARNING");
    let warnings = if hardlink_warning {
        vec!["Hardlinks are not supported by this storage backend; imports may copy instead of link.".to_string()]
    } else {
        Vec::new()
    };

    Ok(StorageValidationResponse {
        valid: true,
        message: "Storage validation passed".to_string(),
        checks: vec![
            check("mount", true, false, "Mounted media-data at /data"),
            check("folder_create", true, false, "Created validation folder"),
            check(
                "write_read",
                true,
                false,
                "Wrote and read a validation file as UID/GID 1000",
            ),
            check(
                "rename_delete",
                true,
                false,
                "Renamed and deleted files successfully",
            ),
            check(
                "cross_namespace_visibility",
                true,
                false,
                "A second namespace saw the same validation file",
            ),
            check(
                "hardlink",
                !hardlink_warning,
                hardlink_warning,
                if hardlink_warning {
                    "Hardlink failed; this is a warning for some NAS exports"
                } else {
                    "Hardlink succeeded"
                },
            ),
        ],
        warnings,
        benchmark: Some(StorageBenchmark {
            bytes: 262_144,
            message: "Small sequential write/read benchmark completed".to_string(),
        }),
    })
}

fn check(name: &str, passed: bool, warning: bool, message: &str) -> StorageValidationCheck {
    StorageValidationCheck {
        name: name.to_string(),
        passed,
        warning,
        message: message.to_string(),
    }
}

async fn run_validation_pod(
    k8s: &K8sClient,
    namespace: &str,
    config: &PersistedStorageConfig,
    pod_name: &str,
    script: &str,
) -> Result<String> {
    let pods: Api<Pod> = Api::namespaced(k8s.client().clone(), namespace);
    let pod = Pod {
        metadata: ObjectMeta {
            name: Some(pod_name.to_string()),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        spec: Some(PodSpec {
            restart_policy: Some("Never".to_string()),
            security_context: Some(PodSecurityContext {
                run_as_user: Some(config.uid as i64),
                run_as_group: Some(config.gid as i64),
                fs_group: Some(config.fs_group as i64),
                ..Default::default()
            }),
            containers: vec![Container {
                name: "check".to_string(),
                image: Some("busybox:1.36".to_string()),
                command: Some(vec!["/bin/sh".to_string(), "-c".to_string()]),
                args: Some(vec![script.to_string()]),
                volume_mounts: Some(vec![VolumeMount {
                    name: "media".to_string(),
                    mount_path: STORAGE_MOUNT_PATH.to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            }],
            volumes: Some(vec![Volume {
                name: "media".to_string(),
                persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                    claim_name: MEDIA_PVC_NAME.to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    };

    pods.create(&PostParams::default(), &pod).await?;

    for _ in 0..90 {
        let current = pods.get(pod_name).await?;
        if let Some(status) = current.status {
            if let Some(phase) = status.phase {
                if phase == "Succeeded" {
                    return Ok(pods
                        .logs(pod_name, &LogParams::default())
                        .await
                        .unwrap_or_default());
                }
                if phase == "Failed" {
                    let logs = pods
                        .logs(pod_name, &LogParams::default())
                        .await
                        .unwrap_or_default();
                    return Err(AppError::BadRequest(format!(
                        "validation pod failed: {}",
                        logs
                    )));
                }
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    Err(AppError::BadRequest(
        "validation pod did not complete within timeout".to_string(),
    ))
}

async fn cleanup_validation_namespace(k8s: &K8sClient, namespace: &str) {
    let namespaces: Api<Namespace> = Api::all(k8s.client().clone());
    let _ = namespaces.delete(namespace, &DeleteParams::default()).await;
    let pvs: Api<PersistentVolume> = Api::all(k8s.client().clone());
    let _ = pvs
        .delete(&media_pv_name(namespace), &DeleteParams::default())
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_mode_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&StorageMode::ExternalNfs).unwrap(),
            "\"external_nfs\""
        );
        assert_eq!(
            serde_json::from_str::<StorageMode>("\"managed_nfs\"").unwrap(),
            StorageMode::ManagedNfs
        );
    }

    #[test]
    fn validation_flag_reads_response_json() {
        let mut config = PersistedStorageConfig {
            mode: StorageMode::ExternalNfs,
            mount_path: STORAGE_MOUNT_PATH.to_string(),
            uid: 1000,
            gid: 1000,
            fs_group: 1000,
            config_json: serde_json::json!({"server":"nas","export_path":"/media"}),
            validation_json: None,
        };
        assert!(!config.validated());
        config.validation_json = Some(serde_json::json!({"valid": true}));
        assert!(config.validated());
    }
}
