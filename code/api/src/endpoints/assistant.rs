//! AI assistant settings, API key and status (`/api/assistant`).
//!
//! The API key is write-only through this API: responses only ever say
//! whether one is configured.

use axum::{
    body::Body,
    extract::{rejection::JsonRejection, Path, State},
    http::{header, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio_util::io::ReaderStream;

use crate::error::{AppError, Result};
use crate::middleware::permissions::{
    AssistantUse, Authenticated, Authorized, Permission, SettingsManage, SettingsView,
};
use crate::models::audit_log::{AuditAction, ResourceType};
use crate::services::assistant::actions::{self, RunResult};
use crate::services::assistant::chat::{run_turn, EventSink, Turn};
use crate::services::assistant::cluster;
use crate::services::assistant::conversations::{
    self, ChatContext, ConversationDetail, ConversationSummary,
};
use crate::services::assistant::key_store::normalize_key;
use crate::services::assistant::tools::{Monitoring, ToolContext};
use crate::services::assistant::{budget, settings, AssistantSettings};
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
        .route(
            "/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/conversations/{id}",
            get(get_conversation).delete(delete_conversation),
        )
        .route("/conversations/{id}/messages", post(post_message))
        .route("/actions/{id}/run", post(run_action))
        .route("/actions/{id}/cancel", post(cancel_action))
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
    /// The key Secret could not be read (RBAC, cluster unreachable), so
    /// `configured` is unknown and reported as false.
    pub key_error: bool,
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

/// Whether a key is stored, or `Err(())` when the store could not be read.
async fn key_configured(state: &AppState) -> std::result::Result<bool, ()> {
    match state.assistant.key_store.get().await {
        Ok(key) => Ok(key.is_some()),
        Err(e) => {
            tracing::warn!("Could not read the assistant API key: {e}");
            Err(())
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
    let configured = key_configured(&state).await;
    Ok(Json(AssistantStatus {
        configured: configured.unwrap_or(false),
        key_error: configured.is_err(),
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
    let configured = key_configured(&state)
        .await
        .map_err(|()| AppError::ServiceUnavailable("Couldn't read the assistant API key".into()))?;
    Ok(Json(KeyStatus { configured }))
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

/// Check the stored key against the selected model (`GET /v1/models/{model}`, free)
#[utoipa::path(
    post,
    path = "/api/assistant/key/test",
    tag = "Assistant",
    responses((status = 200, body = KeyTestResult))
)]
pub async fn test_key(
    State(state): State<AppState>,
    _auth: Authorized<SettingsManage>,
) -> Result<Json<KeyTestResult>> {
    let db = state.get_db().await?;
    let settings = settings::load(&db).await?;
    let Some(key) = state.assistant.key_store.get().await? else {
        return Ok(Json(KeyTestResult {
            ok: false,
            error: Some("No API key is configured.".into()),
        }));
    };
    match state
        .assistant
        .client
        .check_model(&key, settings.model.id())
        .await
    {
        Ok(()) => Ok(Json(KeyTestResult {
            ok: true,
            error: None,
        })),
        Err(e) => {
            tracing::info!(error = %e, "Assistant API key test failed");
            Ok(Json(KeyTestResult {
                ok: false,
                error: Some(e.user_message().to_string()),
            }))
        }
    }
}

// ============================================================================
// Chat
// ============================================================================

fn require_use(auth: &Authenticated) -> Result<()> {
    if auth.permissions().iter().any(|p| p == AssistantUse::NAME) {
        Ok(())
    } else {
        Err(AppError::Forbidden(format!(
            "Permission denied: {} required",
            AssistantUse::NAME
        )))
    }
}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct CreateConversationRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub context: Option<ChatContext>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PostMessageRequest {
    /// The question.
    pub text: String,
    /// The page and app the question is asked from.
    #[serde(default)]
    pub context: Option<ChatContext>,
}

/// The caller's conversations, most recent first
#[utoipa::path(
    get,
    path = "/api/assistant/conversations",
    tag = "Assistant",
    responses((status = 200, body = Vec<ConversationSummary>))
)]
pub async fn list_conversations(
    State(state): State<AppState>,
    auth: Authenticated,
) -> Result<Json<Vec<ConversationSummary>>> {
    require_use(&auth)?;
    let db = state.get_db().await?;
    Ok(Json(conversations::list(&db, auth.user_id()).await?))
}

/// Start a conversation
#[utoipa::path(
    post,
    path = "/api/assistant/conversations",
    tag = "Assistant",
    request_body = CreateConversationRequest,
    responses((status = 200, body = ConversationSummary))
)]
pub async fn create_conversation(
    State(state): State<AppState>,
    auth: Authenticated,
    body: Option<Json<CreateConversationRequest>>,
) -> Result<Json<ConversationSummary>> {
    require_use(&auth)?;
    let request = body.map(|Json(b)| b).unwrap_or_default();
    let db = state.get_db().await?;
    let conversation = conversations::create(
        &db,
        auth.user_id(),
        request.title.as_deref(),
        &request.context.unwrap_or_default(),
    )
    .await?;
    Ok(Json(conversation.into()))
}

/// One of the caller's conversations with its messages
#[utoipa::path(
    get,
    path = "/api/assistant/conversations/{id}",
    tag = "Assistant",
    params(("id" = String, Path, description = "Conversation id")),
    responses(
        (status = 200, body = ConversationDetail),
        (status = 404, description = "No such conversation of the caller")
    )
)]
pub async fn get_conversation(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Result<Json<ConversationDetail>> {
    require_use(&auth)?;
    let db = state.get_db().await?;
    let conversation = conversations::owned(&db, auth.user_id(), &id).await?;
    Ok(Json(conversations::detail(&db, conversation).await?))
}

/// Delete one of the caller's conversations
#[utoipa::path(
    delete,
    path = "/api/assistant/conversations/{id}",
    tag = "Assistant",
    params(("id" = String, Path, description = "Conversation id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such conversation of the caller")
    )
)]
pub async fn delete_conversation(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    require_use(&auth)?;
    let db = state.get_db().await?;
    let conversation = conversations::owned(&db, auth.user_id(), &id).await?;
    conversations::delete(&db, &conversation.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Ask a question; the answer streams back as server-sent events
///
/// Events: `status` `{text}` while tools run, `text_delta` `{text}`,
/// `action_card` `{id, kind, app, summary, status, expires_at, ...}` when the
/// assistant proposes an action, `sources` `{items}`, then `done`
/// `{message_id}` or `error` `{message}`.
#[utoipa::path(
    post,
    path = "/api/assistant/conversations/{id}/messages",
    tag = "Assistant",
    params(("id" = String, Path, description = "Conversation id")),
    request_body = PostMessageRequest,
    responses(
        (status = 200, description = "Server-sent event stream", content_type = "text/event-stream"),
        (status = 400, description = "Empty or too long question"),
        (status = 404, description = "No such conversation of the caller"),
        (status = 429, description = "Too many questions; try again later"),
        (status = 503, description = "Not set up, switched off, or the monthly budget is used up")
    )
)]
pub async fn post_message(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<String>,
    body: std::result::Result<Json<PostMessageRequest>, JsonRejection>,
) -> Result<Response> {
    require_use(&auth)?;
    let Json(request) =
        body.map_err(|_| AppError::BadRequest("Expected a JSON body {\"text\": \"...\"}".into()))?;
    let question = request.text.trim().to_string();
    if question.is_empty() {
        return Err(AppError::BadRequest("text: ask a question".into()));
    }
    if question.chars().count() > conversations::MAX_QUESTION_CHARS {
        return Err(AppError::BadRequest(format!(
            "text: at most {} characters",
            conversations::MAX_QUESTION_CHARS
        )));
    }
    let db = state.get_db().await?;
    let conversation = conversations::owned(&db, auth.user_id(), &id).await?;

    let settings = settings::load(&db).await?;
    if !settings.enabled {
        return Err(AppError::ServiceUnavailable(
            "The assistant is switched off.".into(),
        ));
    }
    let api_key = match state.assistant.key_store.get().await {
        Ok(Some(key)) => key,
        Ok(None) => {
            return Err(AppError::ServiceUnavailable(
                "The assistant is not set up: no Anthropic API key is configured.".into(),
            ))
        }
        Err(e) => {
            tracing::warn!("Could not read the assistant API key: {e}");
            return Err(AppError::ServiceUnavailable(
                "The assistant's API key could not be read.".into(),
            ));
        }
    };
    budget::ensure_budget(&db).await?;
    conversations::take_rate_slot(&db, auth.user_id()).await?;

    let stored: ChatContext = conversation
        .context_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
    let context = request
        .context
        .unwrap_or_default()
        .sanitized()
        .or(&stored.sanitized());
    let (question_id, answer_id) = conversations::start_turn(&db, &conversation, &question).await?;
    let mut ctx = ToolContext {
        state: state.clone(),
        db: db.clone(),
        user_id: auth.user_id(),
        username: auth.user().username.clone(),
        permissions: auth.permissions().to_vec(),
        conversation_id: conversation.id.clone(),
        monitoring: Monitoring::detect(&db).await,
        cluster: false,
        message_id: Some(answer_id),
    };
    ctx.cluster = cluster::available(&ctx).await;
    let turn = Turn {
        ctx,
        conversation,
        question,
        question_id,
        answer_id,
        context,
        settings,
        api_key,
    };

    let (writer, reader) = tokio::io::duplex(16 * 1024);
    tokio::spawn(async move {
        let mut sink = EventSink::new(writer);
        run_turn(turn, &mut sink).await;
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(ReaderStream::new(reader)))
        .map_err(|e| AppError::Internal(format!("Could not start the stream: {e}")))
}

/// Run an action the assistant proposed (the caller's own, once, before it expires)
///
/// This is the only way a proposed action is carried out. Permissions and
/// the app rules are checked again now; the work goes through Kubarr's
/// operation queue. The outcome is added to the conversation.
#[utoipa::path(
    post,
    path = "/api/assistant/actions/{id}/run",
    tag = "Assistant",
    params(("id" = String, Path, description = "Action id")),
    responses(
        (status = 200, body = RunResult),
        (status = 403, description = "No longer permitted (the action is marked failed)"),
        (status = 404, description = "No such action of the caller"),
        (status = 409, description = "Already run or cancelled, expired, or the app changed")
    )
)]
pub async fn run_action(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Result<Json<RunResult>> {
    require_use(&auth)?;
    let db = state.get_db().await?;
    let result = actions::run(&state, &db, auth.user_id(), &auth.user().username, &id).await?;
    Ok(Json(result))
}

/// Cancel an action the assistant proposed (the caller's own, while pending)
#[utoipa::path(
    post,
    path = "/api/assistant/actions/{id}/cancel",
    tag = "Assistant",
    params(("id" = String, Path, description = "Action id")),
    responses(
        (status = 204, description = "Cancelled"),
        (status = 404, description = "No such action of the caller"),
        (status = 409, description = "Already run, cancelled or expired")
    )
)]
pub async fn cancel_action(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    require_use(&auth)?;
    let db = state.get_db().await?;
    actions::cancel(&state, &db, auth.user_id(), &auth.user().username, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
