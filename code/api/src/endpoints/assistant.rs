//! AI assistant settings, API key and status (`/api/assistant`).
//!
//! The API key is write-only through this API: responses only ever say
//! whether one is configured.

use axum::{
    extract::{rejection::JsonRejection, State},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::middleware::permissions::{
    AssistantUse, Authenticated, Authorized, Permission, SettingsManage, SettingsView,
};
use crate::models::audit_log::{AuditAction, ResourceType};
use crate::services::assistant::client::{Message, MessagesRequest};
use crate::services::assistant::key_store::normalize_key;
use crate::services::assistant::{budget, record_usage, settings, AssistantSettings, UsageRecord};
use crate::state::AppState;

pub fn assistant_routes(state: AppState) -> Router {
    Router::new()
        .route("/status", get(get_status))
        .route(
            "/settings",
            get(get_assistant_settings).put(put_assistant_settings),
        )
        .route("/key", get(get_key).put(put_key).delete(delete_key))
        .route("/key/test", post(test_key))
        .with_state(state)
}

/// Whether an API key is stored. The key itself is never returned.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct KeyStatus {
    pub configured: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetKeyRequest {
    /// An Anthropic API key (`sk-ant-...`).
    pub api_key: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct KeyTestResult {
    pub ok: bool,
    /// A fixed sentence describing the failure; never provider text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AssistantStatus {
    /// An API key is stored.
    pub configured: bool,
    /// The owner has switched the assistant on.
    pub enabled: bool,
    pub model: crate::services::assistant::AssistantModel,
    pub effort: crate::services::assistant::Effort,
    /// Spend in the current UTC calendar month.
    pub month_spend_usd: f64,
    pub monthly_budget_usd: f64,
    /// The budget is used up; new requests are refused until next month.
    pub paused: bool,
}

/// Whether a key is stored; storage failures read as "not configured".
async fn key_configured(state: &AppState) -> bool {
    match state.assistant.key_store.get().await {
        Ok(key) => key.is_some(),
        Err(e) => {
            tracing::warn!("Could not read the assistant API key: {e}");
            false
        }
    }
}

async fn audit_key_change(state: &AppState, user: &crate::models::user::Model, change: &str) {
    if let Err(e) = state
        .audit
        .log(
            AuditAction::SystemSettingChanged,
            ResourceType::System,
            Some("assistant.api_key".into()),
            Some(user.id),
            Some(user.username.clone()),
            Some(serde_json::json!({ "change": change })),
            None,
            None,
            true,
            None,
        )
        .await
    {
        tracing::warn!("Failed to audit the assistant API key change: {e}");
    }
}

/// Assistant status for the settings page and the chat UI
#[utoipa::path(
    get,
    path = "/api/assistant/status",
    tag = "Assistant",
    responses(
        (status = 200, body = AssistantStatus),
        (status = 403, description = "Requires settings.view or assistant.use")
    )
)]
pub async fn get_status(
    State(state): State<AppState>,
    auth: Authenticated,
) -> Result<Json<AssistantStatus>> {
    let allowed = auth
        .permissions()
        .iter()
        .any(|p| p == SettingsView::NAME || p == AssistantUse::NAME);
    if !allowed {
        return Err(AppError::Forbidden(format!(
            "Permission denied: {} or {} required",
            SettingsView::NAME,
            AssistantUse::NAME
        )));
    }
    let db = state.get_db().await?;
    let settings = settings::load(&db).await?;
    let budget = budget::budget_state(&db, &settings, Utc::now()).await?;
    Ok(Json(AssistantStatus {
        configured: key_configured(&state).await,
        enabled: settings.enabled,
        model: settings.model,
        effort: settings.effort,
        month_spend_usd: budget.spend_usd(),
        monthly_budget_usd: settings.monthly_budget_usd,
        paused: budget.paused(),
    }))
}

/// Get the assistant settings
#[utoipa::path(
    get,
    path = "/api/assistant/settings",
    tag = "Assistant",
    responses((status = 200, body = AssistantSettings))
)]
pub async fn get_assistant_settings(
    State(state): State<AppState>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<AssistantSettings>> {
    let db = state.get_db().await?;
    Ok(Json(settings::load(&db).await?))
}

/// Replace the assistant settings
#[utoipa::path(
    put,
    path = "/api/assistant/settings",
    tag = "Assistant",
    request_body = AssistantSettings,
    responses(
        (status = 200, body = AssistantSettings),
        (status = 400, description = "A field is invalid; the message names it")
    )
)]
pub async fn put_assistant_settings(
    State(state): State<AppState>,
    auth: Authorized<SettingsManage>,
    body: std::result::Result<Json<AssistantSettings>, JsonRejection>,
) -> Result<Json<AssistantSettings>> {
    let Json(new_settings) = body.map_err(|e| AppError::BadRequest(e.body_text()))?;
    let db = state.get_db().await?;
    settings::save(
        &db,
        &new_settings,
        Some((auth.user_id(), auth.user().username.clone())),
    )
    .await?;
    Ok(Json(new_settings))
}

/// Whether an Anthropic API key is configured
#[utoipa::path(
    get,
    path = "/api/assistant/key",
    tag = "Assistant",
    responses((status = 200, body = KeyStatus))
)]
pub async fn get_key(
    State(state): State<AppState>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<KeyStatus>> {
    Ok(Json(KeyStatus {
        configured: key_configured(&state).await,
    }))
}

/// Store or replace the Anthropic API key
#[utoipa::path(
    put,
    path = "/api/assistant/key",
    tag = "Assistant",
    request_body = SetKeyRequest,
    responses(
        (status = 200, body = KeyStatus),
        (status = 400, description = "Not an Anthropic API key")
    )
)]
pub async fn put_key(
    State(state): State<AppState>,
    auth: Authorized<SettingsManage>,
    body: std::result::Result<Json<SetKeyRequest>, JsonRejection>,
) -> Result<Json<KeyStatus>> {
    // Never echo the body: the rejection text could quote it.
    let Json(request) = body.map_err(|_| {
        AppError::BadRequest("Expected a JSON body {\"api_key\": \"sk-ant-...\"}".into())
    })?;
    let key = normalize_key(&request.api_key)?;
    state.assistant.key_store.set(&key).await?;
    audit_key_change(&state, auth.user(), "set").await;
    Ok(Json(KeyStatus { configured: true }))
}

/// Remove the Anthropic API key
#[utoipa::path(
    delete,
    path = "/api/assistant/key",
    tag = "Assistant",
    responses((status = 200, body = KeyStatus))
)]
pub async fn delete_key(
    State(state): State<AppState>,
    auth: Authorized<SettingsManage>,
) -> Result<Json<KeyStatus>> {
    state.assistant.key_store.delete().await?;
    audit_key_change(&state, auth.user(), "removed").await;
    Ok(Json(KeyStatus { configured: false }))
}

/// Check the stored key with a one-token request to the selected model
#[utoipa::path(
    post,
    path = "/api/assistant/key/test",
    tag = "Assistant",
    responses((status = 200, body = KeyTestResult))
)]
pub async fn test_key(
    State(state): State<AppState>,
    auth: Authorized<SettingsManage>,
) -> Result<Json<KeyTestResult>> {
    let db = state.get_db().await?;
    let settings = settings::load(&db).await?;
    let Some(key) = state.assistant.key_store.get().await? else {
        return Ok(Json(KeyTestResult {
            ok: false,
            error: Some("No API key is configured.".into()),
        }));
    };
    let request = MessagesRequest {
        model: settings.model.id().to_string(),
        max_tokens: 1,
        messages: vec![Message::user("ping")],
        system: None,
        output_config: None,
    };
    match state.assistant.client.create(&key, &request).await {
        Ok(response) => {
            let recorded = record_usage(
                &db,
                &state.notification,
                UsageRecord {
                    user_id: Some(auth.user_id()),
                    conversation_id: None,
                    model: settings.model,
                    usage: response.usage,
                },
            )
            .await;
            if let Err(e) = recorded {
                tracing::warn!("Failed to record assistant key test usage: {e}");
            }
            Ok(Json(KeyTestResult {
                ok: true,
                error: None,
            }))
        }
        Err(e) => {
            tracing::info!(error = %e, "Assistant API key test failed");
            Ok(Json(KeyTestResult {
                ok: false,
                error: Some(e.user_message().to_string()),
            }))
        }
    }
}
