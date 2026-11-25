//! Settings for scheduled automatic app updates (`/api/updates`).

use axum::{extract::State, routing::get, Json, Router};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::Result;
use crate::middleware::permissions::{Authorized, SettingsManage, SettingsView};
use crate::services::auto_update_settings::{self, AutoUpdateSettings};
use crate::state::AppState;

pub fn updates_routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/settings",
            get(get_update_settings).put(put_update_settings),
        )
        .with_state(state)
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AutoUpdateSettingsResponse {
    #[serde(flatten)]
    pub settings: AutoUpdateSettings,
    /// Start (UTC) of the open run of scheduled hours containing now, otherwise
    /// of the next one; null when the schedule has no hours enabled.
    pub next_window_start: Option<DateTime<Utc>>,
    /// False until an owner has saved a timezone; the reported one is then a default.
    pub timezone_saved: bool,
}

fn respond(settings: AutoUpdateSettings, timezone_saved: bool) -> Json<AutoUpdateSettingsResponse> {
    Json(AutoUpdateSettingsResponse {
        next_window_start: settings.next_window_start(Utc::now()),
        timezone_saved,
        settings,
    })
}

/// Get the automatic update settings
#[utoipa::path(
    get,
    path = "/api/updates/settings",
    tag = "Updates",
    responses((status = 200, body = AutoUpdateSettingsResponse))
)]
pub async fn get_update_settings(
    State(state): State<AppState>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<AutoUpdateSettingsResponse>> {
    let db = state.get_db().await?;
    let settings = auto_update_settings::load(&db).await?;
    let saved = auto_update_settings::timezone_saved(&db).await?;
    Ok(respond(settings, saved))
}

/// Replace the automatic update settings
#[utoipa::path(
    put,
    path = "/api/updates/settings",
    tag = "Updates",
    request_body = AutoUpdateSettings,
    responses(
        (status = 200, body = AutoUpdateSettingsResponse),
        (status = 400, description = "A field is invalid; the message names it")
    )
)]
pub async fn put_update_settings(
    State(state): State<AppState>,
    auth: Authorized<SettingsManage>,
    Json(settings): Json<AutoUpdateSettings>,
) -> Result<Json<AutoUpdateSettingsResponse>> {
    let db = state.get_db().await?;
    auto_update_settings::save(
        &db,
        &settings,
        Some((auth.user_id(), auth.user().username.clone())),
    )
    .await?;
    Ok(respond(settings, true))
}
