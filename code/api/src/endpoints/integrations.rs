//! Service-to-service endpoints for app plugins that use Kubarr as their
//! identity provider. These sit outside session authentication: each caller
//! presents the managed key Kubarr gave its namespace.

use axum::{
    extract::{FromRequestParts, State},
    http::{header, request::Parts},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;

use crate::error::{AppError, Result};
use crate::services::jellyfin_identity::{self, IdentitySnapshot, IdentityUser};
use crate::state::AppState;

pub fn integration_routes(state: AppState) -> Router {
    Router::new()
        .route("/jellyfin/verify", post(verify_jellyfin_credentials))
        .route("/jellyfin/users", get(jellyfin_users))
        .with_state(state)
}

/// Proof that the caller holds the Jellyfin identity key.
pub struct JellyfinPlugin;

impl FromRequestParts<AppState> for JellyfinPlugin {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> std::result::Result<Self, Self::Rejection> {
        let presented = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| AppError::Unauthorized("Missing service key".to_string()))?;
        let db = state.get_db().await?;
        if jellyfin_identity::is_identity_key(&db, presented).await? {
            Ok(JellyfinPlugin)
        } else {
            Err(AppError::Unauthorized("Invalid service key".to_string()))
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct VerifyRequest {
    pub username: String,
    pub password: String,
}

/// Check a Jellyfin native client's credentials against Kubarr.
#[utoipa::path(
    post,
    path = "/api/integrations/jellyfin/verify",
    tag = "Integrations",
    request_body = VerifyRequest,
    responses(
        (status = 200, body = IdentityUser),
        (status = 401, description = "Bad credentials or service key"),
        (status = 403, description = "User may not use Jellyfin"),
        (status = 429, description = "Too many failed sign-ins")
    )
)]
async fn verify_jellyfin_credentials(
    State(state): State<AppState>,
    _plugin: JellyfinPlugin,
    Json(request): Json<VerifyRequest>,
) -> Result<Json<IdentityUser>> {
    if request.username.trim().is_empty() || request.password.is_empty() {
        return Err(AppError::Unauthorized("Invalid credentials".to_string()));
    }
    let db = state.get_db().await?;
    let user =
        jellyfin_identity::verify(&db, &state, request.username.trim(), &request.password).await?;
    Ok(Json(user))
}

/// Kubarr users who belong in Jellyfin.
#[utoipa::path(
    get,
    path = "/api/integrations/jellyfin/users",
    tag = "Integrations",
    responses((status = 200, body = IdentitySnapshot))
)]
async fn jellyfin_users(
    State(state): State<AppState>,
    _plugin: JellyfinPlugin,
) -> Result<Json<IdentitySnapshot>> {
    let db = state.get_db().await?;
    Ok(Json(jellyfin_identity::snapshot(&db).await?))
}

/// After a successful change to users or roles, push the identity snapshot to
/// plugins so access changes reach Jellyfin within seconds.
pub async fn push_identity_on_change(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mutating = !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    let response = next.run(req).await;
    if mutating && response.status().is_success() {
        jellyfin_identity::notify_changed(&state);
    }
    response
}
