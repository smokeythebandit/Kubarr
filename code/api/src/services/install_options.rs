//! The install UI contract is derived from the pulled chart, never from the browser.
use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::config::CONFIG;
use crate::error::{AppError, Result};
use crate::services::catalog::AppCatalog;

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    pub name: String,
    pub source: String,
    pub target: String,
    pub mode: MountMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MountMode {
    Read,
    ReadWrite,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrivateConfig {
    pub create: bool,
    #[serde(default)]
    pub existing_claim: String,
    #[serde(default)]
    pub storage_class_name: String,
    pub size: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstallStorage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mounts: Option<Vec<Mount>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workloads: Option<BTreeMap<String, WorkloadMounts>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<PrivateConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkloadMounts {
    pub mounts: Vec<Mount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InstallOptions {
    pub app_name: String,
    pub chart_version: String,
    pub mounts: Option<Vec<Mount>>,
    pub workloads: BTreeMap<String, Vec<Mount>>,
    pub config: Option<PrivateConfig>,
    /// Expected claim for unmodified chart naming; None when Helm overrides or truncation apply.
    pub private_claim: Option<String>,
    pub required: BTreeMap<String, Vec<String>>,
    pub reserved: BTreeMap<String, Vec<String>>,
    pub vpn_supported: bool,
    pub gpu_supported: bool,
    /// Desired DB choices. None means this app has no saved storage configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_storage: Option<InstallStorage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_chart_version: Option<String>,
}

// This explicit chart-specific contract supplements the generic common-chart
// validator. Missing writable persistent mounts must never silently fall back
// to the container filesystem. The source of defaults remains values.yaml.
fn required(app: &str, component: &str) -> &'static [&'static str] {
    match (app, component) {
        ("jenkins", "main") => &["/var/jenkins_home"],
        ("scm-manager", "main") => &["/var/lib/scm"],
        ("redmine", "main") => &["/usr/src/redmine/files"],
        ("redmine", "postgres") => &["/var/lib/postgresql/data"],
        ("immich", "main") => &["/data"],
        ("immich", "machineLearning") => &["/cache"],
        ("frigate", "main") => &["/media/frigate"],
        ("victoriametrics", "main") => &["/victoria-metrics-data"],
        ("victorialogs", "main") => &["/data"],
        ("postgresql", "main") => &["/var/lib/postgresql/data"],
        ("qbittorrent" | "transmission" | "deluge" | "rutorrent" | "sabnzbd", "main") => {
            &["/downloads"]
        }
        _ => &[],
    }
}

const VPN_APPS: &[&str] = &[
    "radarr",
    "sonarr",
    "jellyseerr",
    "jackett",
    "plex",
    "jellyfin",
    "qbittorrent",
    "transmission",
    "deluge",
    "rutorrent",
    "sabnzbd",
];

pub fn load(catalog: &AppCatalog, name: &str) -> Result<InstallOptions> {
    let app = catalog
        .get_app(name)
        .ok_or_else(|| AppError::NotFound(format!("App '{name}' not found")))?;
    if app.is_system || app.is_hidden {
        return Err(AppError::BadRequest(
            "This chart is not available for app installs".into(),
        ));
    }
    let path = CONFIG.charts.dir.join(&app.name);
    let version = catalog.chart_version(&app.name).ok_or_else(|| {
        AppError::BadRequest("Chart metadata is unavailable; sync charts before installing".into())
    })?;
    let yaml = std::fs::read_to_string(path.join("values.yaml")).map_err(|_| {
        AppError::BadRequest("Chart values are unavailable; sync charts before installing".into())
    })?;
    let values: serde_yaml::Value = serde_yaml::from_str(&yaml)
        .map_err(|_| AppError::BadRequest("Invalid chart values".into()))?;
    let storage = values.get("storage");
    let parse_mounts = |value: &serde_yaml::Value| -> Result<Vec<Mount>> {
        serde_yaml::from_value(value.clone())
            .map_err(|_| AppError::BadRequest("Invalid chart mount defaults".into()))
    };
    let mounts = storage
        .and_then(|s| s.get("mounts"))
        .map(&parse_mounts)
        .transpose()?;
    let mut workloads = BTreeMap::new();
    if let Some(map) = storage
        .and_then(|s| s.get("workloads"))
        .and_then(serde_yaml::Value::as_mapping)
    {
        for (name, value) in map {
            let key = name
                .as_str()
                .ok_or_else(|| AppError::BadRequest("Invalid chart component".into()))?;
            workloads.insert(
                key.to_string(),
                parse_mounts(
                    value
                        .get("mounts")
                        .ok_or_else(|| AppError::BadRequest("Missing component mounts".into()))?,
                )?,
            );
        }
    }
    let config = storage
        .and_then(|s| s.get("config"))
        .map(|c| PrivateConfig {
            create: c
                .get("create")
                .and_then(serde_yaml::Value::as_bool)
                .unwrap_or(true),
            existing_claim: c
                .get("existingClaim")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or("")
                .into(),
            storage_class_name: c
                .get("storageClassName")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or("")
                .into(),
            size: c
                .get("size")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or("5Gi")
                .into(),
        });
    let private_claim = config.as_ref().and_then(|_| {
        let claim = format!("{}-config", app.name);
        (claim.len() <= 63
            && values
                .get("fullnameOverride")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or("")
                .is_empty()
            && values
                .get("nameOverride")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or("")
                .is_empty())
        .then_some(claim)
    });
    let mut required_paths = BTreeMap::new();
    let mut reserved = BTreeMap::new();
    if mounts.is_some() {
        required_paths.insert(
            "main".into(),
            required(&app.name, "main")
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );
        let paths: &[&str] = match app.name.as_str() {
            "jellyseerr" => &["/app/config"],
            "radarr" | "sonarr" => &[
                "/config",
                "/custom-services.d/setup-downloadclients",
                "/custom-services.d/setup-indexers",
            ],
            "qbittorrent" => &["/config", "/custom-cont-init.d", "/config-immutable"],
            "deluge" => &["/config", "/custom-cont-init.d/enable-label-plugin.sh"],
            "rutorrent" => &["/config", "/var/lib/nginx"],
            "sabnzbd" => &["/config", "/custom-services.d/setup-apikey"],
            "frigate" => &["/config", "/dev/shm", "/tmp/cache"],
            "immich" | "victorialogs" => &["/tmp"],
            "victoriametrics" => &["/etc/prometheus", "/tmp"],
            _ => &["/config"],
        };
        reserved.insert("main".into(), paths.iter().map(|s| s.to_string()).collect());
    }
    for component in workloads.keys() {
        required_paths.insert(
            component.clone(),
            required(&app.name, component)
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );
        reserved.insert(
            component.clone(),
            match (app.name.as_str(), component.as_str()) {
                ("redmine", "postgres") => vec!["/dev/shm".into(), "/config".into()],
                ("immich", "machineLearning") => vec!["/nfs".into(), "/config".into()],
                _ => vec!["/config".into()],
            },
        );
    }
    let options = InstallOptions {
        app_name: app.name.clone(),
        chart_version: version,
        mounts,
        workloads,
        config,
        private_claim,
        required: required_paths,
        reserved,
        vpn_supported: VPN_APPS.contains(&app.name.as_str())
            && values.get("vpn").and_then(|v| v.get("enabled")).is_some(),
        gpu_supported: matches!(app.name.as_str(), "plex" | "jellyfin"),
        saved_storage: None,
        saved_chart_version: None,
    };
    validate(&options, &InstallStorage::default())?;
    Ok(options)
}

fn dns(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && s.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && s.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
}
fn storage_class(s: &str) -> bool {
    s.len() <= 253 && s.split('.').all(dns)
}
fn binary_size(size: &str) -> bool {
    size.len() <= 24
        && ["Ki", "Mi", "Gi", "Ti", "Pi", "Ei"].iter().any(|unit| {
            size.strip_suffix(unit).is_some_and(|n| {
                !n.is_empty() && !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit())
            })
        })
}
fn overlap(a: &str, b: &str) -> bool {
    a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"))
}
fn valid_source(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 512
        && s.split('/').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
        && s != "config"
        && !s.starts_with("config/")
}
fn valid_target(s: &str) -> bool {
    s.starts_with('/')
        && s.len() <= 512
        && s.len() > 1
        && s[1..].split('/').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

fn validate_mounts(mounts: &[Mount], required: &[String], reserved: &[String]) -> Result<()> {
    if mounts.len() > 64 {
        return Err(AppError::BadRequest("Too many mounts".into()));
    }
    let mut names = HashSet::new();
    for (i, m) in mounts.iter().enumerate() {
        if !dns(&m.name)
            || !names.insert(&m.name)
            || !valid_source(&m.source)
            || !valid_target(&m.target)
        {
            return Err(AppError::BadRequest(format!(
                "Invalid storage mount at index {i}"
            )));
        }
        if reserved.iter().any(|path| overlap(&m.target, path))
            || mounts[..i]
                .iter()
                .any(|other| overlap(&m.target, &other.target))
        {
            return Err(AppError::BadRequest(format!(
                "Storage target {} overlaps another or a reserved path",
                m.target
            )));
        }
        if mounts[..i]
            .iter()
            .any(|other| other.mode != m.mode && overlap(&m.source, &other.source))
        {
            return Err(AppError::BadRequest(
                "Overlapping sources cannot have different modes".into(),
            ));
        }
    }
    for target in required {
        if !mounts
            .iter()
            .any(|m| &m.target == target && m.mode == MountMode::ReadWrite)
        {
            return Err(AppError::BadRequest(format!(
                "A writable storage mount at {target} is required"
            )));
        }
    }
    Ok(())
}

pub fn validate(options: &InstallOptions, choices: &InstallStorage) -> Result<()> {
    if choices.mounts.is_some() && options.mounts.is_none() {
        return Err(AppError::BadRequest(
            "This chart has no configurable shared mounts".into(),
        ));
    }
    if choices.config.is_some() && options.config.is_none() {
        return Err(AppError::BadRequest(
            "This chart has no private config claim".into(),
        ));
    }
    if let Some(mounts) = choices.mounts.as_ref().or(options.mounts.as_ref()) {
        validate_mounts(
            mounts,
            options
                .required
                .get("main")
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            options
                .reserved
                .get("main")
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        )?;
    }
    if let Some(ref overrides) = choices.workloads {
        if overrides.len() > options.workloads.len()
            || overrides.keys().any(|k| !options.workloads.contains_key(k))
        {
            return Err(AppError::BadRequest("Unknown storage workload".into()));
        }
    }
    for (name, defaults) in &options.workloads {
        let mounts = choices
            .workloads
            .as_ref()
            .and_then(|map| map.get(name))
            .map(|v| &v.mounts)
            .unwrap_or(defaults);
        validate_mounts(
            mounts,
            options.required.get(name).map(Vec::as_slice).unwrap_or(&[]),
            options.reserved.get(name).map(Vec::as_slice).unwrap_or(&[]),
        )?;
    }
    if let Some(config) = choices.config.as_ref().or(options.config.as_ref()) {
        if !binary_size(&config.size) {
            return Err(AppError::BadRequest("Invalid private config size".into()));
        }
        if config.create {
            if !config.existing_claim.is_empty() || !binary_size(&config.size) {
                return Err(AppError::BadRequest(
                    "Invalid private config claim size or existing claim".into(),
                ));
            }
            if !config.storage_class_name.is_empty() && !storage_class(&config.storage_class_name) {
                return Err(AppError::BadRequest("Invalid config StorageClass".into()));
            }
        } else if !dns(&config.existing_claim)
            || !config.storage_class_name.is_empty()
            || config.existing_claim == "media-data"
        {
            return Err(AppError::BadRequest(
                "Select a distinct existing private config claim".into(),
            ));
        }
    }
    Ok(())
}

pub fn helm_values(storage: &InstallStorage) -> serde_json::Value {
    let mut result = serde_json::Map::new();
    if let Some(mounts) = &storage.mounts {
        result.insert("mounts".into(), serde_json::json!(mounts));
    }
    if let Some(workloads) = &storage.workloads {
        result.insert("workloads".into(), serde_json::json!(workloads));
    }
    if let Some(config) = &storage.config {
        result.insert("config".into(), serde_json::json!({"create": config.create, "existingClaim": config.existing_claim, "storageClassName": config.storage_class_name, "size": config.size}));
    }
    serde_json::json!({"storage": result})
}

/// Legacy Helm overrides must not change managed storage alongside a saved
/// typed snapshot (or bypass the private-config migration guard).
pub fn validate_custom_storage_keys(
    config: &std::collections::HashMap<String, String>,
) -> Result<()> {
    fn managed(key: &str) -> bool {
        let key = key.trim_start().trim_start_matches('{').trim_start();
        key == "storage" || key.starts_with("storage.") || key.starts_with("storage[")
    }
    for (key, value) in config {
        if key.split(',').any(managed)
            || value
                .split(',')
                .skip(1)
                .filter_map(|fragment| fragment.split_once('=').map(|(k, _)| k))
                .any(managed)
        {
            return Err(AppError::BadRequest(format!(
                "custom_config key '{key}' overrides managed storage; use install storage choices"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mount(name: &str, source: &str, target: &str, mode: MountMode) -> Mount {
        Mount {
            name: name.into(),
            source: source.into(),
            target: target.into(),
            mode,
        }
    }
    fn options() -> InstallOptions {
        InstallOptions {
            app_name: "redmine".into(),
            chart_version: "1.0.0".into(),
            mounts: Some(vec![mount(
                "attachments",
                "development/redmine/files",
                "/usr/src/redmine/files",
                MountMode::ReadWrite,
            )]),
            workloads: BTreeMap::from([(
                "postgres".into(),
                vec![mount(
                    "database",
                    "development/redmine/db",
                    "/var/lib/postgresql/data",
                    MountMode::ReadWrite,
                )],
            )]),
            config: None,
            private_claim: None,
            required: BTreeMap::from([
                ("main".into(), vec!["/usr/src/redmine/files".into()]),
                ("postgres".into(), vec!["/var/lib/postgresql/data".into()]),
            ]),
            reserved: BTreeMap::from([("main".into(), vec!["/config".into()])]),
            vpn_supported: false,
            gpu_supported: false,
            saved_storage: None,
            saved_chart_version: None,
        }
    }
    #[test]
    fn required_mounts_and_explicit_empty_are_distinct_from_defaults() {
        assert!(validate(&options(), &InstallStorage::default()).is_ok());
        let empty = InstallStorage {
            mounts: Some(vec![]),
            ..Default::default()
        };
        assert!(validate(&options(), &empty)
            .unwrap_err()
            .to_string()
            .contains("writable"));
        let empty_component = InstallStorage {
            workloads: Some(BTreeMap::from([(
                "postgres".into(),
                WorkloadMounts { mounts: vec![] },
            )])),
            ..Default::default()
        };
        assert!(validate(&options(), &empty_component).is_err());
        assert_eq!(
            helm_values(&InstallStorage::default())["storage"],
            serde_json::json!({})
        );
        assert_eq!(
            helm_values(&empty)["storage"]["mounts"],
            serde_json::json!([])
        );
        let yaml = serde_yaml::to_string(&helm_values(&InstallStorage {
            mounts: Some(vec![mount(
                "library",
                "media/movies",
                "/data",
                MountMode::Read,
            )]),
            workloads: Some(BTreeMap::from([(
                "postgres".into(),
                WorkloadMounts {
                    mounts: vec![mount(
                        "database",
                        "redmine/postgres",
                        "/var/lib/postgresql/data",
                        MountMode::ReadWrite,
                    )],
                },
            )])),
            ..Default::default()
        }))
        .unwrap();
        let rendered: serde_json::Value = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(rendered["storage"]["mounts"][0]["source"], "media/movies");
        assert_eq!(rendered["storage"]["mounts"][0]["mode"], "read");
        assert_eq!(
            rendered["storage"]["workloads"]["postgres"]["mounts"][0]["target"],
            "/var/lib/postgresql/data"
        );
    }
    #[test]
    fn rejects_traversal_reserved_targets_and_mixed_source_modes() {
        for source in [".", "../media", "/media", "config/old", "a//b"] {
            let choices = InstallStorage {
                mounts: Some(vec![mount(
                    "attachments",
                    source,
                    "/usr/src/redmine/files",
                    MountMode::ReadWrite,
                )]),
                ..Default::default()
            };
            assert!(validate(&options(), &choices).is_err(), "{source}");
        }
        let choices = InstallStorage {
            mounts: Some(vec![
                mount(
                    "attachments",
                    "media",
                    "/usr/src/redmine/files",
                    MountMode::ReadWrite,
                ),
                mount("other", "media/incoming", "/config/secret", MountMode::Read),
            ]),
            ..Default::default()
        };
        assert!(validate(&options(), &choices).is_err());
        let choices = InstallStorage {
            mounts: Some(vec![
                mount(
                    "attachments",
                    "media",
                    "/usr/src/redmine/files",
                    MountMode::Read,
                ),
                mount("other", "media/incoming", "/other", MountMode::ReadWrite),
            ]),
            ..Default::default()
        };
        assert!(validate(&options(), &choices).is_err());
        let unknown = InstallStorage {
            workloads: Some(BTreeMap::from([(
                "unknown".into(),
                WorkloadMounts { mounts: vec![] },
            )])),
            ..Default::default()
        };
        assert!(validate(&options(), &unknown).is_err());
    }
    #[test]
    fn private_config_choices_cannot_use_shared_claim_or_invalid_size() {
        let mut opts = options();
        opts.config = Some(PrivateConfig {
            create: true,
            existing_claim: "".into(),
            storage_class_name: "".into(),
            size: "5Gi".into(),
        });
        let choice = |config| InstallStorage {
            config: Some(config),
            ..Default::default()
        };
        assert!(validate(
            &opts,
            &choice(PrivateConfig {
                create: false,
                existing_claim: "private-config".into(),
                storage_class_name: "".into(),
                size: "5Gi".into()
            })
        )
        .is_ok());
        assert!(validate(
            &opts,
            &choice(PrivateConfig {
                create: false,
                existing_claim: "media-data".into(),
                storage_class_name: "".into(),
                size: "5Gi".into()
            })
        )
        .is_err());
        assert!(validate(
            &opts,
            &choice(PrivateConfig {
                create: true,
                existing_claim: "".into(),
                storage_class_name: "".into(),
                size: "0Gi".into()
            })
        )
        .is_err());
    }
    #[test]
    fn unknown_fields_and_modes_fail_deserialization() {
        for raw in [
            r#"{"mounts":[{"name":"a","source":"a","target":"/a","mode":"writable"}]}"#,
            r#"{"mounts":[],"migrationConfirmed":true}"#,
        ] {
            assert!(serde_json::from_str::<InstallStorage>(raw).is_err());
        }
    }
    #[test]
    fn custom_values_cannot_override_typed_storage() {
        for (key, value) in [
            ("storage.mounts[0].source", "data"),
            ("image.tag,storage.config.create", "false"),
            ("image.tag", "stable,storage.mounts[0].mode=read-write"),
        ] {
            assert!(
                validate_custom_storage_keys(&std::collections::HashMap::from([(
                    key.into(),
                    value.into()
                )]))
                .is_err()
            );
        }
        validate_custom_storage_keys(&std::collections::HashMap::from([(
            "image.tag".into(),
            "stable".into(),
        )]))
        .unwrap();
    }
}
