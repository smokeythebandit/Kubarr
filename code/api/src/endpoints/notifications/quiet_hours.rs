//! The signed-in user's own quiet hours.
//!
//! `GET /api/notifications/quiet-hours` and `PUT /api/notifications/quiet-hours`.
//! Any authenticated user; each user only ever sees and changes their own row.

use axum::{extract::State, Json};
use sea_orm::sea_query::OnConflict;
use sea_orm::{EntityTrait, Set, TransactionTrait};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::middleware::permissions::Authenticated;
use crate::models::user_quiet_hours;
use crate::services::notification::quiet_hours::{self, QuietWindow};
use crate::state::AppState;

/// Shown before a user saves anything: 23:00–07:00, off.
const DEFAULT_FROM_MINUTE: i32 = 23 * 60;
const DEFAULT_TO_MINUTE: i32 = 7 * 60;
const DEFAULT_TIMEZONE: &str = "UTC";

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct QuietHoursDto {
    pub enabled: bool,
    /// Minutes after local midnight, 0–1439.
    pub from_minute: i32,
    /// Minutes after local midnight, 0–1439; may be before `from_minute`
    /// (the window crosses midnight).
    pub to_minute: i32,
    /// IANA time zone, e.g. `Europe/Amsterdam`.
    pub timezone: String,
    /// Critical notifications are sent straight away during quiet hours.
    pub critical_bypass: bool,
    /// False until the user saves quiet hours; the values are then defaults.
    pub configured: bool,
}

impl From<user_quiet_hours::Model> for QuietHoursDto {
    fn from(row: user_quiet_hours::Model) -> Self {
        Self {
            enabled: row.enabled,
            from_minute: row.from_minute,
            to_minute: row.to_minute,
            timezone: row.timezone,
            critical_bypass: row.critical_bypass,
            configured: true,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateQuietHoursRequest {
    pub enabled: bool,
    pub from_minute: i32,
    pub to_minute: i32,
    pub timezone: String,
    pub critical_bypass: bool,
}

#[utoipa::path(
    get,
    path = "/api/notifications/quiet-hours",
    tag = "Notifications",
    responses(
        (status = 200, body = QuietHoursDto)
    )
)]
pub(crate) async fn get_quiet_hours(
    State(state): State<AppState>,
    auth: Authenticated,
) -> Result<Json<QuietHoursDto>> {
    let db = state.get_db().await?;
    let row = user_quiet_hours::Entity::find_by_id(auth.user_id())
        .one(&db)
        .await?;
    Ok(Json(row.map(QuietHoursDto::from).unwrap_or(
        QuietHoursDto {
            enabled: false,
            from_minute: DEFAULT_FROM_MINUTE,
            to_minute: DEFAULT_TO_MINUTE,
            timezone: DEFAULT_TIMEZONE.to_string(),
            critical_bypass: true,
            configured: false,
        },
    )))
}

#[utoipa::path(
    put,
    path = "/api/notifications/quiet-hours",
    tag = "Notifications",
    request_body = UpdateQuietHoursRequest,
    responses(
        (status = 200, body = QuietHoursDto),
        (status = 400, description = "Invalid times or unknown time zone")
    )
)]
pub(crate) async fn update_quiet_hours(
    State(state): State<AppState>,
    auth: Authenticated,
    Json(req): Json<UpdateQuietHoursRequest>,
) -> Result<Json<QuietHoursDto>> {
    // Validated even when disabled, so a stored row is always usable.
    let window = QuietWindow::new(req.from_minute, req.to_minute, &req.timezone)?;
    let now = chrono::Utc::now();
    let row = user_quiet_hours::ActiveModel {
        user_id: Set(auth.user_id()),
        enabled: Set(req.enabled),
        from_minute: Set(req.from_minute),
        to_minute: Set(req.to_minute),
        timezone: Set(window.timezone_name().to_string()),
        critical_bypass: Set(req.critical_bypass),
        updated_at: Set(now),
    };

    let db = state.get_db().await?;
    let txn = db.begin().await?;
    user_quiet_hours::Entity::insert(row)
        .on_conflict(
            OnConflict::column(user_quiet_hours::Column::UserId)
                .update_columns([
                    user_quiet_hours::Column::Enabled,
                    user_quiet_hours::Column::FromMinute,
                    user_quiet_hours::Column::ToMinute,
                    user_quiet_hours::Column::Timezone,
                    user_quiet_hours::Column::CriticalBypass,
                    user_quiet_hours::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec_without_returning(&txn)
        .await?;
    // Messages already held follow the new setting.
    quiet_hours::reschedule_held(&txn, auth.user_id(), req.enabled.then_some(window), now).await?;
    txn.commit().await?;

    let saved = user_quiet_hours::Entity::find_by_id(auth.user_id())
        .one(&db)
        .await?
        .ok_or_else(|| crate::error::AppError::Internal("Quiet hours not saved".to_string()))?;
    Ok(Json(saved.into()))
}
