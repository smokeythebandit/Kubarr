//! App passwords: per-device passwords for clients that cannot complete
//! Kubarr's sign-in, such as Jellyfin's TV and phone apps. A user with
//! two-factor authentication must use one of these there; anyone may, so a
//! lost device can be cut off without changing the account password.

use axum::{
    extract::{Path, State},
    routing::{delete, get},
    Json, Router,
};
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::middleware::Authenticated;
use crate::models::app_password;
use crate::models::prelude::*;
use crate::services::jellyfin_identity::generate_app_password;
use crate::state::AppState;

/// Enough for every device a household owns; bounds the table per user.
const MAX_APP_PASSWORDS: u64 = 25;

pub fn app_password_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_app_passwords).post(create_app_password))
        .route("/{id}", delete(delete_app_password))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AppPasswordInfo {
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl From<app_password::Model> for AppPasswordInfo {
    fn from(m: app_password::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            created_at: m.created_at,
            last_used_at: m.last_used_at,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateAppPasswordRequest {
    pub name: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CreatedAppPassword {
    #[serde(flatten)]
    pub info: AppPasswordInfo,
    /// Shown once; Kubarr keeps only a digest.
    pub password: String,
}

/// List the current user's app passwords.
#[utoipa::path(
    get,
    path = "/api/users/me/app-passwords",
    tag = "Users",
    responses((status = 200, body = Vec<AppPasswordInfo>))
)]
pub(crate) async fn list_app_passwords(
    State(state): State<AppState>,
    auth: Authenticated,
) -> Result<Json<Vec<AppPasswordInfo>>> {
    let db = state.get_db().await?;
    let rows = AppPassword::find()
        .filter(app_password::Column::UserId.eq(auth.user_id()))
        .order_by_desc(app_password::Column::CreatedAt)
        .all(&db)
        .await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

/// Create an app password. The response is the only time it is shown.
#[utoipa::path(
    post,
    path = "/api/users/me/app-passwords",
    tag = "Users",
    request_body = CreateAppPasswordRequest,
    responses((status = 200, body = CreatedAppPassword))
)]
pub(crate) async fn create_app_password(
    State(state): State<AppState>,
    auth: Authenticated,
    Json(request): Json<CreateAppPasswordRequest>,
) -> Result<Json<CreatedAppPassword>> {
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AppError::BadRequest(
            "Name must be 1 to 64 characters".to_string(),
        ));
    }
    let db = state.get_db().await?;
    let existing = AppPassword::find()
        .filter(app_password::Column::UserId.eq(auth.user_id()))
        .count(&db)
        .await?;
    if existing >= MAX_APP_PASSWORDS {
        return Err(AppError::BadRequest(format!(
            "At most {MAX_APP_PASSWORDS} app passwords; delete one first"
        )));
    }

    let (password, digest) = generate_app_password();
    let row = app_password::ActiveModel {
        user_id: Set(auth.user_id()),
        name: Set(name.to_string()),
        token_hash: Set(digest),
        created_at: Set(Utc::now()),
        last_used_at: Set(None),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    Ok(Json(CreatedAppPassword {
        info: row.into(),
        password,
    }))
}

/// Revoke one of the current user's app passwords.
#[utoipa::path(
    delete,
    path = "/api/users/me/app-passwords/{id}",
    tag = "Users",
    params(("id" = i64, Path, description = "App password id")),
    responses((status = 200))
)]
pub(crate) async fn delete_app_password(
    State(state): State<AppState>,
    auth: Authenticated,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>> {
    let db = state.get_db().await?;
    let result = AppPassword::delete_many()
        .filter(app_password::Column::Id.eq(id))
        .filter(app_password::Column::UserId.eq(auth.user_id()))
        .exec(&db)
        .await?;
    if result.rows_affected == 0 {
        return Err(AppError::NotFound("App password not found".to_string()));
    }
    Ok(Json(serde_json::json!({ "deleted": id })))
}
