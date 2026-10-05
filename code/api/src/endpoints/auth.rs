use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{header, HeaderMap, HeaderValue},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use chrono::{Duration, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, ExprTrait, PaginatorTrait, QueryFilter, Set,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::config::CONFIG;
use crate::error::{AppError, Result};
use crate::middleware::auth::{
    ACTIVE_SESSION_COOKIE, MAX_SESSIONS, SESSION_COOKIE_BASE, SESSION_COOKIE_NAME,
};
use crate::models::prelude::*;
use crate::models::{
    app_domain_assignment,
    audit_log::{AuditAction, ResourceType},
    domain, invite, role, session, two_factor_recovery_code, user, user_role,
};
use crate::services::{
    create_session_token, decode_session_token, hash_password, verify_password,
    verify_recovery_code, verify_totp,
};
use crate::state::AppState;

fn audit_context(
    headers: &HeaderMap,
    peer: Option<ConnectInfo<std::net::SocketAddr>>,
) -> (Option<String>, Option<String>) {
    // This is the immediate TCP peer (possibly a gateway pod), not necessarily
    // the originating client. Forwarded headers are not authenticated here.
    let ip = peer.map(|ConnectInfo(addr)| addr.ip().to_string());
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.chars().take(255).collect());
    (ip, agent)
}

async fn audit_failure(
    state: &AppState,
    action: AuditAction,
    actor: Option<&user::Model>,
    reason: &'static str,
    ip: &Option<String>,
    agent: &Option<String>,
) {
    if state
        .audit
        .log(
            action,
            ResourceType::Session,
            actor.map(|u| u.id.to_string()),
            actor.map(|u| u.id),
            actor.map(|u| u.username.clone()),
            Some(serde_json::json!({"reason": reason})),
            ip.clone(),
            agent.clone(),
            false,
            None,
        )
        .await
        .is_err()
    {
        tracing::warn!("Failed to persist authentication failure audit event");
    }
}

async fn audit_success(
    tx: &sea_orm::DatabaseTransaction,
    action: AuditAction,
    user_id: i64,
    username: &str,
    ip: &Option<String>,
    agent: &Option<String>,
    details: Option<&'static str>,
) -> Result<()> {
    crate::services::audit::log_on_transaction(
        tx,
        action,
        ResourceType::Session,
        Some(user_id.to_string()),
        Some(user_id),
        Some(username.to_owned()),
        details.map(|method| serde_json::json!({"method": method})),
        ip.clone(),
        agent.clone(),
        true,
        None,
    )
    .await?;
    Ok(())
}

/// Create auth routes for session management
pub fn auth_routes(state: AppState) -> Router {
    Router::new()
        .route("/login", post(login))
        .route("/register", post(register))
        .route("/logout", post(logout))
        .route("/sessions", get(list_sessions))
        .route("/sessions/{session_id}", delete(revoke_session))
        .route("/switch/{slot}", post(switch_session))
        .route("/accounts", get(list_accounts))
        .route("/host-transfer/start", get(start_host_transfer))
        .route("/host-transfer/complete", get(complete_host_transfer))
        .route("/2fa/recover", post(recover_with_code))
        .with_state(state)
}

#[derive(Deserialize)]
struct StartTransfer {
    app: String,
}

#[derive(Deserialize)]
struct CompleteTransfer {
    ticket: String,
}

const TRANSFER_MARKER: &str = "host-transfer|";

fn transfer_record_id(ticket: uuid::Uuid) -> String {
    ticket.to_string()
}

// The sessions.user_agent column is a 255-character string in PostgreSQL.
fn transfer_metadata(host: &str, app: &str, sid: &str) -> Option<String> {
    let metadata = format!("{TRANSFER_MARKER}{host}|{app}|{sid}");
    (metadata.len() <= 255).then_some(metadata)
}

fn transfer_source<'a>(agent: Option<&'a str>, host: &str) -> Option<(&'a str, &'a str)> {
    let metadata = agent?.strip_prefix(TRANSFER_MARKER)?;
    let (expected_host, rest) = metadata.split_once('|')?;
    let (app, sid) = rest.split_once('|')?;
    if expected_host != host || !valid_hostname(expected_host) {
        return None;
    }
    if !valid_app_name(app) {
        return None;
    }
    let parsed = uuid::Uuid::parse_str(sid).ok()?;
    (parsed.to_string() == sid).then_some((app, sid))
}

fn valid_app_name(app: &str) -> bool {
    !app.is_empty()
        && app
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_hostname(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

fn request_host(headers: &HeaderMap) -> Option<(String, String)> {
    let raw = headers.get(header::HOST)?.to_str().ok()?;
    let (host, port) = match raw.rsplit_once(':') {
        Some((host, port))
            if !host.contains(':') && port.parse::<u16>().ok().filter(|p| *p > 0).is_some() =>
        {
            (host, format!(":{port}"))
        }
        Some(_) => return None,
        None => (raw, String::new()),
    };
    let host = host.to_ascii_lowercase();
    valid_hostname(&host).then_some((host, port))
}

fn transfer_target(
    assignment: &app_domain_assignment::Model,
    domain: &domain::Model,
) -> Option<String> {
    if !assignment.enabled
        || !domain.enabled
        || !matches!(assignment.route_mode.as_str(), "exact_host" | "subdomain")
    {
        return None;
    }
    let hostname = assignment.hostname.as_deref()?.trim().to_ascii_lowercase();
    let target = if assignment.route_mode == "exact_host" || hostname.contains('.') {
        hostname
    } else {
        format!("{}.{}", hostname, domain.domain.trim_start_matches("*."))
    };
    valid_hostname(&target).then_some(target)
}

fn transfer_port<'a>(source: &str, target: &str, port: &'a str) -> &'a str {
    // Local *.localhost aliases share the gateway's development port. An
    // external host has no configured port: use its scheme's default instead.
    if (source == "localhost" || source.ends_with(".localhost"))
        && (target == "localhost" || target.ends_with(".localhost"))
    {
        port
    } else {
        ""
    }
}

async fn start_host_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<StartTransfer>,
) -> Result<Response> {
    let (source_host, port) =
        request_host(&headers).ok_or_else(|| AppError::BadRequest("Invalid host".into()))?;
    if !valid_app_name(&query.app) {
        return Err(AppError::BadRequest("Invalid app".into()));
    }
    let req = axum::extract::Request::builder()
        .header(
            header::COOKIE,
            headers
                .get(header::COOKIE)
                .cloned()
                .unwrap_or_else(|| HeaderValue::from_static("")),
        )
        .body(axum::body::Body::empty())
        .map_err(|_| AppError::Unauthorized("Invalid session".into()))?;
    let token = crate::middleware::auth::extract_token(&req)
        .ok_or_else(|| AppError::Unauthorized("Missing session".into()))?;
    let user = crate::middleware::auth::authenticate_session(&state, &token)
        .await
        .map_err(AppError::Unauthorized)?;
    if !user.has_app_access(&query.app) {
        return Err(AppError::Forbidden("No app access".into()));
    }
    let db = state.get_db().await?;
    let assignments = AppDomainAssignment::find()
        .filter(app_domain_assignment::Column::AppName.eq(&query.app))
        .filter(app_domain_assignment::Column::Enabled.eq(true))
        .all(&db)
        .await?;
    let mut target = None;
    for assignment in assignments {
        if let Some(domain) = Domain::find_by_id(assignment.domain_id).one(&db).await? {
            if let Some(host) = transfer_target(&assignment, &domain) {
                if assignment.primary || target.is_none() {
                    target = Some(host);
                }
                if assignment.primary {
                    break;
                }
            }
        }
    }
    let Some(target) = target else {
        return Ok(Redirect::to(&format!("/{}/", query.app)).into_response());
    };
    if target == source_host {
        return Ok(Redirect::to("/").into_response());
    }
    let claims = decode_session_token(&token)
        .map_err(|_| AppError::Unauthorized("Invalid session".into()))?;
    let metadata = transfer_metadata(&target, &query.app, &claims.sid)
        .ok_or_else(|| AppError::BadRequest("Transfer target too long".into()))?;
    let now = Utc::now();
    let ticket = uuid::Uuid::new_v4();
    session::ActiveModel {
        id: Set(transfer_record_id(ticket)),
        user_id: Set(user.user.id),
        user_agent: Set(Some(metadata)),
        ip_address: Set(None),
        created_at: Set(now),
        expires_at: Set(now + Duration::seconds(45)),
        last_accessed_at: Set(now),
        is_revoked: Set(false),
    }
    .insert(&db)
    .await?;
    let scheme = if headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        == Some("https")
    {
        "https"
    } else {
        "http"
    };
    let port = transfer_port(&source_host, &target, &port);
    let url = format!("{scheme}://{target}{port}/auth/host-transfer/complete?ticket={ticket}");
    let mut response = Redirect::to(&url).into_response();
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn complete_host_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CompleteTransfer>,
) -> Result<Response> {
    let (host, _) =
        request_host(&headers).ok_or_else(|| AppError::BadRequest("Invalid host".into()))?;
    let ticket = uuid::Uuid::parse_str(&query.ticket)
        .map_err(|_| AppError::Unauthorized("Invalid transfer".into()))?;
    let db = state.get_db().await?;
    let id = transfer_record_id(ticket);
    let tx = db.begin().await?;
    let record = Session::find_by_id(&id)
        .one(&tx)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid transfer".into()))?;
    let (app, source_sid) = transfer_source(record.user_agent.as_deref(), &host)
        .filter(|(_, sid)| *sid != id)
        .map(|(app, sid)| (app.to_owned(), sid.to_owned()))
        .ok_or_else(|| AppError::Unauthorized("Invalid transfer host".into()))?;
    if record.is_revoked || record.expires_at <= Utc::now() {
        return Err(AppError::Unauthorized("Expired transfer".into()));
    }
    let updated = Session::update_many()
        .col_expr(session::Column::IsRevoked, true.into())
        .filter(session::Column::Id.eq(&id))
        .filter(session::Column::IsRevoked.eq(false))
        .exec(&tx)
        .await?;
    if updated.rows_affected != 1 {
        return Err(AppError::Unauthorized("Used transfer".into()));
    }
    let source = Session::find_by_id(&source_sid)
        .one(&tx)
        .await?
        .filter(|s| s.user_id == record.user_id && !s.is_revoked && s.expires_at > Utc::now())
        .ok_or_else(|| AppError::Unauthorized("Expired session".into()))?;
    User::find_by_id(source.user_id)
        .one(&tx)
        .await?
        .filter(|u| u.is_active && u.is_approved)
        .ok_or_else(|| AppError::Unauthorized("Inactive account".into()))?;
    tx.commit().await?;
    let assignments = AppDomainAssignment::find()
        .filter(app_domain_assignment::Column::AppName.eq(&app))
        .filter(app_domain_assignment::Column::Enabled.eq(true))
        .all(&db)
        .await?;
    let permissions = crate::endpoints::extractors::get_user_permissions(&db, source.user_id).await;
    let mut authorized = false;
    for assignment in assignments {
        let Some(domain) = Domain::find_by_id(assignment.domain_id).one(&db).await? else {
            continue;
        };
        if transfer_target(&assignment, &domain).as_deref() == Some(&host)
            && permissions
                .iter()
                .any(|p| p == "app.*" || p == &format!("app.{app}"))
        {
            authorized = true;
            break;
        }
    }
    if !authorized {
        return Err(AppError::Forbidden("App access revoked".into()));
    }
    let token = create_session_token(&source_sid)?;
    let secure = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        == Some("https");
    let mut response = Redirect::to("/").into_response();
    let cookie_domain = cookie_domain_for_host(&host);
    response.headers_mut().append(
        header::SET_COOKIE,
        session_cookie_for_slot(0, &token, secure, cookie_domain),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        active_session_cookie(0, secure, cookie_domain),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        session_cookie(&token, secure, cookie_domain),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

#[cfg(test)]
mod host_transfer_tests {
    use super::*;

    #[test]
    fn ticket_record_id_fits_session_column() {
        let ticket = uuid::Uuid::new_v4();
        let id = transfer_record_id(ticket);
        assert_eq!(id, ticket.to_string());
        assert!(id.len() <= 36);
    }

    #[test]
    fn ticket_metadata_fits_session_user_agent_column() {
        let sid = uuid::Uuid::new_v4().to_string();
        let host = "a".repeat(240);
        assert!(transfer_metadata(&host, "plex", &sid).is_none());
        assert!(transfer_metadata("plex.example", "plex", &sid).is_some());
    }

    #[test]
    fn cookie_domain_only_applies_to_the_target_parent() {
        assert!(host_accepts_cookie_domain("example.test", "example.test"));
        assert!(host_accepts_cookie_domain(
            "Plex.Example.Test",
            "example.test"
        ));
        assert!(!host_accepts_cookie_domain(
            "unrelated.test",
            "example.test"
        ));
        assert!(!host_accepts_cookie_domain(
            "badexample.test",
            "example.test"
        ));
    }

    #[test]
    fn transfer_metadata_requires_discriminator_host_and_canonical_source_sid() {
        let sid = uuid::Uuid::new_v4().to_string();
        let host = "photos.example";
        let valid = format!("{TRANSFER_MARKER}{host}|plex|{sid}");
        assert_eq!(
            transfer_source(Some(&valid), host),
            Some(("plex", sid.as_str()))
        );
        for agent in [
            None,
            Some("Mozilla/5.0"),
            Some("photos.example|source-session"),
            Some("photos.example|"),
            Some("host-transfer|photos.example"),
            Some("host-transfer|other.example|source-session"),
            Some("host-transfer|photos.example|plex|not-a-uuid"),
            Some("host-transfer|photos.example|"),
            Some("host-transfer|photos.example||00000000-0000-0000-0000-000000000000"),
        ] {
            assert_eq!(transfer_source(agent, host), None, "{agent:?}");
        }
        assert_eq!(transfer_source(Some(&valid), "other.example"), None);
        assert_eq!(transfer_source(Some(&format!("{valid}|extra")), host), None);
    }

    #[test]
    fn host_and_port_validation_rejects_redirect_injection() {
        for bad in [
            "evil.test/path",
            "evil.test@trusted.test",
            "evil.test:99999",
            "evil.test:0",
            "evil.test:80@trusted",
            "evil.test\r\nLocation:evil.test",
            "-bad.test",
            "bad..test",
            "[::1]:80",
        ] {
            let mut headers = HeaderMap::new();
            if let Ok(host) = HeaderValue::from_str(bad) {
                headers.insert(header::HOST, host);
                assert!(request_host(&headers).is_none(), "{bad}");
            }
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HOST,
            HeaderValue::from_static("KUBARR.EXAMPLE:18080"),
        );
        assert_eq!(
            request_host(&headers),
            Some(("kubarr.example".into(), ":18080".into()))
        );
    }
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub totp_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    username: String,
    email: String,
    password: String,
    invite_code: Option<String>,
}

// Bound unauthenticated work (including bcrypt and database transactions).
static REGISTRATION_SLOTS: Semaphore = Semaphore::const_new(8);

/// Public registration never grants an administrator role or a session. Invite
/// consumption and the audit record commit atomically. Accounts registered here
/// have no roles; administrators can explicitly grant roles after registration.
async fn register(
    State(state): State<AppState>,
    Json(data): Json<RegisterRequest>,
) -> Result<Json<serde_json::Value>> {
    let username = data.username.trim();
    let email = data.email.trim();
    if username.len() < 3
        || username.len() > 64
        || email.len() > 255
        || !email.contains('@')
        || data.password.len() < 8
        || data.password.len() > 1024
    {
        return Err(AppError::BadRequest(
            "Invalid registration fields".to_string(),
        ));
    }
    let slot = Arc::new(
        REGISTRATION_SLOTS
            .try_acquire()
            .map_err(|_| AppError::ServiceUnavailable("Registration is busy".to_string()))?,
    );
    let db = state.get_db().await?;
    // bcrypt is CPU-bound: keep it off the async worker and outside the DB transaction.
    // The task also holds a permit so a cancelled request cannot free its slot
    // while its non-cancellable blocking hash is still running.
    let password = data.password;
    let hash_slot = Arc::clone(&slot);
    let hashed_password = tokio::task::spawn_blocking(move || {
        let _hash_slot = hash_slot;
        hash_password(&password)
    })
    .await
    .map_err(|e| AppError::Internal(format!("Password hashing task failed: {e}")))??;
    let tx = db.begin().await?;
    let enabled = SystemSetting::find_by_id("registration_enabled")
        .one(&tx)
        .await?
        .map(|setting| setting.value == "true")
        .unwrap_or(true);
    let approval = SystemSetting::find_by_id("registration_require_approval")
        .one(&tx)
        .await?
        .map(|setting| setting.value == "true")
        .unwrap_or(true);
    let invite_code = data.invite_code.as_deref().filter(|code| !code.is_empty());
    if !enabled && invite_code.is_none() {
        return Err(AppError::Forbidden(
            "Open registration is disabled".to_string(),
        ));
    }
    let invitation = if let Some(code) = invite_code {
        let invitation = Invite::find()
            .filter(invite::Column::Code.eq(code))
            .one(&tx)
            .await?
            .ok_or_else(|| AppError::BadRequest("Invalid or used invite".to_string()))?;
        if invitation.is_used || invitation.expires_at.is_some_and(|date| date <= Utc::now()) {
            return Err(AppError::BadRequest("Invalid or used invite".to_string()));
        }
        Some(invitation)
    } else {
        None
    };
    if User::find()
        .filter(user::Column::Username.eq(username))
        .one(&tx)
        .await?
        .is_some()
        || User::find()
            .filter(user::Column::Email.eq(email))
            .one(&tx)
            .await?
            .is_some()
    {
        return Err(AppError::BadRequest("Account already exists".to_string()));
    }
    let now = Utc::now();
    let created = user::ActiveModel {
        username: Set(username.to_string()),
        email: Set(email.to_string()),
        hashed_password: Set(hashed_password),
        is_active: Set(true),
        is_approved: Set(invitation.is_some() || !approval),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&tx)
    .await?;
    if let Some(invitation) = invitation {
        let updated = Invite::update_many()
            .col_expr(
                invite::Column::IsUsed,
                sea_orm::sea_query::Expr::value(true),
            )
            .col_expr(
                invite::Column::UsedById,
                sea_orm::sea_query::Expr::value(created.id),
            )
            .col_expr(invite::Column::UsedAt, sea_orm::sea_query::Expr::value(now))
            .filter(invite::Column::Id.eq(invitation.id))
            .filter(invite::Column::IsUsed.eq(false))
            .exec(&tx)
            .await?;
        if updated.rows_affected != 1 {
            return Err(AppError::BadRequest("Invalid or used invite".to_string()));
        }
        crate::services::audit::log_on_transaction(
            &tx,
            AuditAction::InviteUsed,
            ResourceType::Invite,
            Some(invitation.id.to_string()),
            Some(created.id),
            Some(username.to_string()),
            None,
            None,
            None,
            true,
            None,
        )
        .await?;
    }
    crate::services::audit::log_on_transaction(
        &tx,
        AuditAction::UserCreated,
        ResourceType::User,
        Some(created.id.to_string()),
        Some(created.id),
        Some(username.to_string()),
        Some(serde_json::json!({"method": "registration"})),
        None,
        None,
        true,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        serde_json::json!({"status": if created.is_approved {"approved"} else {"pending"}}),
    ))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LoginResponse {
    pub user_id: i64,
    pub username: String,
    pub email: String,
    pub session_slot: usize,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AccountInfo {
    pub slot: usize,
    pub user_id: i64,
    pub username: String,
    pub email: String,
    pub is_active: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SessionInfo {
    pub id: String,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
    pub created_at: String,
    pub last_accessed_at: String,
    pub is_current: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RecoveryLoginRequest {
    pub username: String,
    pub password: String,
    pub recovery_code: String,
}

// ============================================================================
// Session Cookie Helpers
// ============================================================================

/// Optional `; Domain=...` cookie attribute from KUBARR_COOKIE_DOMAIN.
/// Setting it to the parent domain (e.g. `example.com`) lets the session
/// cookie flow to subdomain-routed apps (`watch.example.com`); unset keeps
/// host-only cookies.
fn cookie_domain_attribute() -> &'static str {
    static ATTR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ATTR.get_or_init(|| {
        std::env::var("KUBARR_COOKIE_DOMAIN")
            .ok()
            .map(|d| d.trim().trim_start_matches('.').to_string())
            .filter(|d| !d.is_empty())
            .map(|d| format!("; Domain={}", d))
            .unwrap_or_default()
    })
}

// A configured parent cookie domain is invalid on an unrelated exact-host
// assignment. Issue host-only cookies there; sibling hosts keep the shared
// parent-domain cookie behavior.
fn cookie_domain_for_host(host: &str) -> &'static str {
    let attribute = cookie_domain_attribute();
    let Some(domain) = attribute.strip_prefix("; Domain=") else {
        return "";
    };
    if host_accepts_cookie_domain(host, domain) {
        attribute
    } else {
        ""
    }
}

fn host_accepts_cookie_domain(host: &str, domain: &str) -> bool {
    host.eq_ignore_ascii_case(domain)
        || host
            .to_ascii_lowercase()
            .ends_with(&format!(".{}", domain.to_ascii_lowercase()))
}

/// Create an indexed session cookie with the given token
fn create_session_cookie_for_slot(slot: usize, token: &str, secure: bool) -> HeaderValue {
    session_cookie_for_slot(slot, token, secure, cookie_domain_attribute())
}

fn session_cookie_for_slot(slot: usize, token: &str, secure: bool, domain: &str) -> HeaderValue {
    let cookie = format!(
        "{}_{}={}; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800{}{}",
        SESSION_COOKIE_BASE,
        slot,
        token,
        domain,
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&cookie).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Create the active session cookie
fn create_active_session_cookie(slot: usize, secure: bool) -> HeaderValue {
    active_session_cookie(slot, secure, cookie_domain_attribute())
}

fn active_session_cookie(slot: usize, secure: bool, domain: &str) -> HeaderValue {
    let cookie = format!(
        "{}={}; SameSite=Lax; Path=/; Max-Age=604800{}{}",
        ACTIVE_SESSION_COOKIE,
        slot,
        domain,
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&cookie).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Legacy: Create a session cookie with the given token (for backwards compatibility)
fn create_session_cookie(token: &str, secure: bool) -> HeaderValue {
    session_cookie(token, secure, cookie_domain_attribute())
}

fn session_cookie(token: &str, secure: bool, domain: &str) -> HeaderValue {
    let cookie = format!(
        "{}={}; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800{}{}",
        SESSION_COOKIE_NAME,
        token,
        domain,
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&cookie).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Create a cookie that clears the session
fn clear_session_cookie() -> HeaderValue {
    let cookie = format!(
        "{}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0{}",
        SESSION_COOKIE_NAME,
        cookie_domain_attribute()
    );
    HeaderValue::from_str(&cookie).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Parse existing session cookies from headers to find used slots and their user IDs
async fn get_existing_sessions(state: &AppState, headers: &HeaderMap) -> Vec<(usize, i64, String)> {
    let mut sessions = Vec::new();

    let db = match state.get_db().await {
        Ok(db) => db,
        Err(_) => return sessions,
    };

    let cookies = match headers.get(header::COOKIE) {
        Some(c) => c,
        None => return sessions,
    };
    let cookie_str = match cookies.to_str() {
        Ok(s) => s,
        Err(_) => return sessions,
    };

    for i in 0..MAX_SESSIONS {
        let prefix = format!("{}_{}=", SESSION_COOKIE_BASE, i);
        for cookie in cookie_str.split(';') {
            let cookie = cookie.trim();
            if let Some(token) = cookie.strip_prefix(&prefix) {
                // Decode token to get session ID, then look up user
                if let Ok(claims) = decode_session_token(token) {
                    if let Ok(Some(session)) = Session::find_by_id(&claims.sid).one(&db).await {
                        if !session.is_revoked && session.expires_at > Utc::now() {
                            if let Ok(Some(user)) = User::find_by_id(session.user_id).one(&db).await
                            {
                                sessions.push((i, user.id, user.username.clone()));
                            }
                        }
                    }
                }
                break;
            }
        }
    }

    sessions
}

/// Find the next available session slot
fn find_available_slot(existing: &[(usize, i64, String)], user_id: i64) -> usize {
    // If user already has a session, return that slot
    for (slot, uid, _) in existing {
        if *uid == user_id {
            return *slot;
        }
    }
    // Find first unused slot
    let used_slots: std::collections::HashSet<usize> =
        existing.iter().map(|(s, _, _)| *s).collect();
    for i in 0..MAX_SESSIONS {
        if !used_slots.contains(&i) {
            return i;
        }
    }
    // All slots used, reuse slot 0
    0
}

// ============================================================================
// Session Management Endpoints
// ============================================================================

/// Login with username and password, returns session cookie
#[utoipa::path(
    post,
    path = "/auth/login",
    tag = "Auth",
    request_body = LoginRequest,
    responses(
        (status = 200, body = LoginResponse)
    )
)]
async fn login(
    State(state): State<AppState>,
    peer: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> Result<Response> {
    let db = state.get_db().await?;
    let (ip_address, user_agent) = audit_context(&headers, peer.map(|Extension(info)| info));

    // Find user by username or email
    let found_user = User::find()
        .filter(
            user::Column::Username
                .eq(&request.username)
                .or(user::Column::Email.eq(&request.username)),
        )
        .one(&db)
        .await?;
    let Some(found_user) = found_user else {
        audit_failure(
            &state,
            AuditAction::LoginFailed,
            None,
            "invalid_credentials",
            &ip_address,
            &user_agent,
        )
        .await;
        return Err(AppError::Unauthorized("Invalid credentials".to_string()));
    };

    // Check if user is active and approved
    if !found_user.is_active {
        audit_failure(
            &state,
            AuditAction::LoginFailed,
            Some(&found_user),
            "account_disabled",
            &ip_address,
            &user_agent,
        )
        .await;
        return Err(AppError::Unauthorized("Account is disabled".to_string()));
    }
    if !found_user.is_approved {
        audit_failure(
            &state,
            AuditAction::LoginFailed,
            Some(&found_user),
            "approval_required",
            &ip_address,
            &user_agent,
        )
        .await;
        return Err(AppError::Unauthorized(
            "Account is pending approval".to_string(),
        ));
    }

    // Verify password
    if !verify_password(&request.password, &found_user.hashed_password) {
        audit_failure(
            &state,
            AuditAction::LoginFailed,
            Some(&found_user),
            "invalid_credentials",
            &ip_address,
            &user_agent,
        )
        .await;
        return Err(AppError::Unauthorized("Invalid credentials".to_string()));
    }

    // If role requires 2FA but user hasn't set it up, block login
    if !found_user.totp_enabled && role_requires_2fa(&db, found_user.id).await {
        audit_failure(
            &state,
            AuditAction::TwoFactorFailed,
            Some(&found_user),
            "setup_required",
            &ip_address,
            &user_agent,
        )
        .await;
        return Err(AppError::BadRequest(
            "Two-factor authentication setup required".to_string(),
        ));
    }

    // Check TOTP if enabled
    if found_user.totp_enabled {
        let Some(totp_code) = request.totp_code.as_ref() else {
            audit_failure(
                &state,
                AuditAction::TwoFactorFailed,
                Some(&found_user),
                "totp_required",
                &ip_address,
                &user_agent,
            )
            .await;
            return Err(AppError::BadRequest(
                "Two-factor authentication code required".to_string(),
            ));
        };

        let totp_secret = found_user.totp_secret.as_ref().ok_or_else(|| {
            AppError::Internal("TOTP enabled but no secret configured".to_string())
        })?;

        if !verify_totp(totp_secret, totp_code, &found_user.email)? {
            audit_failure(
                &state,
                AuditAction::TwoFactorFailed,
                Some(&found_user),
                "invalid_totp",
                &ip_address,
                &user_agent,
            )
            .await;
            return Err(AppError::Unauthorized("Invalid TOTP code".to_string()));
        }
    }

    // Create session record in database
    let session_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires_at = now + Duration::days(7);

    let session = session::ActiveModel {
        id: Set(session_id.clone()),
        user_id: Set(found_user.id),
        user_agent: Set(user_agent.clone()),
        ip_address: Set(ip_address.clone()),
        created_at: Set(now),
        expires_at: Set(expires_at),
        last_accessed_at: Set(now),
        is_revoked: Set(false),
    };
    // Generate the token before committing the new session.
    let session_token = create_session_token(&session_id)?;
    let tx = db.begin().await?;
    session.insert(&tx).await?;
    if found_user.totp_enabled {
        audit_success(
            &tx,
            AuditAction::TwoFactorVerified,
            found_user.id,
            &found_user.username,
            &ip_address,
            &user_agent,
            Some("totp"),
        )
        .await?;
    }
    audit_success(
        &tx,
        AuditAction::Login,
        found_user.id,
        &found_user.username,
        &ip_address,
        &user_agent,
        None,
    )
    .await?;
    tx.commit().await?;

    // Find available slot for this session
    let existing_sessions = get_existing_sessions(&state, &headers).await;
    let slot = find_available_slot(&existing_sessions, found_user.id);

    // Build response with cookie
    let response = Json(LoginResponse {
        user_id: found_user.id,
        username: found_user.username.clone(),
        email: found_user.email.clone(),
        session_slot: slot,
    });

    // Determine if we should set Secure flag (check if running behind HTTPS)
    let secure = CONFIG.auth.oauth2_issuer_url.starts_with("https://");

    tracing::info!(
        user_id = found_user.id,
        username = found_user.username,
        slot = slot,
        "User logged in, session created in slot {}",
        slot
    );

    // Set both the indexed session cookie and the active session cookie
    let mut response_headers = axum::http::HeaderMap::new();
    response_headers.insert(
        header::SET_COOKIE,
        create_session_cookie_for_slot(slot, &session_token, secure),
    );
    response_headers.append(
        header::SET_COOKIE,
        create_active_session_cookie(slot, secure),
    );
    // Also set legacy cookie for backwards compatibility
    response_headers.append(
        header::SET_COOKIE,
        create_session_cookie(&session_token, secure),
    );

    Ok((response_headers, response).into_response())
}

/// Logout - revokes the session and clears the cookie
#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = "Auth",
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn logout(
    State(state): State<AppState>,
    peer: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
) -> Result<Response> {
    // Try to get and revoke the current session
    if let Some(token) = extract_session_token(&headers) {
        if let Ok(claims) = decode_session_token(&token) {
            // Revoke the session in the database
            if let Ok(db) = state.get_db().await {
                let current = Session::find_by_id(&claims.sid).one(&db).await?;
                if let Some(current) =
                    current.filter(|s| !s.is_revoked && s.expires_at > Utc::now())
                {
                    let actor = User::find_by_id(current.user_id).one(&db).await?;
                    if let Some(actor) = actor {
                        let (ip, agent) = audit_context(&headers, peer.map(|Extension(info)| info));
                        let tx = db.begin().await?;
                        Session::delete_by_id(&claims.sid).exec(&tx).await?;
                        audit_success(
                            &tx,
                            AuditAction::Logout,
                            actor.id,
                            &actor.username,
                            &ip,
                            &agent,
                            None,
                        )
                        .await?;
                        tx.commit().await?;
                    }
                }
            }
        }
    }

    Ok((
        [(header::SET_COOKIE, clear_session_cookie())],
        Json(serde_json::json!({"message": "Logged out"})),
    )
        .into_response())
}

/// List all active sessions for the current user
#[utoipa::path(
    get,
    path = "/auth/sessions",
    tag = "Auth",
    responses(
        (status = 200, body = Vec<SessionInfo>)
    )
)]
async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<SessionInfo>>> {
    let db = state.get_db().await?;

    // Get current session from cookie
    let token = extract_session_token(&headers)
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))?;

    let claims = decode_session_token(&token)
        .map_err(|_| AppError::Unauthorized("Invalid or expired session".to_string()))?;

    // Get the current session to find user_id
    let current_session = Session::find_by_id(&claims.sid)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Session not found".to_string()))?;

    // Get all active sessions for this user
    let sessions = Session::find()
        .filter(session::Column::UserId.eq(current_session.user_id))
        .filter(session::Column::IsRevoked.eq(false))
        .filter(session::Column::ExpiresAt.gt(Utc::now()))
        .all(&db)
        .await?;

    let session_infos: Vec<SessionInfo> = sessions
        .into_iter()
        .map(|s| SessionInfo {
            id: s.id.clone(),
            user_agent: s.user_agent,
            ip_address: s.ip_address,
            created_at: s.created_at.to_rfc3339(),
            last_accessed_at: s.last_accessed_at.to_rfc3339(),
            is_current: s.id == claims.sid,
        })
        .collect();

    Ok(Json(session_infos))
}

/// Revoke a specific session (must belong to current user)
#[utoipa::path(
    delete,
    path = "/auth/sessions/{session_id}",
    tag = "Auth",
    params(
        ("session_id" = String, Path, description = "Session ID to revoke")
    ),
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn revoke_session(
    State(state): State<AppState>,
    peer: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    let db = state.get_db().await?;

    // Get current session from cookie
    let token = extract_session_token(&headers)
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".to_string()))?;

    let claims = decode_session_token(&token)
        .map_err(|_| AppError::Unauthorized("Invalid or expired session".to_string()))?;

    // Get the current session to find user_id
    let current_session = Session::find_by_id(&claims.sid)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Session not found".to_string()))?;

    // Find the session to revoke
    let target_session = Session::find_by_id(&session_id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound("Session not found".to_string()))?;

    // Verify it belongs to the same user
    if target_session.user_id != current_session.user_id {
        return Err(AppError::Forbidden(
            "Cannot revoke another user's session".to_string(),
        ));
    }

    // Don't allow revoking current session (use logout instead)
    if target_session.id == claims.sid {
        return Err(AppError::BadRequest(
            "Cannot revoke current session. Use logout instead.".to_string(),
        ));
    }

    // The target ID is a secret: record only the owning user and a fixed
    // operation label. Commit the deletion and its audit row atomically.
    let (ip, agent) = audit_context(&headers, peer.map(|Extension(info)| info));
    let actor = User::find_by_id(current_session.user_id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::Unauthorized("User not found".to_string()))?;
    let tx = db.begin().await?;
    Session::delete_by_id(&session_id).exec(&tx).await?;
    crate::services::audit::log_on_transaction(
        &tx,
        AuditAction::Logout,
        ResourceType::Session,
        Some(actor.id.to_string()),
        Some(actor.id),
        Some(actor.username),
        Some(serde_json::json!({"operation": "session_revoked", "target_user_id": target_session.user_id})),
        ip,
        agent,
        true,
        None,
    )
    .await?;
    tx.commit().await?;

    tracing::info!(user_id = current_session.user_id, "Session revoked by user");

    Ok(Json(serde_json::json!({"message": "Session revoked"})))
}

/// Check if any of a user's roles require 2FA
async fn role_requires_2fa(db: &sea_orm::DatabaseConnection, user_id: i64) -> bool {
    let roles: Vec<role::Model> = Role::find()
        .inner_join(UserRole)
        .filter(user_role::Column::UserId.eq(user_id))
        .all(db)
        .await
        .unwrap_or_default();

    roles.iter().any(|r| r.requires_2fa)
}

/// Login using a recovery code instead of a TOTP code
#[utoipa::path(
    post,
    path = "/auth/2fa/recover",
    tag = "Auth",
    request_body = RecoveryLoginRequest,
    responses(
        (status = 200, body = LoginResponse)
    )
)]
async fn recover_with_code(
    State(state): State<AppState>,
    peer: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    Json(request): Json<RecoveryLoginRequest>,
) -> Result<Response> {
    let db = state.get_db().await?;
    let (ip_address, user_agent) = audit_context(&headers, peer.map(|Extension(info)| info));

    // Find user by username or email
    let found_user = User::find()
        .filter(
            user::Column::Username
                .eq(&request.username)
                .or(user::Column::Email.eq(&request.username)),
        )
        .one(&db)
        .await?;
    let Some(found_user) = found_user else {
        audit_failure(
            &state,
            AuditAction::LoginFailed,
            None,
            "invalid_credentials",
            &ip_address,
            &user_agent,
        )
        .await;
        return Err(AppError::Unauthorized("Invalid credentials".to_string()));
    };

    if !found_user.is_active {
        audit_failure(
            &state,
            AuditAction::LoginFailed,
            Some(&found_user),
            "account_disabled",
            &ip_address,
            &user_agent,
        )
        .await;
        return Err(AppError::Unauthorized("Account is disabled".to_string()));
    }
    if !found_user.is_approved {
        audit_failure(
            &state,
            AuditAction::LoginFailed,
            Some(&found_user),
            "approval_required",
            &ip_address,
            &user_agent,
        )
        .await;
        return Err(AppError::Unauthorized(
            "Account is pending approval".to_string(),
        ));
    }

    // Verify password
    if !verify_password(&request.password, &found_user.hashed_password) {
        audit_failure(
            &state,
            AuditAction::LoginFailed,
            Some(&found_user),
            "invalid_credentials",
            &ip_address,
            &user_agent,
        )
        .await;
        return Err(AppError::Unauthorized("Invalid credentials".to_string()));
    }

    // Recovery only applies when 2FA is enabled
    if !found_user.totp_enabled {
        audit_failure(
            &state,
            AuditAction::TwoFactorFailed,
            Some(&found_user),
            "setup_required",
            &ip_address,
            &user_agent,
        )
        .await;
        return Err(AppError::BadRequest(
            "Two-factor authentication is not enabled for this account".to_string(),
        ));
    }

    let user_id = found_user.id;
    let username = found_user.username.clone();
    let email = found_user.email.clone();

    // Find all unused recovery codes for this user
    let recovery_codes = TwoFactorRecoveryCode::find()
        .filter(two_factor_recovery_code::Column::UserId.eq(user_id))
        .filter(two_factor_recovery_code::Column::UsedAt.is_null())
        .all(&db)
        .await?;

    // Find a matching code
    let matching = recovery_codes
        .iter()
        .find(|rc| verify_recovery_code(&request.recovery_code, &rc.code_hash));

    let Some(matched_code) = matching.cloned() else {
        audit_failure(
            &state,
            AuditAction::TwoFactorFailed,
            Some(&found_user),
            "invalid_totp",
            &ip_address,
            &user_agent,
        )
        .await;
        return Err(AppError::Unauthorized("Invalid recovery code".to_string()));
    };

    let session_id = uuid::Uuid::new_v4().to_string();
    let session_token = create_session_token(&session_id)?;
    let tx = db.begin().await?;

    // Mark the code as used
    let now = Utc::now();
    let mut code_model: two_factor_recovery_code::ActiveModel = matched_code.into();
    code_model.used_at = Set(Some(now));
    code_model.update(&tx).await?;

    // Check how many unused codes remain
    let remaining = TwoFactorRecoveryCode::find()
        .filter(two_factor_recovery_code::Column::UserId.eq(user_id))
        .filter(two_factor_recovery_code::Column::UsedAt.is_null())
        .count(&tx)
        .await?;

    if remaining == 0 {
        // All codes exhausted — disable 2FA so user can re-enable with a new authenticator
        let mut user_model: user::ActiveModel = found_user.into();
        user_model.totp_enabled = Set(false);
        user_model.totp_secret = Set(None);
        user_model.totp_verified_at = Set(None);
        user_model.updated_at = Set(now);
        user_model.update(&tx).await?;

        // Clean up all (used) recovery codes
        TwoFactorRecoveryCode::delete_many()
            .filter(two_factor_recovery_code::Column::UserId.eq(user_id))
            .exec(&tx)
            .await?;

        tracing::info!(
            user_id = user_id,
            "All recovery codes exhausted; 2FA disabled for user"
        );
    }

    // Create session record in database
    let expires_at = now + chrono::Duration::days(7);

    let session = session::ActiveModel {
        id: Set(session_id.clone()),
        user_id: Set(user_id),
        user_agent: Set(user_agent.clone()),
        ip_address: Set(ip_address.clone()),
        created_at: Set(now),
        expires_at: Set(expires_at),
        last_accessed_at: Set(now),
        is_revoked: Set(false),
    };
    session.insert(&tx).await?;
    // Consumption, verification event, login event and the new session all
    // commit together; failure to audit must not spend a recovery code.
    audit_success(
        &tx,
        AuditAction::TwoFactorVerified,
        user_id,
        &username,
        &ip_address,
        &user_agent,
        Some("recovery_code"),
    )
    .await?;
    audit_success(
        &tx,
        AuditAction::Login,
        user_id,
        &username,
        &ip_address,
        &user_agent,
        Some("recovery_code"),
    )
    .await?;
    tx.commit().await?;

    let existing_sessions = get_existing_sessions(&state, &headers).await;
    let slot = find_available_slot(&existing_sessions, user_id);

    let response = Json(LoginResponse {
        user_id,
        username: username.clone(),
        email,
        session_slot: slot,
    });

    let secure = CONFIG.auth.oauth2_issuer_url.starts_with("https://");

    tracing::info!(
        user_id = user_id,
        username = username,
        slot = slot,
        "User logged in via recovery code, session created in slot {}",
        slot
    );

    let mut response_headers = axum::http::HeaderMap::new();
    response_headers.insert(
        header::SET_COOKIE,
        create_session_cookie_for_slot(slot, &session_token, secure),
    );
    response_headers.append(
        header::SET_COOKIE,
        create_active_session_cookie(slot, secure),
    );
    response_headers.append(
        header::SET_COOKIE,
        create_session_cookie(&session_token, secure),
    );

    Ok((response_headers, response).into_response())
}

/// Extract session token from cookie header
fn extract_session_token(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?;
    let cookie_str = cookies.to_str().ok()?;

    // First try to find active slot
    let mut active_slot: Option<usize> = None;
    for cookie in cookie_str.split(';') {
        let cookie = cookie.trim();
        if let Some(value) = cookie.strip_prefix(&format!("{}=", ACTIVE_SESSION_COOKIE)) {
            active_slot = value.parse().ok();
            break;
        }
    }

    // Look for indexed session cookie
    if let Some(slot) = active_slot {
        let prefix = format!("{}_{}=", SESSION_COOKIE_BASE, slot);
        for cookie in cookie_str.split(';') {
            let cookie = cookie.trim();
            if let Some(value) = cookie.strip_prefix(&prefix) {
                return Some(value.to_string());
            }
        }
    }

    // Fallback to legacy cookie
    for cookie in cookie_str.split(';') {
        let cookie = cookie.trim();
        if let Some(value) = cookie.strip_prefix(&format!("{}=", SESSION_COOKIE_NAME)) {
            return Some(value.to_string());
        }
    }
    None
}

/// Switch to a different session slot
#[utoipa::path(
    post,
    path = "/auth/sessions/{session_id}/switch",
    tag = "Auth",
    params(
        ("session_id" = usize, Path, description = "Session slot to switch to")
    ),
    responses(
        (status = 200, body = LoginResponse)
    )
)]
async fn switch_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slot): Path<usize>,
) -> Result<Response> {
    if slot >= MAX_SESSIONS {
        return Err(AppError::BadRequest(format!(
            "Invalid session slot. Max is {}",
            MAX_SESSIONS - 1
        )));
    }

    // Verify that the requested slot has a valid session
    let existing_sessions = get_existing_sessions(&state, &headers).await;
    let slot_exists = existing_sessions.iter().any(|(s, _, _)| *s == slot);

    if !slot_exists {
        return Err(AppError::NotFound("No session in that slot".to_string()));
    }

    let secure = CONFIG.auth.oauth2_issuer_url.starts_with("https://");

    tracing::info!(slot = slot, "User switched to session slot {}", slot);

    Ok((
        [(
            header::SET_COOKIE,
            create_active_session_cookie(slot, secure),
        )],
        Json(serde_json::json!({"message": "Switched session", "slot": slot})),
    )
        .into_response())
}

/// List all signed-in accounts
#[utoipa::path(
    get,
    path = "/auth/accounts",
    tag = "Auth",
    responses(
        (status = 200, body = Vec<AccountInfo>)
    )
)]
async fn list_accounts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AccountInfo>>> {
    let db = state.get_db().await?;
    let existing_sessions = get_existing_sessions(&state, &headers).await;

    // Get active slot
    let cookies = headers.get(header::COOKIE);
    let active_slot: usize = cookies
        .and_then(|c| c.to_str().ok())
        .and_then(|s| {
            for cookie in s.split(';') {
                let cookie = cookie.trim();
                if let Some(value) = cookie.strip_prefix(&format!("{}=", ACTIVE_SESSION_COOKIE)) {
                    return value.parse().ok();
                }
            }
            None
        })
        .unwrap_or(0);

    let mut accounts = Vec::new();
    for (slot, user_id, username) in &existing_sessions {
        // Get full user info
        if let Ok(Some(user)) = User::find_by_id(*user_id).one(&db).await {
            accounts.push(AccountInfo {
                slot: *slot,
                user_id: *user_id,
                username: username.clone(),
                email: user.email,
                is_active: *slot == active_slot,
            });
        }
    }

    Ok(Json(accounts))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // create_session_cookie_for_slot
    // -------------------------------------------------------------------------

    #[test]
    fn session_cookie_for_slot_format() {
        let hv = create_session_cookie_for_slot(0, "tok123", false);
        let s = hv.to_str().unwrap();
        assert!(s.contains("kubarr_session_0=tok123"));
        assert!(s.contains("HttpOnly"));
        assert!(s.contains("Max-Age=604800"));
        assert!(
            !s.contains("Secure"),
            "must NOT contain Secure when secure=false"
        );
    }

    #[test]
    fn session_cookie_for_slot_secure_flag() {
        let hv = create_session_cookie_for_slot(2, "abc", true);
        let s = hv.to_str().unwrap();
        assert!(s.contains("Secure"));
    }

    #[test]
    fn session_cookie_for_slot_different_slots() {
        let hv1 = create_session_cookie_for_slot(1, "t1", false);
        let hv3 = create_session_cookie_for_slot(3, "t3", false);
        assert!(hv1.to_str().unwrap().contains("kubarr_session_1=t1"));
        assert!(hv3.to_str().unwrap().contains("kubarr_session_3=t3"));
    }

    // -------------------------------------------------------------------------
    // create_active_session_cookie
    // -------------------------------------------------------------------------

    #[test]
    fn active_session_cookie_format() {
        let hv = create_active_session_cookie(0, false);
        let s = hv.to_str().unwrap();
        assert!(s.contains("=0"));
        assert!(s.contains("Max-Age=604800"));
        assert!(
            !s.contains("HttpOnly"),
            "active session cookie is NOT HttpOnly"
        );
        assert!(!s.contains("Secure"));
    }

    #[test]
    fn active_session_cookie_secure() {
        let hv = create_active_session_cookie(1, true);
        assert!(hv.to_str().unwrap().contains("Secure"));
    }

    // -------------------------------------------------------------------------
    // create_session_cookie (legacy)
    // -------------------------------------------------------------------------

    #[test]
    fn legacy_session_cookie_format() {
        let hv = create_session_cookie("mytoken", false);
        let s = hv.to_str().unwrap();
        assert!(s.contains("kubarr_session=mytoken"));
        assert!(s.contains("HttpOnly"));
    }

    #[test]
    fn legacy_session_cookie_secure() {
        let hv = create_session_cookie("tok", true);
        assert!(hv.to_str().unwrap().contains("Secure"));
    }

    // -------------------------------------------------------------------------
    // clear_session_cookie (legacy)
    // -------------------------------------------------------------------------

    #[test]
    fn clear_legacy_cookie_sets_max_age_zero() {
        let hv = clear_session_cookie();
        let s = hv.to_str().unwrap();
        assert!(s.contains("Max-Age=0"));
        assert!(s.contains("kubarr_session="));
    }

    // -------------------------------------------------------------------------
    // find_available_slot
    // -------------------------------------------------------------------------

    #[test]
    fn find_slot_empty_returns_zero() {
        let result = find_available_slot(&[], 42);
        assert_eq!(result, 0);
    }

    #[test]
    fn find_slot_user_already_has_slot() {
        let existing = vec![(1, 42_i64, "alice".to_string())];
        // user 42 already has slot 1 → returns 1
        assert_eq!(find_available_slot(&existing, 42), 1);
    }

    #[test]
    fn find_slot_finds_first_unused() {
        // slot 0 is used by user 10
        let existing = vec![(0, 10_i64, "bob".to_string())];
        // user 99 gets slot 1 (first unused)
        assert_eq!(find_available_slot(&existing, 99), 1);
    }

    #[test]
    fn find_slot_multiple_used_finds_next() {
        let existing = vec![
            (0, 1_i64, "u0".to_string()),
            (1, 2_i64, "u1".to_string()),
            (2, 3_i64, "u2".to_string()),
        ];
        // slots 0,1,2 used → slot 3 is next
        assert_eq!(find_available_slot(&existing, 99), 3);
    }

    #[test]
    fn find_slot_all_used_returns_zero() {
        let existing: Vec<(usize, i64, String)> = (0..MAX_SESSIONS)
            .map(|i| (i, i as i64 + 100, format!("u{}", i)))
            .collect();
        // All slots used → falls back to slot 0
        assert_eq!(find_available_slot(&existing, 999), 0);
    }

    // -------------------------------------------------------------------------
    // Serde tests for request/response types
    // -------------------------------------------------------------------------

    #[test]
    fn login_request_deser() {
        let r: LoginRequest = serde_json::from_str(
            r#"{"username":"admin","password":"secret","totp_code":"123456"}"#,
        )
        .expect("deser");
        assert_eq!(r.username, "admin");
        assert_eq!(r.password, "secret");
        assert_eq!(r.totp_code, Some("123456".to_string()));
    }

    #[test]
    fn login_request_deser_no_totp() {
        let r: LoginRequest =
            serde_json::from_str(r#"{"username":"admin","password":"secret"}"#).expect("deser");
        assert_eq!(r.totp_code, None);
    }

    #[test]
    fn login_response_ser() {
        let r = LoginResponse {
            user_id: 1,
            username: "admin".to_string(),
            email: "admin@example.com".to_string(),
            session_slot: 0,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"user_id\":1"));
        assert!(json.contains("\"username\":\"admin\""));
        assert!(json.contains("\"session_slot\":0"));
    }

    #[test]
    fn account_info_ser() {
        let a = AccountInfo {
            slot: 0,
            user_id: 5,
            username: "alice".to_string(),
            email: "alice@example.com".to_string(),
            is_active: true,
        };
        let json = serde_json::to_string(&a).expect("ser");
        assert!(json.contains("\"slot\":0"));
        assert!(json.contains("\"user_id\":5"));
        assert!(json.contains("\"is_active\":true"));
    }

    #[test]
    fn session_info_ser() {
        let s = SessionInfo {
            id: "sess-123".to_string(),
            user_agent: Some("Mozilla/5.0".to_string()),
            ip_address: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            last_accessed_at: "2026-01-02T00:00:00Z".to_string(),
            is_current: true,
        };
        let json = serde_json::to_string(&s).expect("ser");
        assert!(json.contains("\"id\":\"sess-123\""));
        assert!(json.contains("\"is_current\":true"));
        assert!(json.contains("\"ip_address\":null"));
    }

    #[test]
    fn recovery_login_request_deser() {
        let r: RecoveryLoginRequest = serde_json::from_str(
            r#"{"username":"admin","password":"secret","recovery_code":"ABCDEF1234"}"#,
        )
        .expect("deser");
        assert_eq!(r.username, "admin");
        assert_eq!(r.recovery_code, "ABCDEF1234");
    }
}
