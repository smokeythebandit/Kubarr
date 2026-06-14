use axum::{
    extract::{Path, Query, State},
    http::{header::SET_COOKIE, HeaderMap, HeaderValue},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get},
    Json, Router,
};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, ModelTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};

use crate::config::CONFIG;
use crate::error::{AppError, Result};
use crate::middleware::permissions::{Authenticated, Authorized, SettingsManage, SettingsView};
use crate::models::prelude::*;
use crate::models::{oauth_account, oauth_provider, user};
use crate::services::{create_access_token, generate_random_string, hash_password};
use crate::state::AppState;

/// Create OAuth routes
pub fn oauth_routes(state: AppState) -> Router {
    Router::new()
        // Public: Available providers (for login page)
        .route("/available", get(list_available_providers))
        // Admin: Provider configuration
        .route("/providers", get(list_providers))
        .route(
            "/providers/{provider}",
            get(get_provider).put(update_provider),
        )
        // OAuth flow
        .route("/{provider}/login", get(oauth_login))
        .route("/{provider}/callback", get(oauth_callback))
        // Account linking (authenticated users)
        .route("/accounts", get(list_linked_accounts))
        .route("/accounts/{provider}", delete(unlink_account))
        .route("/link/{provider}", get(link_account_start))
        .with_state(state)
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProviderResponse {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub client_id: Option<String>,
    pub has_secret: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateProviderRequest {
    pub enabled: Option<bool>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct OAuthCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LinkedAccountResponse {
    pub provider: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub linked_at: String,
}

// ============================================================================
// Public Endpoints
// ============================================================================

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AvailableProvider {
    pub id: String,
    pub name: String,
}

/// List available (enabled) OAuth providers - public endpoint for login page
#[utoipa::path(
    get,
    path = "/api/oauth/available",
    tag = "OAuth",
    responses(
        (status = 200, body = Vec<AvailableProvider>)
    )
)]
async fn list_available_providers(
    State(state): State<AppState>,
) -> Result<Json<Vec<AvailableProvider>>> {
    let db = state.get_db().await?;
    let providers = OauthProvider::find()
        .filter(oauth_provider::Column::Enabled.eq(true))
        .all(&db)
        .await?;

    let available: Vec<AvailableProvider> = providers
        .into_iter()
        .filter(|p| p.client_id.is_some() && p.client_secret.is_some())
        .map(|p| AvailableProvider {
            id: p.id,
            name: p.name,
        })
        .collect();

    Ok(Json(available))
}

// ============================================================================
// Admin Endpoints
// ============================================================================

/// List all OAuth providers
#[utoipa::path(
    get,
    path = "/api/oauth/providers",
    tag = "OAuth",
    responses(
        (status = 200, body = Vec<ProviderResponse>)
    )
)]
async fn list_providers(
    State(state): State<AppState>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<Vec<ProviderResponse>>> {
    let db = state.get_db().await?;
    let providers = OauthProvider::find().all(&db).await?;

    let responses: Vec<ProviderResponse> = providers
        .into_iter()
        .map(|p| ProviderResponse {
            id: p.id,
            name: p.name,
            enabled: p.enabled,
            client_id: p.client_id,
            has_secret: p.client_secret.is_some(),
        })
        .collect();

    Ok(Json(responses))
}

/// Get a specific OAuth provider
#[utoipa::path(
    get,
    path = "/api/oauth/providers/{provider}",
    tag = "OAuth",
    params(
        ("provider" = String, Path, description = "Provider ID")
    ),
    responses(
        (status = 200, body = ProviderResponse)
    )
)]
async fn get_provider(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<ProviderResponse>> {
    let db = state.get_db().await?;
    let provider_model = OauthProvider::find_by_id(&provider)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Provider '{}' not found", provider)))?;

    Ok(Json(ProviderResponse {
        id: provider_model.id,
        name: provider_model.name,
        enabled: provider_model.enabled,
        client_id: provider_model.client_id,
        has_secret: provider_model.client_secret.is_some(),
    }))
}

/// Update OAuth provider settings
#[utoipa::path(
    put,
    path = "/api/oauth/providers/{provider}",
    tag = "OAuth",
    params(
        ("provider" = String, Path, description = "Provider ID")
    ),
    request_body = UpdateProviderRequest,
    responses(
        (status = 200, body = ProviderResponse)
    )
)]
async fn update_provider(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    _auth: Authorized<SettingsManage>,
    Json(data): Json<UpdateProviderRequest>,
) -> Result<Json<ProviderResponse>> {
    let db = state.get_db().await?;
    // Find or create provider
    let existing = OauthProvider::find_by_id(&provider).one(&db).await?;

    let now = Utc::now();
    let provider_model = if let Some(existing) = existing {
        let mut model: oauth_provider::ActiveModel = existing.into();
        if let Some(enabled) = data.enabled {
            model.enabled = Set(enabled);
        }
        if let Some(client_id) = data.client_id {
            model.client_id = Set(Some(client_id));
        }
        if let Some(client_secret) = data.client_secret {
            model.client_secret = Set(Some(client_secret));
        }
        model.updated_at = Set(now);
        model.update(&db).await?
    } else {
        // Create new provider
        let name = match provider.as_str() {
            "google" => "Google",
            "microsoft" => "Microsoft",
            _ => &provider,
        };
        let new_provider = oauth_provider::ActiveModel {
            id: Set(provider.clone()),
            name: Set(name.to_string()),
            enabled: Set(data.enabled.unwrap_or(false)),
            client_id: Set(data.client_id),
            client_secret: Set(data.client_secret),
            created_at: Set(now),
            updated_at: Set(now),
        };
        new_provider.insert(&db).await?
    };

    Ok(Json(ProviderResponse {
        id: provider_model.id,
        name: provider_model.name,
        enabled: provider_model.enabled,
        client_id: provider_model.client_id,
        has_secret: provider_model.client_secret.is_some(),
    }))
}

// ============================================================================
// OAuth Flow
// ============================================================================

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct OAuthLoginQuery {
    pub link: Option<String>, // If set, this is a linking flow (value is user_id)
}

/// Initiate OAuth login flow
#[utoipa::path(
    get,
    path = "/api/oauth/{provider}/login",
    tag = "OAuth",
    params(
        ("provider" = String, Path, description = "OAuth provider ID")
    ),
    responses(
        (status = 302, description = "Redirect")
    )
)]
async fn oauth_login(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(query): Query<OAuthLoginQuery>,
) -> Result<Response> {
    let db = state.get_db().await?;
    let provider_config = OauthProvider::find_by_id(&provider)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Provider '{}' not found", provider)))?;

    if !provider_config.enabled {
        return Err(AppError::BadRequest(format!(
            "Provider '{}' is not enabled",
            provider
        )));
    }

    let client_id = provider_config
        .client_id
        .ok_or_else(|| AppError::BadRequest("Provider client_id not configured".to_string()))?;

    // Generate state token (includes link user_id if linking)
    let state_token = if let Some(link_user) = query.link {
        format!("link:{}:{}", link_user, generate_random_string(16))
    } else {
        format!("login:{}", generate_random_string(16))
    };

    // Build authorization URL based on provider
    let (auth_url, scopes) = match provider.as_str() {
        "google" => (
            "https://accounts.google.com/o/oauth2/v2/auth",
            "openid email profile",
        ),
        "microsoft" => (
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            "openid email profile offline_access",
        ),
        _ => {
            return Err(AppError::BadRequest(format!(
                "Unknown provider: {}",
                provider
            )))
        }
    };

    let redirect_uri = format!(
        "{}/api/oauth/{}/callback",
        CONFIG.auth.oauth2_issuer_url, provider
    );

    let url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}",
        auth_url,
        urlencoding::encode(&client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(scopes),
        urlencoding::encode(&state_token),
    );

    Ok(Redirect::to(&url).into_response())
}

/// OAuth callback handler
#[utoipa::path(
    get,
    path = "/api/oauth/{provider}/callback",
    tag = "OAuth",
    params(
        ("provider" = String, Path, description = "OAuth provider ID")
    ),
    responses(
        (status = 302, description = "Redirect")
    )
)]
async fn oauth_callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Result<Response> {
    let db = state.get_db().await?;
    // Check for errors
    if let Some(error) = query.error {
        let msg = query.error_description.unwrap_or(error);
        return Ok(
            Redirect::to(&format!("/login?error={}", urlencoding::encode(&msg))).into_response(),
        );
    }

    let code = query
        .code
        .ok_or_else(|| AppError::BadRequest("Missing authorization code".to_string()))?;

    let state_token = query
        .state
        .ok_or_else(|| AppError::BadRequest("Missing state parameter".to_string()))?;

    // Parse state to determine if this is login or linking
    let is_linking = state_token.starts_with("link:");
    let link_user_id: Option<i64> = if is_linking {
        let parts: Vec<&str> = state_token.split(':').collect();
        parts.get(1).and_then(|s| s.parse().ok())
    } else {
        None
    };

    // Get provider config
    let provider_config = OauthProvider::find_by_id(&provider)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Provider '{}' not found", provider)))?;

    let client_id = provider_config
        .client_id
        .ok_or_else(|| AppError::Internal("Provider client_id not configured".to_string()))?;
    let client_secret = provider_config
        .client_secret
        .ok_or_else(|| AppError::Internal("Provider client_secret not configured".to_string()))?;

    let redirect_uri = format!(
        "{}/api/oauth/{}/callback",
        CONFIG.auth.oauth2_issuer_url, provider
    );

    // Exchange code for tokens
    let (token_url, userinfo_url) = match provider.as_str() {
        "google" => (
            "https://oauth2.googleapis.com/token",
            "https://www.googleapis.com/oauth2/v3/userinfo",
        ),
        "microsoft" => (
            "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            "https://graph.microsoft.com/v1.0/me",
        ),
        _ => {
            return Err(AppError::BadRequest(format!(
                "Unknown provider: {}",
                provider
            )))
        }
    };

    let http_client = reqwest::Client::new();

    // Exchange code for token
    let token_response = http_client
        .post(token_url)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Token exchange failed: {}", e)))?;

    if !token_response.status().is_success() {
        let error_text = token_response.text().await.unwrap_or_default();
        tracing::error!("OAuth token exchange failed: {}", error_text);
        return Ok(Redirect::to("/login?error=OAuth%20authentication%20failed").into_response());
    }

    let token_data: serde_json::Value = token_response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse token response: {}", e)))?;

    let access_token = token_data["access_token"]
        .as_str()
        .ok_or_else(|| AppError::Internal("No access token in response".to_string()))?;

    // Fetch user info
    let userinfo_response = http_client
        .get(userinfo_url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to fetch user info: {}", e)))?;

    if !userinfo_response.status().is_success() {
        return Ok(Redirect::to("/login?error=Failed%20to%20fetch%20user%20info").into_response());
    }

    let userinfo: serde_json::Value = userinfo_response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse user info: {}", e)))?;

    // Extract user info based on provider
    let (provider_user_id, email, display_name) = match provider.as_str() {
        "google" => (
            userinfo["sub"].as_str().map(|s| s.to_string()),
            userinfo["email"].as_str().map(|s| s.to_string()),
            userinfo["name"].as_str().map(|s| s.to_string()),
        ),
        "microsoft" => (
            userinfo["id"].as_str().map(|s| s.to_string()),
            userinfo["mail"]
                .as_str()
                .or_else(|| userinfo["userPrincipalName"].as_str())
                .map(|s| s.to_string()),
            userinfo["displayName"].as_str().map(|s| s.to_string()),
        ),
        _ => return Err(AppError::Internal("Unknown provider".to_string())),
    };

    let provider_user_id = provider_user_id
        .ok_or_else(|| AppError::Internal("No user ID from provider".to_string()))?;

    // Check if this OAuth account is already linked
    let existing_oauth = OauthAccount::find()
        .filter(oauth_account::Column::Provider.eq(&provider))
        .filter(oauth_account::Column::ProviderUserId.eq(&provider_user_id))
        .one(&db)
        .await?;

    if is_linking {
        // Linking flow - add OAuth account to existing user
        let user_id = link_user_id
            .ok_or_else(|| AppError::BadRequest("Invalid linking state".to_string()))?;

        if existing_oauth.is_some() {
            return Ok(Redirect::to(
                "/account?error=This%20account%20is%20already%20linked%20to%20another%20user",
            )
            .into_response());
        }

        // Create the link
        let now = Utc::now();
        let new_oauth = oauth_account::ActiveModel {
            user_id: Set(user_id),
            provider: Set(provider.clone()),
            provider_user_id: Set(provider_user_id),
            email: Set(email),
            display_name: Set(display_name),
            access_token: Set(Some(access_token.to_string())),
            refresh_token: Set(token_data["refresh_token"].as_str().map(|s| s.to_string())),
            token_expires_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        new_oauth.insert(&db).await?;

        return Ok(
            Redirect::to("/account?success=Account%20linked%20successfully").into_response(),
        );
    }

    // Login flow
    let found_user = if let Some(oauth) = existing_oauth {
        // User exists with this OAuth link
        User::find_by_id(oauth.user_id).one(&db).await?
    } else {
        // No existing link - try to find user by email or create new account
        let found_user = if let Some(ref email) = email {
            User::find()
                .filter(user::Column::Email.eq(email))
                .one(&db)
                .await?
        } else {
            None
        };

        if let Some(found_user) = found_user {
            // Link OAuth to existing user (found by email)
            let now = Utc::now();
            let new_oauth = oauth_account::ActiveModel {
                user_id: Set(found_user.id),
                provider: Set(provider.clone()),
                provider_user_id: Set(provider_user_id),
                email: Set(email.clone()),
                display_name: Set(display_name.clone()),
                access_token: Set(Some(access_token.to_string())),
                refresh_token: Set(token_data["refresh_token"].as_str().map(|s| s.to_string())),
                token_expires_at: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            };
            new_oauth.insert(&db).await?;
            Some(found_user)
        } else {
            // Create new user
            let email = email.ok_or_else(|| {
                AppError::BadRequest("Email is required for new accounts".to_string())
            })?;

            // Generate username from email or display name
            let username = display_name
                .clone()
                .unwrap_or_else(|| email.split('@').next().unwrap_or("user").to_string())
                .to_lowercase()
                .replace(' ', "_");

            // Make sure username is unique
            let mut final_username = username.clone();
            let mut counter = 1;
            while User::find()
                .filter(user::Column::Username.eq(&final_username))
                .one(&db)
                .await?
                .is_some()
            {
                final_username = format!("{}_{}", username, counter);
                counter += 1;
            }

            // Create user with random password (they'll use OAuth to login)
            let random_password = generate_random_string(32);
            let password_hash = hash_password(&random_password)?;

            let now = Utc::now();
            let new_user = user::ActiveModel {
                username: Set(final_username),
                email: Set(email.clone()),
                hashed_password: Set(password_hash),
                is_active: Set(true),
                is_approved: Set(true), // Auto-approve OAuth users
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            };

            let created_user = new_user.insert(&db).await?;

            // Link OAuth account
            let new_oauth = oauth_account::ActiveModel {
                user_id: Set(created_user.id),
                provider: Set(provider.clone()),
                provider_user_id: Set(provider_user_id),
                email: Set(Some(email)),
                display_name: Set(display_name),
                access_token: Set(Some(access_token.to_string())),
                refresh_token: Set(token_data["refresh_token"].as_str().map(|s| s.to_string())),
                token_expires_at: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            };
            new_oauth.insert(&db).await?;

            Some(created_user)
        }
    };

    let found_user = found_user
        .ok_or_else(|| AppError::Internal("Failed to find or create user".to_string()))?;

    // Check if user is active and approved
    if !found_user.is_active {
        return Ok(Redirect::to("/login?error=Account%20is%20inactive").into_response());
    }
    if !found_user.is_approved {
        return Ok(Redirect::to("/login?error=Account%20pending%20approval").into_response());
    }

    // Create session token
    use crate::endpoints::extractors::{get_user_app_access, get_user_permissions};

    let permissions = get_user_permissions(&db, found_user.id).await;
    let allowed_apps = get_user_app_access(&db, found_user.id).await;

    let session_token = create_access_token(
        &found_user.id.to_string(),
        Some(&found_user.email),
        None,
        None,
        None,
        Some(permissions),
        Some(allowed_apps),
    )?;

    // Set session cookie and redirect
    let cookie = format!(
        "kubarr_session={}; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800",
        session_token
    );

    let mut headers = HeaderMap::new();
    headers.insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie)
            .map_err(|e| AppError::Internal(format!("invalid cookie header: {}", e)))?,
    );

    Ok((headers, Redirect::to("/")).into_response())
}

// ============================================================================
// Account Linking
// ============================================================================

/// List OAuth accounts linked to current user
#[utoipa::path(
    get,
    path = "/api/oauth/accounts",
    tag = "OAuth",
    responses(
        (status = 200, body = Vec<LinkedAccountResponse>)
    )
)]
async fn list_linked_accounts(
    State(state): State<AppState>,
    auth: Authenticated,
) -> Result<Json<Vec<LinkedAccountResponse>>> {
    let db = state.get_db().await?;
    let accounts = OauthAccount::find()
        .filter(oauth_account::Column::UserId.eq(auth.user_id()))
        .all(&db)
        .await?;

    let responses: Vec<LinkedAccountResponse> = accounts
        .into_iter()
        .map(|a| LinkedAccountResponse {
            provider: a.provider,
            email: a.email,
            display_name: a.display_name,
            linked_at: a.created_at.to_string(),
        })
        .collect();

    Ok(Json(responses))
}

/// Unlink an OAuth account
#[utoipa::path(
    delete,
    path = "/api/oauth/accounts/{provider}",
    tag = "OAuth",
    params(
        ("provider" = String, Path, description = "OAuth provider ID")
    ),
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn unlink_account(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    auth: Authenticated,
) -> Result<Json<serde_json::Value>> {
    let db = state.get_db().await?;
    let account = OauthAccount::find()
        .filter(oauth_account::Column::UserId.eq(auth.user_id()))
        .filter(oauth_account::Column::Provider.eq(&provider))
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("No {} account linked", provider)))?;

    account.delete(&db).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": format!("{} account unlinked", provider)
    })))
}

/// Start OAuth linking flow for current user
#[utoipa::path(
    get,
    path = "/api/oauth/link/{provider}",
    tag = "OAuth",
    params(
        ("provider" = String, Path, description = "OAuth provider ID")
    ),
    responses(
        (status = 302, description = "Redirect")
    )
)]
async fn link_account_start(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    auth: Authenticated,
) -> Result<Response> {
    let db = state.get_db().await?;
    // Check if already linked
    let existing = OauthAccount::find()
        .filter(oauth_account::Column::UserId.eq(auth.user_id()))
        .filter(oauth_account::Column::Provider.eq(&provider))
        .one(&db)
        .await?;

    if existing.is_some() {
        return Err(AppError::BadRequest(format!(
            "You already have a {} account linked",
            provider
        )));
    }

    // Redirect to OAuth login with link parameter
    let redirect_url = format!("/api/oauth/{}/login?link={}", provider, auth.user_id());
    Ok(Redirect::to(&redirect_url).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // ProviderResponse serialization
    // -------------------------------------------------------------------------

    #[test]
    fn provider_response_serializes_with_secret() {
        let resp = ProviderResponse {
            id: "google".to_string(),
            name: "Google".to_string(),
            enabled: true,
            client_id: Some("my-client-id".to_string()),
            has_secret: true,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["id"], "google");
        assert_eq!(json["name"], "Google");
        assert_eq!(json["enabled"], true);
        assert_eq!(json["client_id"], "my-client-id");
        assert_eq!(json["has_secret"], true);
    }

    #[test]
    fn provider_response_serializes_without_client_id() {
        let resp = ProviderResponse {
            id: "microsoft".to_string(),
            name: "Microsoft".to_string(),
            enabled: false,
            client_id: None,
            has_secret: false,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["id"], "microsoft");
        assert_eq!(json["enabled"], false);
        assert!(json["client_id"].is_null());
        assert_eq!(json["has_secret"], false);
    }

    // -------------------------------------------------------------------------
    // AvailableProvider serialization
    // -------------------------------------------------------------------------

    #[test]
    fn available_provider_serializes() {
        let provider = AvailableProvider {
            id: "google".to_string(),
            name: "Google".to_string(),
        };
        let json = serde_json::to_value(&provider).unwrap();
        assert_eq!(json["id"], "google");
        assert_eq!(json["name"], "Google");
    }

    #[test]
    fn available_provider_list_serializes() {
        let providers = vec![
            AvailableProvider {
                id: "google".to_string(),
                name: "Google".to_string(),
            },
            AvailableProvider {
                id: "microsoft".to_string(),
                name: "Microsoft".to_string(),
            },
        ];
        let json = serde_json::to_value(&providers).unwrap();
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 2);
        assert_eq!(json[0]["id"], "google");
        assert_eq!(json[1]["id"], "microsoft");
    }

    // -------------------------------------------------------------------------
    // UpdateProviderRequest deserialization
    // -------------------------------------------------------------------------

    #[test]
    fn update_provider_request_deserializes_full() {
        let json = r#"{"enabled": true, "client_id": "abc123", "client_secret": "secret456"}"#;
        let req: UpdateProviderRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.enabled, Some(true));
        assert_eq!(req.client_id.as_deref(), Some("abc123"));
        assert_eq!(req.client_secret.as_deref(), Some("secret456"));
    }

    #[test]
    fn update_provider_request_deserializes_partial() {
        let json = r#"{"enabled": false}"#;
        let req: UpdateProviderRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.enabled, Some(false));
        assert!(req.client_id.is_none());
        assert!(req.client_secret.is_none());
    }

    #[test]
    fn update_provider_request_deserializes_empty() {
        let json = r#"{}"#;
        let req: UpdateProviderRequest = serde_json::from_str(json).unwrap();
        assert!(req.enabled.is_none());
        assert!(req.client_id.is_none());
        assert!(req.client_secret.is_none());
    }

    // -------------------------------------------------------------------------
    // OAuthCallbackQuery deserialization
    // -------------------------------------------------------------------------

    #[test]
    fn oauth_callback_query_deserializes_success() {
        let json = r#"{"code": "auth-code-123", "state": "login:abcdef"}"#;
        let q: OAuthCallbackQuery = serde_json::from_str(json).unwrap();
        assert_eq!(q.code.as_deref(), Some("auth-code-123"));
        assert_eq!(q.state.as_deref(), Some("login:abcdef"));
        assert!(q.error.is_none());
        assert!(q.error_description.is_none());
    }

    #[test]
    fn oauth_callback_query_deserializes_error() {
        let json = r#"{"error": "access_denied", "error_description": "User denied access"}"#;
        let q: OAuthCallbackQuery = serde_json::from_str(json).unwrap();
        assert!(q.code.is_none());
        assert!(q.state.is_none());
        assert_eq!(q.error.as_deref(), Some("access_denied"));
        assert_eq!(q.error_description.as_deref(), Some("User denied access"));
    }

    #[test]
    fn oauth_callback_query_deserializes_all_none() {
        let json = r#"{}"#;
        let q: OAuthCallbackQuery = serde_json::from_str(json).unwrap();
        assert!(q.code.is_none());
        assert!(q.state.is_none());
        assert!(q.error.is_none());
        assert!(q.error_description.is_none());
    }

    // -------------------------------------------------------------------------
    // LinkedAccountResponse serialization
    // -------------------------------------------------------------------------

    #[test]
    fn linked_account_response_serializes_with_optional_fields() {
        let resp = LinkedAccountResponse {
            provider: "google".to_string(),
            email: Some("user@example.com".to_string()),
            display_name: Some("Jane Doe".to_string()),
            linked_at: "2026-02-22T00:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["provider"], "google");
        assert_eq!(json["email"], "user@example.com");
        assert_eq!(json["display_name"], "Jane Doe");
        assert_eq!(json["linked_at"], "2026-02-22T00:00:00Z");
    }

    #[test]
    fn linked_account_response_serializes_without_optional_fields() {
        let resp = LinkedAccountResponse {
            provider: "microsoft".to_string(),
            email: None,
            display_name: None,
            linked_at: "2026-02-22T00:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["provider"], "microsoft");
        assert!(json["email"].is_null());
        assert!(json["display_name"].is_null());
    }

    // -------------------------------------------------------------------------
    // OAuthLoginQuery deserialization
    // -------------------------------------------------------------------------

    #[test]
    fn oauth_login_query_deserializes_with_link() {
        let json = r#"{"link": "42"}"#;
        let q: OAuthLoginQuery = serde_json::from_str(json).unwrap();
        assert_eq!(q.link.as_deref(), Some("42"));
    }

    #[test]
    fn oauth_login_query_deserializes_without_link() {
        let json = r#"{}"#;
        let q: OAuthLoginQuery = serde_json::from_str(json).unwrap();
        assert!(q.link.is_none());
    }
}
