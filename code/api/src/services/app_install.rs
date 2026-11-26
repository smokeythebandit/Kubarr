//! Installing an app from reviewed install choices (the install wizard's
//! path). Shared by `POST /api/apps/install` and the assistant's
//! `install_app` action, so both enforce the same rules: the chart must be
//! the reviewed version, storage choices must fit the chart, GPU and VPN only
//! where the chart supports them (VPN needs `vpn.manage`), system and hidden
//! apps are refused, and only one operation per app at a time.

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait};

use crate::error::{AppError, Result};
use crate::models::{app_operation, app_vpn_config};
use crate::services::app_storage;
use crate::services::gpu::GpuSelection;
use crate::services::install_options::{self, InstallStorage};
use crate::services::vpn::AssignVpnRequest;
use crate::services::{AppManager, AppOperationResponse};
use crate::state::SharedCatalog;

/// What the person reviewed before installing.
#[derive(Debug, Clone, Default)]
pub struct InstallChoices {
    /// The chart version the choices were made for; must still be current.
    pub chart_version: Option<String>,
    pub storage: Option<InstallStorage>,
    /// Omitted keeps an existing assignment; `Some(None)` is explicit "no VPN".
    pub vpn: Option<Option<AssignVpnRequest>>,
    pub gpu: Option<Option<GpuSelection>>,
}

/// The current chart version of an installable app, or an error when the
/// app may not be installed from the catalog (system, hidden, no chart).
pub async fn installable_chart_version(catalog: &SharedCatalog, app_name: &str) -> Result<String> {
    let catalog = catalog.read().await;
    let app = catalog
        .get_app(app_name)
        .ok_or_else(|| AppError::NotFound(format!("App '{app_name}' not found in catalog")))?;
    if app.is_system || app.is_hidden {
        return Err(AppError::Forbidden(format!(
            "'{app_name}' is part of Kubarr and cannot be installed from the catalog"
        )));
    }
    let options = install_options::load(&catalog, app_name)?;
    Ok(options.chart_version)
}

/// Queue an install from reviewed choices; queue, VPN assignment and the
/// durable desired storage are committed together or not at all.
pub async fn install_with_choices(
    db: &DatabaseConnection,
    catalog: &SharedCatalog,
    manager: &AppManager,
    app_name: &str,
    choices: InstallChoices,
    actor: i64,
    permissions: &[String],
) -> Result<AppOperationResponse> {
    let options = {
        let catalog = catalog.read().await;
        install_options::load(&catalog, app_name)?
    };
    if choices.chart_version.as_deref() != Some(options.chart_version.as_str()) {
        return Err(AppError::Conflict(
            "Chart changed; reload install options and review again".into(),
        ));
    }
    let requested_storage = choices.storage.unwrap_or_default();
    install_options::validate(&options, &requested_storage)?;
    if choices.gpu.is_some() && !options.gpu_supported {
        return Err(AppError::BadRequest(
            "GPU is unsupported for this chart".into(),
        ));
    }
    if choices.vpn.as_ref().is_some_and(|v| v.is_some()) && !options.vpn_supported {
        return Err(AppError::BadRequest(
            "VPN is unsupported for this chart".into(),
        ));
    }
    let can_manage_vpn = permissions.iter().any(|p| p == "vpn.manage");
    if choices.vpn.as_ref().is_some_and(|v| v.is_some()) && !can_manage_vpn {
        return Err(AppError::Forbidden(
            "vpn.manage required for install-time VPN".into(),
        ));
    }
    let tx = db.begin().await?;
    let active = app_operation::Entity::find()
        .filter(app_operation::Column::AppName.eq(app_name))
        .filter(app_operation::Column::Status.is_in(["queued", "running", "paused"]))
        .one(&tx)
        .await?;
    if active.is_some() {
        return Err(AppError::Conflict(
            "An app operation is already in progress".into(),
        ));
    }
    let saved = app_storage::get(&tx, app_name).await?;
    let storage = app_storage::materialize(
        &options,
        &requested_storage,
        saved.as_ref().map(|s| &s.storage),
    )?;
    let fresh_private_config = saved
        .as_ref()
        .is_some_and(|s| s.fresh_private_config && s.storage.config == storage.config);
    let existing = app_vpn_config::Entity::find_by_id(app_name)
        .one(&tx)
        .await?;
    if existing.is_some() && !options.vpn_supported && !matches!(choices.vpn, Some(None)) {
        return Err(AppError::BadRequest("This chart cannot install with its existing VPN assignment; remove it explicitly with vpn.manage".into()));
    }
    if matches!(choices.vpn, Some(None)) && existing.is_some() && !can_manage_vpn {
        return Err(AppError::Forbidden(
            "vpn.manage required to remove an existing VPN assignment".into(),
        ));
    }
    if let Some(vpn) = choices.vpn {
        if let Some(vpn) = vpn {
            crate::services::vpn::assign_vpn_to_app(&tx, app_name, vpn).await?;
        } else {
            crate::services::vpn::remove_vpn_from_app(&tx, app_name).await?;
        }
    }
    let result = manager
        .enqueue_install_in_transaction(
            &tx,
            app_name,
            &options.chart_version,
            storage.clone(),
            fresh_private_config,
            choices.gpu,
            actor,
        )
        .await?;
    app_storage::save(&tx, app_name, &options.chart_version, &storage).await?;
    tx.commit().await?;
    Ok(result)
}
