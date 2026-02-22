use axum::{
    extract::{Path, Query, State},
    routing::{delete, get, post, put},
    Json, Router,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::middleware::permissions::{
    AuditView, Authenticated, Authorized, SettingsManage, SettingsView,
};
use crate::models::{
    notification_channel, notification_event, notification_log, user_notification_pref,
};
use crate::services::notification::ChannelType;
use crate::state::AppState;

pub fn notifications_routes(state: AppState) -> Router {
    Router::new()
        // User inbox endpoints
        .route("/inbox", get(get_inbox))
        .route("/inbox/count", get(get_unread_count))
        .route("/inbox/{id}/read", post(mark_as_read))
        .route("/inbox/read-all", post(mark_all_as_read))
        .route("/inbox/{id}", delete(delete_notification))
        // Admin: Channel configuration
        .route("/channels", get(list_channels))
        .route("/channels/{channel_type}", get(get_channel))
        .route("/channels/{channel_type}", put(update_channel))
        .route("/channels/{channel_type}/test", post(test_channel))
        // Admin: Event settings
        .route("/events", get(list_events))
        .route("/events/{event_type}", put(update_event))
        // User preferences
        .route("/preferences", get(get_preferences))
        .route("/preferences/{channel_type}", put(update_preference))
        // Admin: Logs
        .route("/logs", get(list_logs))
        .with_state(state)
}

// ============================================================================
// User Inbox Endpoints
// ============================================================================

#[derive(Serialize, utoipa::ToSchema)]
pub struct InboxResponse {
    pub notifications: Vec<NotificationDto>,
    pub total: u64,
    pub unread: u64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct NotificationDto {
    pub id: i64,
    pub title: String,
    pub message: String,
    pub event_type: Option<String>,
    pub severity: String,
    pub read: bool,
    pub created_at: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct InboxQuery {
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/api/notifications/inbox",
    tag = "Notifications",
    params(
        ("limit" = Option<u64>, Query, description = "Number of notifications to return"),
        ("offset" = Option<u64>, Query, description = "Offset for pagination"),
    ),
    responses(
        (status = 200, body = InboxResponse)
    )
)]
async fn get_inbox(
    State(state): State<AppState>,
    auth: Authenticated,
    Query(query): Query<InboxQuery>,
) -> Result<Json<InboxResponse>> {
    let db = state.get_db().await?;
    let limit = query.limit.unwrap_or(20).min(100);
    let offset = query.offset.unwrap_or(0);

    let notifications = state
        .notification
        .get_user_notifications(auth.user_id(), limit, offset)
        .await?;

    let unread = state.notification.get_unread_count(auth.user_id()).await?;

    let total = crate::models::user_notification::Entity::find()
        .filter(crate::models::user_notification::Column::UserId.eq(auth.user_id()))
        .count(&db)
        .await?;

    let dtos: Vec<NotificationDto> = notifications
        .into_iter()
        .map(|n| NotificationDto {
            id: n.id,
            title: n.title,
            message: n.message,
            event_type: n.event_type,
            severity: n.severity,
            read: n.read,
            created_at: n.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(InboxResponse {
        notifications: dtos,
        total,
        unread,
    }))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct UnreadCountResponse {
    pub count: u64,
}

#[utoipa::path(
    get,
    path = "/api/notifications/inbox/count",
    tag = "Notifications",
    responses(
        (status = 200, body = UnreadCountResponse)
    )
)]
async fn get_unread_count(
    State(state): State<AppState>,
    auth: Authenticated,
) -> Result<Json<UnreadCountResponse>> {
    let count = state.notification.get_unread_count(auth.user_id()).await?;
    Ok(Json(UnreadCountResponse { count }))
}

#[utoipa::path(
    post,
    path = "/api/notifications/inbox/{id}/read",
    tag = "Notifications",
    params(
        ("id" = i64, Path, description = "Notification ID"),
    ),
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn mark_as_read(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>> {
    state.notification.mark_as_read(id, auth.user_id()).await?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    post,
    path = "/api/notifications/inbox/read-all",
    tag = "Notifications",
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn mark_all_as_read(
    State(state): State<AppState>,
    auth: Authenticated,
) -> Result<Json<serde_json::Value>> {
    state.notification.mark_all_as_read(auth.user_id()).await?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[utoipa::path(
    delete,
    path = "/api/notifications/inbox/{id}",
    tag = "Notifications",
    params(
        ("id" = i64, Path, description = "Notification ID"),
    ),
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn delete_notification(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>> {
    state
        .notification
        .delete_notification(id, auth.user_id())
        .await?;
    Ok(Json(serde_json::json!({ "success": true })))
}

// ============================================================================
// Admin: Channel Configuration
// ============================================================================

#[derive(Serialize, utoipa::ToSchema)]
pub struct ChannelDto {
    pub channel_type: String,
    pub enabled: bool,
    pub config: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

#[utoipa::path(
    get,
    path = "/api/notifications/channels",
    tag = "Notifications",
    responses(
        (status = 200, body = Vec<ChannelDto>)
    )
)]
async fn list_channels(
    State(state): State<AppState>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<Vec<ChannelDto>>> {
    let db = state.get_db().await?;
    let channels = notification_channel::Entity::find()
        .order_by_asc(notification_channel::Column::ChannelType)
        .all(&db)
        .await?;

    // Return all channel types, with defaults for unconfigured ones
    let mut result: Vec<ChannelDto> = Vec::new();

    for channel_type in ChannelType::all() {
        let existing = channels
            .iter()
            .find(|c| c.channel_type == channel_type.as_str());

        if let Some(channel) = existing {
            let config: serde_json::Value =
                serde_json::from_str(&channel.config).unwrap_or(serde_json::json!({}));
            // Mask sensitive fields
            let masked_config = mask_sensitive_config(&config);

            result.push(ChannelDto {
                channel_type: channel.channel_type.clone(),
                enabled: channel.enabled,
                config: masked_config,
                created_at: channel.created_at.to_rfc3339(),
                updated_at: channel.updated_at.to_rfc3339(),
            });
        } else {
            result.push(ChannelDto {
                channel_type: channel_type.as_str().to_string(),
                enabled: false,
                config: serde_json::json!({}),
                created_at: "".to_string(),
                updated_at: "".to_string(),
            });
        }
    }

    Ok(Json(result))
}

#[utoipa::path(
    get,
    path = "/api/notifications/channels/{channel_type}",
    tag = "Notifications",
    params(
        ("channel_type" = String, Path, description = "Channel type"),
    ),
    responses(
        (status = 200, body = ChannelDto)
    )
)]
async fn get_channel(
    State(state): State<AppState>,
    _auth: Authorized<SettingsView>,
    Path(channel_type): Path<String>,
) -> Result<Json<ChannelDto>> {
    let db = state.get_db().await?;
    let channel = notification_channel::Entity::find()
        .filter(notification_channel::Column::ChannelType.eq(&channel_type))
        .one(&db)
        .await?;

    match channel {
        Some(ch) => {
            let config: serde_json::Value =
                serde_json::from_str(&ch.config).unwrap_or(serde_json::json!({}));
            let masked_config = mask_sensitive_config(&config);

            Ok(Json(ChannelDto {
                channel_type: ch.channel_type,
                enabled: ch.enabled,
                config: masked_config,
                created_at: ch.created_at.to_rfc3339(),
                updated_at: ch.updated_at.to_rfc3339(),
            }))
        }
        None => Ok(Json(ChannelDto {
            channel_type,
            enabled: false,
            config: serde_json::json!({}),
            created_at: "".to_string(),
            updated_at: "".to_string(),
        })),
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateChannelRequest {
    pub enabled: Option<bool>,
    pub config: Option<serde_json::Value>,
}

#[utoipa::path(
    put,
    path = "/api/notifications/channels/{channel_type}",
    tag = "Notifications",
    params(
        ("channel_type" = String, Path, description = "Channel type"),
    ),
    request_body = UpdateChannelRequest,
    responses(
        (status = 200, body = ChannelDto)
    )
)]
async fn update_channel(
    State(state): State<AppState>,
    _auth: Authorized<SettingsManage>,
    Path(channel_type): Path<String>,
    Json(req): Json<UpdateChannelRequest>,
) -> Result<Json<ChannelDto>> {
    let db = state.get_db().await?;
    // Validate channel type
    if ChannelType::parse(&channel_type).is_none() {
        return Err(AppError::BadRequest(format!(
            "Invalid channel type: {}",
            channel_type
        )));
    }

    let now = chrono::Utc::now();

    let existing = notification_channel::Entity::find()
        .filter(notification_channel::Column::ChannelType.eq(&channel_type))
        .one(&db)
        .await?;

    let channel = if let Some(existing) = existing {
        let mut active: notification_channel::ActiveModel = existing.into();

        if let Some(enabled) = req.enabled {
            active.enabled = Set(enabled);
        }
        if let Some(config) = req.config {
            active.config = Set(serde_json::to_string(&config).unwrap_or_default());
        }
        active.updated_at = Set(now);

        active.update(&db).await?
    } else {
        let config = req.config.unwrap_or(serde_json::json!({}));
        let new_channel = notification_channel::ActiveModel {
            channel_type: Set(channel_type.clone()),
            enabled: Set(req.enabled.unwrap_or(false)),
            config: Set(serde_json::to_string(&config).unwrap_or_default()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        new_channel.insert(&db).await?
    };

    // Reinitialize providers
    if let Err(e) = state.notification.init_providers().await {
        tracing::warn!("Failed to reinitialize notification providers: {}", e);
    }

    let config: serde_json::Value =
        serde_json::from_str(&channel.config).unwrap_or(serde_json::json!({}));
    let masked_config = mask_sensitive_config(&config);

    Ok(Json(ChannelDto {
        channel_type: channel.channel_type,
        enabled: channel.enabled,
        config: masked_config,
        created_at: channel.created_at.to_rfc3339(),
        updated_at: channel.updated_at.to_rfc3339(),
    }))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct TestChannelRequest {
    pub destination: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct TestChannelResponse {
    pub success: bool,
    pub error: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/notifications/channels/{channel_type}/test",
    tag = "Notifications",
    params(
        ("channel_type" = String, Path, description = "Channel type"),
    ),
    request_body = TestChannelRequest,
    responses(
        (status = 200, body = TestChannelResponse)
    )
)]
async fn test_channel(
    State(state): State<AppState>,
    _auth: Authorized<SettingsManage>,
    Path(channel_type): Path<String>,
    Json(req): Json<TestChannelRequest>,
) -> Result<Json<TestChannelResponse>> {
    let result = state
        .notification
        .test_channel(&channel_type, &req.destination)
        .await;

    Ok(Json(TestChannelResponse {
        success: result.success,
        error: result.error,
    }))
}

// ============================================================================
// Admin: Event Settings
// ============================================================================

#[derive(Serialize, utoipa::ToSchema)]
pub struct EventSettingDto {
    pub event_type: String,
    pub enabled: bool,
    pub severity: String,
}

#[utoipa::path(
    get,
    path = "/api/notifications/events",
    tag = "Notifications",
    responses(
        (status = 200, body = Vec<EventSettingDto>)
    )
)]
async fn list_events(
    State(state): State<AppState>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<Vec<EventSettingDto>>> {
    let db = state.get_db().await?;
    let events = notification_event::Entity::find()
        .order_by_asc(notification_event::Column::EventType)
        .all(&db)
        .await?;

    // Get all possible event types from AuditAction
    let all_event_types = get_all_event_types();

    let mut result: Vec<EventSettingDto> = Vec::new();

    for event_type in all_event_types {
        let existing = events.iter().find(|e| e.event_type == event_type);

        if let Some(event) = existing {
            result.push(EventSettingDto {
                event_type: event.event_type.clone(),
                enabled: event.enabled,
                severity: event.severity.clone(),
            });
        } else {
            result.push(EventSettingDto {
                event_type,
                enabled: false,
                severity: "info".to_string(),
            });
        }
    }

    Ok(Json(result))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateEventRequest {
    pub enabled: Option<bool>,
    pub severity: Option<String>,
}

#[utoipa::path(
    put,
    path = "/api/notifications/events/{event_type}",
    tag = "Notifications",
    params(
        ("event_type" = String, Path, description = "Event type"),
    ),
    request_body = UpdateEventRequest,
    responses(
        (status = 200, body = EventSettingDto)
    )
)]
async fn update_event(
    State(state): State<AppState>,
    _auth: Authorized<SettingsManage>,
    Path(event_type): Path<String>,
    Json(req): Json<UpdateEventRequest>,
) -> Result<Json<EventSettingDto>> {
    let db = state.get_db().await?;
    let existing = notification_event::Entity::find()
        .filter(notification_event::Column::EventType.eq(&event_type))
        .one(&db)
        .await?;

    let event = if let Some(existing) = existing {
        let mut active: notification_event::ActiveModel = existing.into();

        if let Some(enabled) = req.enabled {
            active.enabled = Set(enabled);
        }
        if let Some(severity) = req.severity {
            active.severity = Set(severity);
        }

        active.update(&db).await?
    } else {
        let new_event = notification_event::ActiveModel {
            event_type: Set(event_type.clone()),
            enabled: Set(req.enabled.unwrap_or(false)),
            severity: Set(req.severity.unwrap_or_else(|| "info".to_string())),
            ..Default::default()
        };
        new_event.insert(&db).await?
    };

    Ok(Json(EventSettingDto {
        event_type: event.event_type,
        enabled: event.enabled,
        severity: event.severity,
    }))
}

// ============================================================================
// User Preferences
// ============================================================================

#[derive(Serialize, utoipa::ToSchema)]
pub struct UserPrefDto {
    pub channel_type: String,
    pub enabled: bool,
    pub destination: Option<String>,
    pub verified: bool,
}

#[utoipa::path(
    get,
    path = "/api/notifications/preferences",
    tag = "Notifications",
    responses(
        (status = 200, body = Vec<UserPrefDto>)
    )
)]
async fn get_preferences(
    State(state): State<AppState>,
    auth: Authenticated,
) -> Result<Json<Vec<UserPrefDto>>> {
    let db = state.get_db().await?;
    let prefs = user_notification_pref::Entity::find()
        .filter(user_notification_pref::Column::UserId.eq(auth.user_id()))
        .all(&db)
        .await?;

    let mut result: Vec<UserPrefDto> = Vec::new();

    for channel_type in ChannelType::all() {
        let existing = prefs
            .iter()
            .find(|p| p.channel_type == channel_type.as_str());

        if let Some(pref) = existing {
            result.push(UserPrefDto {
                channel_type: pref.channel_type.clone(),
                enabled: pref.enabled,
                destination: pref.destination.clone().map(|d| mask_destination(&d)),
                verified: pref.verified,
            });
        } else {
            result.push(UserPrefDto {
                channel_type: channel_type.as_str().to_string(),
                enabled: false,
                destination: None,
                verified: false,
            });
        }
    }

    Ok(Json(result))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdatePrefRequest {
    pub enabled: Option<bool>,
    pub destination: Option<String>,
}

#[utoipa::path(
    put,
    path = "/api/notifications/preferences/{channel_type}",
    tag = "Notifications",
    params(
        ("channel_type" = String, Path, description = "Channel type"),
    ),
    request_body = UpdatePrefRequest,
    responses(
        (status = 200, body = UserPrefDto)
    )
)]
async fn update_preference(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(channel_type): Path<String>,
    Json(req): Json<UpdatePrefRequest>,
) -> Result<Json<UserPrefDto>> {
    let db = state.get_db().await?;
    if ChannelType::parse(&channel_type).is_none() {
        return Err(AppError::BadRequest(format!(
            "Invalid channel type: {}",
            channel_type
        )));
    }

    let now = chrono::Utc::now();

    let existing = user_notification_pref::Entity::find()
        .filter(user_notification_pref::Column::UserId.eq(auth.user_id()))
        .filter(user_notification_pref::Column::ChannelType.eq(&channel_type))
        .one(&db)
        .await?;

    let pref = if let Some(existing) = existing {
        let mut active: user_notification_pref::ActiveModel = existing.clone().into();

        if let Some(enabled) = req.enabled {
            active.enabled = Set(enabled);
        }
        if let Some(destination) = req.destination {
            // If destination changed, reset verification
            if Some(&destination) != existing.destination.as_ref() {
                active.destination = Set(Some(destination));
                active.verified = Set(false);
            }
        }
        active.updated_at = Set(now);

        active.update(&db).await?
    } else {
        let new_pref = user_notification_pref::ActiveModel {
            user_id: Set(auth.user_id()),
            channel_type: Set(channel_type.clone()),
            enabled: Set(req.enabled.unwrap_or(false)),
            destination: Set(req.destination),
            verified: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        new_pref.insert(&db).await?
    };

    Ok(Json(UserPrefDto {
        channel_type: pref.channel_type,
        enabled: pref.enabled,
        destination: pref.destination.map(|d| mask_destination(&d)),
        verified: pref.verified,
    }))
}

// ============================================================================
// Admin: Logs
// ============================================================================

#[derive(Deserialize, utoipa::ToSchema)]
pub struct LogsQuery {
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub channel_type: Option<String>,
    pub status: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct LogDto {
    pub id: i64,
    pub user_id: Option<i64>,
    pub channel_type: String,
    pub event_type: String,
    pub recipient: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct LogsResponse {
    pub logs: Vec<LogDto>,
    pub total: u64,
}

#[utoipa::path(
    get,
    path = "/api/notifications/logs",
    tag = "Notifications",
    params(
        ("limit" = Option<u64>, Query, description = "Number of logs to return"),
        ("offset" = Option<u64>, Query, description = "Offset for pagination"),
        ("channel_type" = Option<String>, Query, description = "Filter by channel type"),
        ("status" = Option<String>, Query, description = "Filter by status"),
    ),
    responses(
        (status = 200, body = LogsResponse)
    )
)]
async fn list_logs(
    State(state): State<AppState>,
    _auth: Authorized<AuditView>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<LogsResponse>> {
    let db = state.get_db().await?;
    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0);

    let mut q = notification_log::Entity::find();

    if let Some(channel) = &query.channel_type {
        q = q.filter(notification_log::Column::ChannelType.eq(channel));
    }
    if let Some(status) = &query.status {
        q = q.filter(notification_log::Column::Status.eq(status));
    }

    let total = q.clone().count(&db).await?;

    let logs = q
        .order_by_desc(notification_log::Column::CreatedAt)
        .offset(offset)
        .limit(limit)
        .all(&db)
        .await?;

    let dtos: Vec<LogDto> = logs
        .into_iter()
        .map(|l| LogDto {
            id: l.id,
            user_id: l.user_id,
            channel_type: l.channel_type,
            event_type: l.event_type,
            recipient: l.recipient,
            status: l.status,
            error_message: l.error_message,
            created_at: l.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(LogsResponse { logs: dtos, total }))
}

// ============================================================================
// Helper Functions
// ============================================================================

fn mask_sensitive_config(config: &serde_json::Value) -> serde_json::Value {
    let mut masked = config.clone();

    if let Some(obj) = masked.as_object_mut() {
        for (key, value) in obj.iter_mut() {
            let key_lower = key.to_lowercase();
            let is_sensitive = key_lower.contains("password")
                || key_lower.contains("secret")
                || key_lower.contains("token")
                || key_lower.contains("api_key");
            if is_sensitive && value.is_string() && !value.as_str().unwrap_or("").is_empty() {
                *value = serde_json::Value::String("********".to_string());
            }
        }
    }

    masked
}

fn mask_destination(dest: &str) -> String {
    if dest.contains('@') {
        // Email
        let parts: Vec<&str> = dest.split('@').collect();
        if parts.len() == 2 && parts[0].len() > 2 {
            format!("{}***@{}", &parts[0][..2], parts[1])
        } else {
            "***@***".to_string()
        }
    } else if dest.starts_with('+') {
        // Phone
        if dest.len() > 4 {
            format!("{}***{}", &dest[..3], &dest[dest.len() - 2..])
        } else {
            "+***".to_string()
        }
    } else if dest.len() > 5 {
        // Other (chat ID)
        format!("{}***{}", &dest[..3], &dest[dest.len() - 2..])
    } else {
        "***".to_string()
    }
}

fn get_all_event_types() -> Vec<String> {
    use crate::models::audit_log::AuditAction;

    vec![
        AuditAction::Login.to_string(),
        AuditAction::LoginFailed.to_string(),
        AuditAction::Logout.to_string(),
        AuditAction::UserCreated.to_string(),
        AuditAction::UserUpdated.to_string(),
        AuditAction::UserDeleted.to_string(),
        AuditAction::UserApproved.to_string(),
        AuditAction::UserDeactivated.to_string(),
        AuditAction::RoleCreated.to_string(),
        AuditAction::RoleUpdated.to_string(),
        AuditAction::RoleDeleted.to_string(),
        AuditAction::RoleAssigned.to_string(),
        AuditAction::RoleUnassigned.to_string(),
        AuditAction::AppInstalled.to_string(),
        AuditAction::AppUninstalled.to_string(),
        AuditAction::AppRestarted.to_string(),
        AuditAction::AppAccessed.to_string(),
        AuditAction::TwoFactorEnabled.to_string(),
        AuditAction::TwoFactorDisabled.to_string(),
        AuditAction::PasswordChanged.to_string(),
        AuditAction::SystemSettingChanged.to_string(),
        AuditAction::InviteCreated.to_string(),
        AuditAction::InviteUsed.to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -------------------------------------------------------------------------
    // mask_sensitive_config
    // -------------------------------------------------------------------------

    #[test]
    fn mask_sensitive_config_masks_password() {
        let config = json!({ "password": "secret123", "host": "localhost" });
        let masked = mask_sensitive_config(&config);
        assert_eq!(masked["password"], "********");
        assert_eq!(masked["host"], "localhost");
    }

    #[test]
    fn mask_sensitive_config_masks_token() {
        let config = json!({ "api_token": "tok_abc", "port": 5432 });
        let masked = mask_sensitive_config(&config);
        assert_eq!(masked["api_token"], "********");
        assert_eq!(masked["port"], 5432);
    }

    #[test]
    fn mask_sensitive_config_masks_secret() {
        let config = json!({ "client_secret": "s3cr3t", "name": "app" });
        let masked = mask_sensitive_config(&config);
        assert_eq!(masked["client_secret"], "********");
        assert_eq!(masked["name"], "app");
    }

    #[test]
    fn mask_sensitive_config_masks_api_key() {
        let config = json!({ "api_key": "key-xyz", "region": "us-east-1" });
        let masked = mask_sensitive_config(&config);
        assert_eq!(masked["api_key"], "********");
        assert_eq!(masked["region"], "us-east-1");
    }

    #[test]
    fn mask_sensitive_config_does_not_mask_empty_string() {
        let config = json!({ "password": "" });
        let masked = mask_sensitive_config(&config);
        // empty string is not masked (guarded by !value.as_str().unwrap_or("").is_empty())
        assert_eq!(masked["password"], "");
    }

    #[test]
    fn mask_sensitive_config_non_object_passthrough() {
        let config = json!([1, 2, 3]);
        let masked = mask_sensitive_config(&config);
        assert_eq!(masked, json!([1, 2, 3]));
    }

    #[test]
    fn mask_sensitive_config_non_string_sensitive_field_unchanged() {
        // numeric value under sensitive key — should NOT be masked (only strings)
        let config = json!({ "api_key": 12345 });
        let masked = mask_sensitive_config(&config);
        assert_eq!(masked["api_key"], 12345);
    }

    // -------------------------------------------------------------------------
    // mask_destination
    // -------------------------------------------------------------------------

    #[test]
    fn mask_destination_email_normal() {
        assert_eq!(mask_destination("alice@example.com"), "al***@example.com");
    }

    #[test]
    fn mask_destination_email_short_local() {
        // local part <= 2 chars → "***@***"
        assert_eq!(mask_destination("ab@example.com"), "***@***");
        assert_eq!(mask_destination("a@b.com"), "***@***");
    }

    #[test]
    fn mask_destination_phone_normal() {
        // starts with '+', len > 4 → "+12***90"
        assert_eq!(mask_destination("+1234567890"), "+12***90");
    }

    #[test]
    fn mask_destination_phone_short() {
        // starts with '+', len <= 4 → "+***"
        assert_eq!(mask_destination("+12"), "+***");
    }

    #[test]
    fn mask_destination_chat_id_long() {
        // len > 5 → first 3 + *** + last 2
        assert_eq!(mask_destination("chat1234"), "cha***34");
    }

    #[test]
    fn mask_destination_chat_id_short() {
        // len <= 5 → "***"
        assert_eq!(mask_destination("chat"), "***");
        assert_eq!(mask_destination("abc"), "***");
    }

    // -------------------------------------------------------------------------
    // get_all_event_types
    // -------------------------------------------------------------------------

    #[test]
    fn get_all_event_types_returns_23_entries() {
        let types = get_all_event_types();
        assert_eq!(types.len(), 23);
    }

    #[test]
    fn get_all_event_types_contains_expected_values() {
        let types = get_all_event_types();
        assert!(types.iter().any(|t| t == "login"));
        assert!(types.iter().any(|t| t == "logout"));
        assert!(types.iter().any(|t| t == "login_failed"));
        assert!(types.iter().any(|t| t == "user_created"));
        assert!(types.iter().any(|t| t == "app_installed"));
        assert!(types.iter().any(|t| t == "invite_used"));
    }

    #[test]
    fn get_all_event_types_no_duplicates() {
        let types = get_all_event_types();
        let mut deduped = types.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(types.len(), deduped.len(), "duplicate event types found");
    }

    // -------------------------------------------------------------------------
    // Struct serde tests
    // -------------------------------------------------------------------------

    #[test]
    fn notification_dto_ser() {
        let dto = NotificationDto {
            id: 1,
            title: "Test".to_string(),
            message: "A message".to_string(),
            event_type: Some("login".to_string()),
            severity: "info".to_string(),
            read: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&dto).expect("ser");
        assert!(json.contains("\"title\":\"Test\""));
        assert!(json.contains("\"read\":false"));
    }

    #[test]
    fn inbox_response_ser() {
        let r = InboxResponse {
            notifications: vec![],
            total: 10,
            unread: 3,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"total\":10"));
        assert!(json.contains("\"unread\":3"));
    }

    #[test]
    fn inbox_query_deser() {
        let q: InboxQuery = serde_json::from_str(r#"{"limit":20,"offset":0}"#).expect("deser");
        assert_eq!(q.limit, Some(20));
        assert_eq!(q.offset, Some(0));
    }

    #[test]
    fn unread_count_response_ser() {
        let r = UnreadCountResponse { count: 5 };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"count\":5"));
    }

    #[test]
    fn channel_dto_ser() {
        let dto = ChannelDto {
            channel_type: "email".to_string(),
            enabled: true,
            config: serde_json::json!({"host": "smtp.example.com"}),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&dto).expect("ser");
        assert!(json.contains("\"channel_type\":\"email\""));
        assert!(json.contains("\"enabled\":true"));
    }

    #[test]
    fn update_channel_request_deser() {
        let r: UpdateChannelRequest =
            serde_json::from_str(r#"{"enabled":true,"config":{"host":"smtp.example.com"}}"#)
                .expect("deser");
        assert_eq!(r.enabled, Some(true));
        assert!(r.config.is_some());
    }

    #[test]
    fn test_channel_request_deser() {
        let r: TestChannelRequest =
            serde_json::from_str(r#"{"destination":"test@example.com"}"#).expect("deser");
        assert_eq!(r.destination, "test@example.com");
    }

    #[test]
    fn test_channel_response_ser() {
        let r = TestChannelResponse {
            success: true,
            error: None,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"success\":true"));
    }

    #[test]
    fn logs_query_deser() {
        let q: LogsQuery = serde_json::from_str(r#"{"limit":50,"offset":10}"#).expect("deser");
        assert_eq!(q.limit, Some(50));
        assert_eq!(q.offset, Some(10));
    }

    #[test]
    fn logs_response_ser() {
        let r = LogsResponse {
            logs: vec![],
            total: 100,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"total\":100"));
    }
}
