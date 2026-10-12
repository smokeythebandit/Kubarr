//! Validated desired storage, separate from an operation's immutable input snapshot.
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ConnectionTrait, EntityTrait, Set};

use crate::error::{AppError, Result};
use crate::models::app_storage_config;
use crate::services::install_options::{self, InstallOptions, InstallStorage, WorkloadMounts};

pub const SCHEMA_VERSION: i32 = 1;

#[derive(Debug)]
pub struct SavedStorage {
    pub storage: InstallStorage,
    pub chart_version: String,
    /// True only after a successful install whose Helm release was proven absent.
    pub fresh_private_config: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn get<C: ConnectionTrait>(db: &C, app_name: &str) -> Result<Option<SavedStorage>> {
    let Some(row) = app_storage_config::Entity::find_by_id(app_name)
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    if row.schema_version != SCHEMA_VERSION {
        return Err(AppError::BadRequest(format!(
            "Unsupported saved storage schema for '{app_name}'"
        )));
    }
    let storage = serde_json::from_str::<InstallStorage>(&row.config_json).map_err(|_| {
        AppError::BadRequest(format!("Invalid saved storage config for '{app_name}'"))
    })?;
    Ok(Some(SavedStorage {
        storage,
        chart_version: row.chart_version,
        fresh_private_config: row.fresh_private_config,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }))
}

/// A full immutable snapshot: all chart-supported lists and config are explicit.
/// An omitted field keeps its prior saved selection, otherwise takes this chart's
/// defaults. Existing configured fields cannot disappear on a chart upgrade.
pub fn materialize(
    options: &InstallOptions,
    request: &InstallStorage,
    saved: Option<&InstallStorage>,
) -> Result<InstallStorage> {
    if let Some(saved) = saved {
        install_options::validate(options, saved)?;
    }
    install_options::validate(options, request)?;
    let mounts = request
        .mounts
        .as_ref()
        .or(saved.and_then(|s| s.mounts.as_ref()))
        .or(options.mounts.as_ref())
        .cloned();
    let mut workloads = std::collections::BTreeMap::new();
    for (name, defaults) in &options.workloads {
        let selected = request
            .workloads
            .as_ref()
            .and_then(|v| v.get(name))
            .or_else(|| {
                saved
                    .and_then(|s| s.workloads.as_ref())
                    .and_then(|v| v.get(name))
            });
        workloads.insert(
            name.clone(),
            selected.cloned().unwrap_or_else(|| WorkloadMounts {
                mounts: defaults.clone(),
            }),
        );
    }
    let config = request
        .config
        .as_ref()
        .or(saved.and_then(|s| s.config.as_ref()))
        .or(options.config.as_ref())
        .cloned();
    let effective = InstallStorage {
        mounts,
        workloads: (!workloads.is_empty()).then_some(workloads),
        config,
    };
    install_options::validate(options, &effective)?;
    Ok(effective)
}

pub async fn save<C: ConnectionTrait>(
    db: &C,
    app_name: &str,
    chart_version: &str,
    storage: &InstallStorage,
) -> Result<()> {
    let json = serde_json::to_string(storage)?;
    let now = Utc::now();
    if let Some(existing) = app_storage_config::Entity::find_by_id(app_name)
        .one(db)
        .await?
    {
        let prior: InstallStorage = serde_json::from_str(&existing.config_json).map_err(|_| {
            AppError::BadRequest(format!("Invalid saved storage config for '{app_name}'"))
        })?;
        let same_private_config = prior.config == storage.config;
        let mut active: app_storage_config::ActiveModel = existing.into();
        active.schema_version = Set(SCHEMA_VERSION);
        active.chart_version = Set(chart_version.into());
        active.config_json = Set(json);
        if !same_private_config {
            active.fresh_private_config = Set(false);
        }
        active.updated_at = Set(now);
        active.update(db).await?;
    } else {
        app_storage_config::ActiveModel {
            app_name: Set(app_name.into()),
            schema_version: Set(SCHEMA_VERSION),
            chart_version: Set(chart_version.into()),
            config_json: Set(json),
            fresh_private_config: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await?;
    }
    Ok(())
}

/// Called only after Helm completed a proven first installation. A previous
/// release, even a failed one, is never treated as proof of new private data.
pub async fn mark_fresh_private_config<C: ConnectionTrait>(
    db: &C,
    app_name: &str,
    chart_version: &str,
) -> Result<()> {
    let Some(row) = app_storage_config::Entity::find_by_id(app_name)
        .one(db)
        .await?
    else {
        return Err(AppError::Conflict(
            "Saved storage disappeared during install".into(),
        ));
    };
    if row.chart_version != chart_version || row.schema_version != SCHEMA_VERSION {
        return Err(AppError::Conflict(
            "Saved storage changed during install".into(),
        ));
    }
    let mut active: app_storage_config::ActiveModel = row.into();
    active.fresh_private_config = Set(true);
    active.updated_at = Set(Utc::now());
    active.update(db).await?;
    Ok(())
}
