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
use crate::state::{AppState, DbConn};

/// Default settings values
static DEFAULT_SETTINGS: Lazy<HashMap<&'static str, (&'static str, &'static str)>> =
    Lazy::new(|| {
        let mut m = HashMap::new();
        m.insert(
            "registration_enabled",
            (
                "true",
                "Allow new user registration (invites still work when disabled)",
            ),
        );
        m.insert(
            "registration_require_approval",
            ("true", "Require admin approval for new registrations"),
        );
        m
    });

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

/// Get a setting value from the database (helper for other modules)
#[allow(dead_code)]
pub async fn get_setting_value(db: &DbConn, key: &str) -> Result<Option<String>> {
    let setting = SystemSetting::find_by_id(key).one(db).await?;

    if let Some(s) = setting {
        return Ok(Some(s.value));
    }

    // Return default if exists
    if let Some((default_value, _)) = DEFAULT_SETTINGS.get(key) {
        return Ok(Some(default_value.to_string()));
    }

    Ok(None)
}

/// Get a boolean setting value (helper for other modules)
#[allow(dead_code)]
pub async fn get_setting_bool(db: &DbConn, key: &str) -> Result<bool> {
    let value = get_setting_value(db, key).await?;
    Ok(value
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes"))
        .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_has_registration_enabled() {
        assert!(DEFAULT_SETTINGS.contains_key("registration_enabled"));
        let (val, _desc) = DEFAULT_SETTINGS["registration_enabled"];
        assert_eq!(val, "true");
    }

    #[test]
    fn default_settings_has_registration_require_approval() {
        assert!(DEFAULT_SETTINGS.contains_key("registration_require_approval"));
    }

    #[test]
    fn default_settings_count() {
        assert_eq!(DEFAULT_SETTINGS.len(), 2);
    }

    #[test]
    fn setting_response_ser() {
        let r = SettingResponse {
            key: "registration_enabled".to_string(),
            value: "true".to_string(),
            description: Some("Allow registration".to_string()),
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"key\":\"registration_enabled\""));
    }

    #[test]
    fn setting_update_deser() {
        let u: SettingUpdate = serde_json::from_str(r#"{"value":"false"}"#).expect("deser");
        assert_eq!(u.value, "false");
    }

    async fn make_db() -> crate::state::DbConn {
        crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("in-memory SQLite")
    }

    #[tokio::test]
    async fn get_setting_value_returns_default_when_not_in_db() {
        let db = make_db().await;
        let val = get_setting_value(&db, "registration_enabled")
            .await
            .expect("should succeed");
        assert_eq!(val, Some("true".to_string()));
    }

    #[tokio::test]
    async fn get_setting_value_returns_none_for_unknown_key() {
        let db = make_db().await;
        let val = get_setting_value(&db, "nonexistent_key")
            .await
            .expect("should succeed");
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn get_setting_bool_true_for_default_registration() {
        let db = make_db().await;
        let val = get_setting_bool(&db, "registration_enabled")
            .await
            .expect("should succeed");
        assert!(val);
    }

    #[tokio::test]
    async fn get_setting_bool_false_for_unknown_key() {
        let db = make_db().await;
        let val = get_setting_bool(&db, "nonexistent_key")
            .await
            .expect("should succeed");
        assert!(!val);
    }

    #[tokio::test]
    async fn get_setting_value_returns_seeded_db_value() {
        // The seed migration inserts registration_enabled=true
        let db = make_db().await;
        let val = get_setting_value(&db, "registration_enabled")
            .await
            .expect("should succeed");
        // DB value returned (seeded as "true")
        assert_eq!(val, Some("true".to_string()));
    }

    #[tokio::test]
    async fn get_setting_value_db_value_takes_precedence_over_default() {
        use crate::models::prelude::SystemSetting;
        use crate::models::system_setting;
        use chrono::Utc;
        use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};

        let db = make_db().await;
        // Update the already-seeded row to a different value
        let existing = SystemSetting::find_by_id("registration_enabled")
            .one(&db)
            .await
            .expect("query")
            .expect("seeded row must exist");
        let mut active: system_setting::ActiveModel = existing.into_active_model();
        active.value = Set("false".to_string());
        active.updated_at = Set(Utc::now());
        active.update(&db).await.expect("update setting");

        let val = get_setting_value(&db, "registration_enabled")
            .await
            .expect("should succeed");
        assert_eq!(val, Some("false".to_string()));
    }

    #[tokio::test]
    async fn get_setting_bool_false_when_db_value_is_false() {
        use crate::models::prelude::SystemSetting;
        use crate::models::system_setting;
        use chrono::Utc;
        use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};

        let db = make_db().await;
        let existing = SystemSetting::find_by_id("registration_enabled")
            .one(&db)
            .await
            .expect("query")
            .expect("seeded row must exist");
        let mut active: system_setting::ActiveModel = existing.into_active_model();
        active.value = Set("false".to_string());
        active.updated_at = Set(Utc::now());
        active.update(&db).await.expect("update setting");

        let val = get_setting_bool(&db, "registration_enabled")
            .await
            .expect("should succeed");
        assert!(!val);
    }

    #[tokio::test]
    async fn get_setting_bool_true_for_various_truthy_values() {
        use crate::models::prelude::SystemSetting;
        use crate::models::system_setting;
        use chrono::Utc;
        use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};

        for truthy in &["true", "1", "yes", "TRUE", "YES"] {
            let db = make_db().await;
            let existing = SystemSetting::find_by_id("registration_enabled")
                .one(&db)
                .await
                .expect("query")
                .expect("seeded row must exist");
            let mut active: system_setting::ActiveModel = existing.into_active_model();
            active.value = Set(truthy.to_string());
            active.updated_at = Set(Utc::now());
            active.update(&db).await.expect("update setting");

            let val = get_setting_bool(&db, "registration_enabled")
                .await
                .expect("should succeed");
            assert!(val, "Expected true for value: {}", truthy);
        }
    }
}
