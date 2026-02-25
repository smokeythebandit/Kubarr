use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use chrono::Utc;
use once_cell::sync::Lazy;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::middleware::permissions::{Authorized, SettingsManage, SettingsView};
use crate::models::prelude::*;
use crate::models::system_setting;
use crate::state::AppState;

/// Default settings values (keyed by setting name)
static DEFAULT_SETTINGS: Lazy<HashMap<&'static str, (&'static str, &'static str)>> =
    Lazy::new(HashMap::new);

/// Create settings routes
pub fn settings_routes(state: AppState) -> Router {
    Router::new()
        .route("/", get(list_settings))
        .route("/{key}", get(get_setting).put(update_setting))
        .with_state(state)
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SettingResponse {
    pub key: String,
    pub value: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SettingUpdate {
    pub value: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SettingsResponse {
    pub settings: HashMap<String, SettingResponse>,
}

// ============================================================================
// Endpoint Handlers
// ============================================================================

/// List all system settings (requires settings.view permission)
#[utoipa::path(
    get,
    path = "/api/settings",
    tag = "Settings",
    responses(
        (status = 200, body = SettingsResponse)
    )
)]
async fn list_settings(
    State(state): State<AppState>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<SettingsResponse>> {
    let db = state.get_db().await?;
    // Get all settings from database
    let db_settings = SystemSetting::find().all(&db).await?;

    let db_map: HashMap<String, system_setting::Model> = db_settings
        .into_iter()
        .map(|s| (s.key.clone(), s))
        .collect();

    // Merge with defaults
    let mut settings = HashMap::new();
    for (key, (default_value, description)) in DEFAULT_SETTINGS.iter() {
        let setting = if let Some(db_setting) = db_map.get(*key) {
            SettingResponse {
                key: key.to_string(),
                value: db_setting.value.clone(),
                description: db_setting
                    .description
                    .clone()
                    .or_else(|| Some(description.to_string())),
            }
        } else {
            SettingResponse {
                key: key.to_string(),
                value: default_value.to_string(),
                description: Some(description.to_string()),
            }
        };
        settings.insert(key.to_string(), setting);
    }

    Ok(Json(SettingsResponse { settings }))
}

/// Get a specific setting (requires settings.view permission)
#[utoipa::path(
    get,
    path = "/api/settings/{key}",
    tag = "Settings",
    params(
        ("key" = String, Path, description = "Setting key"),
    ),
    responses(
        (status = 200, body = SettingResponse)
    )
)]
async fn get_setting(
    State(state): State<AppState>,
    Path(key): Path<String>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<SettingResponse>> {
    let db = state.get_db().await?;
    let db_setting = SystemSetting::find_by_id(&key).one(&db).await?;

    if let Some(setting) = db_setting {
        return Ok(Json(SettingResponse {
            key: setting.key,
            value: setting.value,
            description: setting.description,
        }));
    }

    // Check defaults
    if let Some((default_value, description)) = DEFAULT_SETTINGS.get(key.as_str()) {
        return Ok(Json(SettingResponse {
            key,
            value: default_value.to_string(),
            description: Some(description.to_string()),
        }));
    }

    Err(AppError::NotFound(format!("Setting '{}' not found", key)))
}

/// Update a system setting (requires settings.manage permission)
#[utoipa::path(
    put,
    path = "/api/settings/{key}",
    tag = "Settings",
    params(
        ("key" = String, Path, description = "Setting key"),
    ),
    request_body = SettingUpdate,
    responses(
        (status = 200, body = SettingResponse)
    )
)]
async fn update_setting(
    State(state): State<AppState>,
    Path(key): Path<String>,
    _auth: Authorized<SettingsManage>,
    Json(data): Json<SettingUpdate>,
) -> Result<Json<SettingResponse>> {
    let db = state.get_db().await?;
    // Validate key exists in defaults
    let (_, description) = DEFAULT_SETTINGS
        .get(key.as_str())
        .ok_or_else(|| AppError::BadRequest(format!("Unknown setting key '{}'", key)))?;

    let now = Utc::now();

    // Check if setting exists
    let existing = SystemSetting::find_by_id(&key).one(&db).await?;

    let setting = if let Some(existing_setting) = existing {
        // Update existing
        let mut setting_model: system_setting::ActiveModel = existing_setting.into();
        setting_model.value = Set(data.value.clone());
        setting_model.updated_at = Set(now);
        setting_model.update(&db).await?
    } else {
        // Insert new
        let new_setting = system_setting::ActiveModel {
            key: Set(key.clone()),
            value: Set(data.value.clone()),
            description: Set(Some(description.to_string())),
            updated_at: Set(now),
        };
        new_setting.insert(&db).await?
    };

    Ok(Json(SettingResponse {
        key: setting.key,
        value: setting.value,
        description: setting.description,
    }))
}

// ============================================================================
// Public helper functions (used by tests and other modules)
// ============================================================================

/// Get a setting value from the database, falling back to defaults.
/// Returns None if the key is not found in the database or defaults.
pub async fn get_setting_value(
    db: &sea_orm::DatabaseConnection,
    key: &str,
) -> crate::error::Result<Option<String>> {
    // Check DB first
    let db_setting = SystemSetting::find_by_id(key).one(db).await?;
    if let Some(setting) = db_setting {
        return Ok(Some(setting.value));
    }
    // Fall back to defaults
    if let Some((default_value, _)) = DEFAULT_SETTINGS.get(key) {
        return Ok(Some(default_value.to_string()));
    }
    Ok(None)
}

/// Get a setting value as a boolean.
/// Returns true if the value is "true", "yes", or "1" (case-insensitive).
/// Returns false for unknown keys or other values.
pub async fn get_setting_bool(
    db: &sea_orm::DatabaseConnection,
    key: &str,
) -> crate::error::Result<bool> {
    let value = get_setting_value(db, key).await?;
    Ok(match value.as_deref() {
        Some(v) => matches!(v.to_lowercase().as_str(), "true" | "yes" | "1"),
        None => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_response_ser() {
        let r = SettingResponse {
            key: "some_key".to_string(),
            value: "some_value".to_string(),
            description: Some("A setting".to_string()),
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"key\":\"some_key\""));
    }

    #[test]
    fn setting_update_deser() {
        let u: SettingUpdate = serde_json::from_str(r#"{"value":"false"}"#).expect("deser");
        assert_eq!(u.value, "false");
    }
}
