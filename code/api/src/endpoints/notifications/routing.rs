//! Admin: which external channels each notification event is delivered to.
//!
//! `GET /api/notifications/routes` and `PUT /api/notifications/routes/{event_type}`.
//! The in-app bell is not routed: it is on whenever the event is enabled.

use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    Json,
};
use sea_orm::TransactionTrait;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::middleware::permissions::{Authorized, SettingsManage, SettingsView};
use crate::models::audit_log::{AuditAction, ResourceType};
use crate::services::notification::{routing, ChannelType};
use crate::state::AppState;

/// Whether an event is delivered over each external channel.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RouteChannels {
    pub email: bool,
    pub telegram: bool,
    pub messagebird: bool,
}

impl RouteChannels {
    fn from_map(map: Option<&HashMap<ChannelType, bool>>) -> Self {
        let on = |channel| map.and_then(|m| m.get(&channel)).copied().unwrap_or(false);
        Self {
            email: on(ChannelType::Email),
            telegram: on(ChannelType::Telegram),
            messagebird: on(ChannelType::MessageBird),
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct EventRouteDto {
    pub event_type: String,
    pub channels: RouteChannels,
}

/// Channels to change; channels left out keep their current route.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateRouteRequest {
    #[schema(value_type = HashMap<String, bool>)]
    pub channels: HashMap<String, bool>,
}

#[utoipa::path(
    get,
    path = "/api/notifications/routes",
    tag = "Notifications",
    responses(
        (status = 200, body = Vec<EventRouteDto>)
    )
)]
pub(crate) async fn list_routes(
    State(state): State<AppState>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<Vec<EventRouteDto>>> {
    let db = state.get_db().await?;
    let routes = routing::all_routes(&db).await?;
    let result = super::get_all_event_types()
        .into_iter()
        .map(|event_type| EventRouteDto {
            channels: RouteChannels::from_map(routes.get(&event_type)),
            event_type,
        })
        .collect();
    Ok(Json(result))
}

#[utoipa::path(
    put,
    path = "/api/notifications/routes/{event_type}",
    tag = "Notifications",
    params(
        ("event_type" = String, Path, description = "Event type"),
    ),
    request_body = UpdateRouteRequest,
    responses(
        (status = 200, body = EventRouteDto),
        (status = 400, description = "Unknown event type or channel")
    )
)]
pub(crate) async fn update_route(
    State(state): State<AppState>,
    auth: Authorized<SettingsManage>,
    Path(event_type): Path<String>,
    Json(req): Json<UpdateRouteRequest>,
) -> Result<Json<EventRouteDto>> {
    if !super::get_all_event_types().contains(&event_type) {
        return Err(AppError::BadRequest(format!(
            "Unknown event type: {}",
            event_type
        )));
    }
    if req.channels.is_empty() {
        return Err(AppError::BadRequest(
            "No channels given to change".to_string(),
        ));
    }
    let mut changes: HashMap<ChannelType, bool> = HashMap::new();
    for (name, enabled) in &req.channels {
        let channel = ChannelType::parse(name)
            .ok_or_else(|| AppError::BadRequest(format!("Invalid channel type: {}", name)))?;
        changes.insert(channel, *enabled);
    }

    let db = state.get_db().await?;
    let txn = db.begin().await?;
    routing::set_routes(&txn, &event_type, &changes).await?;
    // Like other settings, only the setting key is audited: the audit detail
    // allow-list does not carry event or channel names.
    crate::services::audit::log_on_transaction(
        &txn,
        AuditAction::SystemSettingChanged,
        ResourceType::System,
        Some("notification_routes".to_string()),
        Some(auth.user_id()),
        Some(auth.user().username.clone()),
        None,
        None,
        None,
        true,
        None,
    )
    .await?;
    txn.commit().await?;

    let routes = routing::all_routes(&db).await?;
    Ok(Json(EventRouteDto {
        channels: RouteChannels::from_map(routes.get(&event_type)),
        event_type,
    }))
}
