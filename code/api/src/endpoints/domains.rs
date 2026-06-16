//! Domain, Dynamic DNS, and Let's Encrypt configuration endpoints.

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, Result};
use crate::middleware::permissions::{Authorized, SettingsManage, SettingsView};
use crate::models::prelude::*;
use crate::models::{app_domain_assignment, domain, dynamic_dns_profile, letsencrypt_profile};
use crate::state::AppState;

pub fn domains_routes(state: AppState) -> Router {
    Router::new()
        .route("/", get(list_domains).post(create_domain))
        .route(
            "/ddns-profiles",
            get(list_ddns_profiles).post(create_ddns_profile),
        )
        .route(
            "/ddns-profiles/{id}",
            get(get_ddns_profile)
                .put(update_ddns_profile)
                .delete(delete_ddns_profile),
        )
        .route(
            "/letsencrypt-profiles",
            get(list_letsencrypt_profiles).post(create_letsencrypt_profile),
        )
        .route(
            "/letsencrypt-profiles/{id}",
            get(get_letsencrypt_profile)
                .put(update_letsencrypt_profile)
                .delete(delete_letsencrypt_profile),
        )
        .route(
            "/assignments",
            get(list_assignments).post(create_assignment),
        )
        .route(
            "/assignments/{id}",
            get(get_assignment)
                .put(update_assignment)
                .delete(delete_assignment),
        )
        .route(
            "/{id}",
            get(get_domain).put(update_domain).delete(delete_domain),
        )
        .with_state(state)
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DynamicDnsProfileResponse {
    pub id: i64,
    pub name: String,
    pub provider: String,
    pub capabilities: Value,
    pub config: Value,
    pub enabled: bool,
    pub status: String,
    pub last_error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct DynamicDnsProfileRequest {
    pub name: String,
    pub provider: String,
    pub capabilities: Option<Value>,
    pub config: Option<Value>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LetsEncryptProfileResponse {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub environment: String,
    pub challenge_type: String,
    pub dns_profile_id: Option<i64>,
    pub renewal_enabled: bool,
    pub enabled: bool,
    pub status: String,
    pub last_error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LetsEncryptProfileRequest {
    pub name: String,
    pub email: String,
    pub environment: String,
    pub challenge_type: String,
    pub dns_profile_id: Option<i64>,
    pub renewal_enabled: Option<bool>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DomainResponse {
    pub id: i64,
    pub domain: String,
    pub kind: String,
    pub scope: String,
    pub primary: bool,
    pub enabled: bool,
    pub dns_mode: String,
    pub ddns_profile_id: Option<i64>,
    pub dns_status: String,
    pub tls_mode: String,
    pub letsencrypt_profile_id: Option<i64>,
    pub tls_secret_name: Option<String>,
    pub certificate_status: String,
    pub certificate_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct DomainRequest {
    pub domain: String,
    pub kind: String,
    pub scope: String,
    pub primary: Option<bool>,
    pub enabled: Option<bool>,
    pub dns_mode: String,
    pub ddns_profile_id: Option<i64>,
    pub tls_mode: String,
    pub letsencrypt_profile_id: Option<i64>,
    pub tls_secret_name: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AppDomainAssignmentResponse {
    pub id: i64,
    pub app_name: String,
    pub domain_id: i64,
    pub route_mode: String,
    pub hostname: Option<String>,
    pub path_prefix: Option<String>,
    pub primary: bool,
    pub enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AppDomainAssignmentRequest {
    pub app_name: String,
    pub domain_id: i64,
    pub route_mode: String,
    pub hostname: Option<String>,
    pub path_prefix: Option<String>,
    pub primary: Option<bool>,
    pub enabled: Option<bool>,
}

#[utoipa::path(get, path = "/api/domains/ddns-profiles", tag = "Domains")]
pub async fn list_ddns_profiles(
    State(state): State<AppState>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<Vec<DynamicDnsProfileResponse>>> {
    let db = state.get_db().await?;
    let profiles = DynamicDnsProfile::find()
        .order_by_asc(dynamic_dns_profile::Column::Name)
        .all(&db)
        .await?
        .into_iter()
        .map(ddns_response)
        .collect::<Result<Vec<_>>>()?;
    Ok(Json(profiles))
}

#[utoipa::path(get, path = "/api/domains/ddns-profiles/{id}", tag = "Domains")]
pub async fn get_ddns_profile(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<DynamicDnsProfileResponse>> {
    let db = state.get_db().await?;
    let profile = DynamicDnsProfile::find_by_id(id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Dynamic DNS profile {} not found", id)))?;
    Ok(Json(ddns_response(profile)?))
}

#[utoipa::path(post, path = "/api/domains/ddns-profiles", tag = "Domains")]
pub async fn create_ddns_profile(
    State(state): State<AppState>,
    _auth: Authorized<SettingsManage>,
    Json(req): Json<DynamicDnsProfileRequest>,
) -> Result<Json<DynamicDnsProfileResponse>> {
    validate_required(&req.name, "Profile name")?;
    let now = Utc::now();
    let db = state.get_db().await?;
    let model = dynamic_dns_profile::ActiveModel {
        name: Set(req.name.trim().to_string()),
        provider: Set(req.provider.trim().to_string()),
        capabilities_json: Set(serde_json::to_string(
            &req.capabilities
                .unwrap_or_else(|| default_dns_capabilities(&req.provider)),
        )?),
        config_json: Set(serde_json::to_string(
            &req.config.unwrap_or_else(|| json!({})),
        )?),
        enabled: Set(req.enabled.unwrap_or(true)),
        status: Set("unknown".to_string()),
        last_error: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    Ok(Json(ddns_response(model)?))
}

#[utoipa::path(put, path = "/api/domains/ddns-profiles/{id}", tag = "Domains")]
pub async fn update_ddns_profile(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<SettingsManage>,
    Json(req): Json<DynamicDnsProfileRequest>,
) -> Result<Json<DynamicDnsProfileResponse>> {
    validate_required(&req.name, "Profile name")?;
    let db = state.get_db().await?;
    let existing = DynamicDnsProfile::find_by_id(id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Dynamic DNS profile {} not found", id)))?;
    let existing_config: Value =
        serde_json::from_str(&existing.config_json).unwrap_or_else(|_| json!({}));
    let mut active: dynamic_dns_profile::ActiveModel = existing.into();
    active.name = Set(req.name.trim().to_string());
    active.provider = Set(req.provider.trim().to_string());
    active.capabilities_json = Set(serde_json::to_string(
        &req.capabilities
            .unwrap_or_else(|| default_dns_capabilities(&req.provider)),
    )?);
    active.config_json = Set(serde_json::to_string(&merge_ddns_config_for_update(
        req.config.unwrap_or_else(|| json!({})),
        &existing_config,
    ))?);
    active.enabled = Set(req.enabled.unwrap_or(true));
    active.updated_at = Set(Utc::now());
    Ok(Json(ddns_response(active.update(&db).await?)?))
}

#[utoipa::path(delete, path = "/api/domains/ddns-profiles/{id}", tag = "Domains")]
pub async fn delete_ddns_profile(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<SettingsManage>,
) -> Result<Json<Value>> {
    let db = state.get_db().await?;
    if LetsEncryptProfile::find()
        .filter(letsencrypt_profile::Column::DnsProfileId.eq(id))
        .one(&db)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Dynamic DNS profile is used by a Let's Encrypt profile".to_string(),
        ));
    }
    DynamicDnsProfile::delete_by_id(id).exec(&db).await?;
    Ok(Json(json!({ "success": true })))
}

#[utoipa::path(get, path = "/api/domains/letsencrypt-profiles", tag = "Domains")]
pub async fn list_letsencrypt_profiles(
    State(state): State<AppState>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<Vec<LetsEncryptProfileResponse>>> {
    let db = state.get_db().await?;
    let profiles = LetsEncryptProfile::find()
        .order_by_asc(letsencrypt_profile::Column::Name)
        .all(&db)
        .await?
        .into_iter()
        .map(le_response)
        .collect();
    Ok(Json(profiles))
}

#[utoipa::path(get, path = "/api/domains/letsencrypt-profiles/{id}", tag = "Domains")]
pub async fn get_letsencrypt_profile(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<LetsEncryptProfileResponse>> {
    let db = state.get_db().await?;
    let profile = LetsEncryptProfile::find_by_id(id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Let's Encrypt profile {} not found", id)))?;
    Ok(Json(le_response(profile)))
}

#[utoipa::path(post, path = "/api/domains/letsencrypt-profiles", tag = "Domains")]
pub async fn create_letsencrypt_profile(
    State(state): State<AppState>,
    _auth: Authorized<SettingsManage>,
    Json(req): Json<LetsEncryptProfileRequest>,
) -> Result<Json<LetsEncryptProfileResponse>> {
    let db = state.get_db().await?;
    validate_letsencrypt(&db, &req).await?;
    let now = Utc::now();
    let model = letsencrypt_profile::ActiveModel {
        name: Set(req.name.trim().to_string()),
        email: Set(req.email.trim().to_string()),
        environment: Set(req.environment),
        challenge_type: Set(req.challenge_type),
        dns_profile_id: Set(req.dns_profile_id),
        renewal_enabled: Set(req.renewal_enabled.unwrap_or(true)),
        enabled: Set(req.enabled.unwrap_or(true)),
        status: Set("unknown".to_string()),
        last_error: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    Ok(Json(le_response(model)))
}

#[utoipa::path(put, path = "/api/domains/letsencrypt-profiles/{id}", tag = "Domains")]
pub async fn update_letsencrypt_profile(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<SettingsManage>,
    Json(req): Json<LetsEncryptProfileRequest>,
) -> Result<Json<LetsEncryptProfileResponse>> {
    let db = state.get_db().await?;
    validate_letsencrypt(&db, &req).await?;
    let existing = LetsEncryptProfile::find_by_id(id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Let's Encrypt profile {} not found", id)))?;
    let mut active: letsencrypt_profile::ActiveModel = existing.into();
    active.name = Set(req.name.trim().to_string());
    active.email = Set(req.email.trim().to_string());
    active.environment = Set(req.environment);
    active.challenge_type = Set(req.challenge_type);
    active.dns_profile_id = Set(req.dns_profile_id);
    active.renewal_enabled = Set(req.renewal_enabled.unwrap_or(true));
    active.enabled = Set(req.enabled.unwrap_or(true));
    active.updated_at = Set(Utc::now());
    Ok(Json(le_response(active.update(&db).await?)))
}

#[utoipa::path(
    delete,
    path = "/api/domains/letsencrypt-profiles/{id}",
    tag = "Domains"
)]
pub async fn delete_letsencrypt_profile(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<SettingsManage>,
) -> Result<Json<Value>> {
    let db = state.get_db().await?;
    if Domain::find()
        .filter(domain::Column::LetsencryptProfileId.eq(id))
        .one(&db)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Let's Encrypt profile is used by a domain".to_string(),
        ));
    }
    LetsEncryptProfile::delete_by_id(id).exec(&db).await?;
    Ok(Json(json!({ "success": true })))
}

#[utoipa::path(get, path = "/api/domains", tag = "Domains")]
pub async fn list_domains(
    State(state): State<AppState>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<Vec<DomainResponse>>> {
    let db = state.get_db().await?;
    let domains = Domain::find()
        .order_by_desc(domain::Column::Primary)
        .order_by_asc(domain::Column::Domain)
        .all(&db)
        .await?
        .into_iter()
        .map(domain_response)
        .collect();
    Ok(Json(domains))
}

#[utoipa::path(get, path = "/api/domains/{id}", tag = "Domains")]
pub async fn get_domain(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<DomainResponse>> {
    let db = state.get_db().await?;
    let domain = Domain::find_by_id(id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Domain {} not found", id)))?;
    Ok(Json(domain_response(domain)))
}

#[utoipa::path(post, path = "/api/domains", tag = "Domains")]
pub async fn create_domain(
    State(state): State<AppState>,
    _auth: Authorized<SettingsManage>,
    Json(req): Json<DomainRequest>,
) -> Result<Json<DomainResponse>> {
    let db = state.get_db().await?;
    validate_domain(&db, &req, None).await?;
    let now = Utc::now();
    if req.primary.unwrap_or(false) {
        clear_primary_domain(&db).await?;
    }
    let model = domain::ActiveModel {
        domain: Set(normalize_domain(&req.domain)),
        kind: Set(req.kind),
        scope: Set(req.scope),
        primary: Set(req.primary.unwrap_or(false)),
        enabled: Set(req.enabled.unwrap_or(true)),
        dns_mode: Set(req.dns_mode),
        ddns_profile_id: Set(req.ddns_profile_id),
        dns_status: Set("unknown".to_string()),
        tls_mode: Set(req.tls_mode),
        letsencrypt_profile_id: Set(req.letsencrypt_profile_id),
        tls_secret_name: Set(req.tls_secret_name),
        certificate_status: Set("unknown".to_string()),
        certificate_expires_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    Ok(Json(domain_response(model)))
}

#[utoipa::path(put, path = "/api/domains/{id}", tag = "Domains")]
pub async fn update_domain(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<SettingsManage>,
    Json(req): Json<DomainRequest>,
) -> Result<Json<DomainResponse>> {
    let db = state.get_db().await?;
    validate_domain(&db, &req, Some(id)).await?;
    let existing = Domain::find_by_id(id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Domain {} not found", id)))?;
    if req.primary.unwrap_or(false) {
        clear_primary_domain(&db).await?;
    }
    let mut active: domain::ActiveModel = existing.into();
    active.domain = Set(normalize_domain(&req.domain));
    active.kind = Set(req.kind);
    active.scope = Set(req.scope);
    active.primary = Set(req.primary.unwrap_or(false));
    active.enabled = Set(req.enabled.unwrap_or(true));
    active.dns_mode = Set(req.dns_mode);
    active.ddns_profile_id = Set(req.ddns_profile_id);
    active.tls_mode = Set(req.tls_mode);
    active.letsencrypt_profile_id = Set(req.letsencrypt_profile_id);
    active.tls_secret_name = Set(req.tls_secret_name);
    active.updated_at = Set(Utc::now());
    Ok(Json(domain_response(active.update(&db).await?)))
}

#[utoipa::path(delete, path = "/api/domains/{id}", tag = "Domains")]
pub async fn delete_domain(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<SettingsManage>,
) -> Result<Json<Value>> {
    let db = state.get_db().await?;
    Domain::delete_by_id(id).exec(&db).await?;
    Ok(Json(json!({ "success": true })))
}

#[utoipa::path(get, path = "/api/domains/assignments", tag = "Domains")]
pub async fn list_assignments(
    State(state): State<AppState>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<Vec<AppDomainAssignmentResponse>>> {
    let db = state.get_db().await?;
    let assignments = AppDomainAssignment::find()
        .order_by_asc(app_domain_assignment::Column::AppName)
        .all(&db)
        .await?
        .into_iter()
        .map(assignment_response)
        .collect();
    Ok(Json(assignments))
}

#[utoipa::path(get, path = "/api/domains/assignments/{id}", tag = "Domains")]
pub async fn get_assignment(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<SettingsView>,
) -> Result<Json<AppDomainAssignmentResponse>> {
    let db = state.get_db().await?;
    let assignment = AppDomainAssignment::find_by_id(id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Domain assignment {} not found", id)))?;
    Ok(Json(assignment_response(assignment)))
}

#[utoipa::path(post, path = "/api/domains/assignments", tag = "Domains")]
pub async fn create_assignment(
    State(state): State<AppState>,
    _auth: Authorized<SettingsManage>,
    Json(req): Json<AppDomainAssignmentRequest>,
) -> Result<Json<AppDomainAssignmentResponse>> {
    let db = state.get_db().await?;
    validate_assignment(&db, &req, None).await?;
    if req.primary.unwrap_or(false) {
        clear_primary_assignment(&db, &req.app_name).await?;
    }
    let now = Utc::now();
    let model = app_domain_assignment::ActiveModel {
        app_name: Set(req.app_name.trim().to_string()),
        domain_id: Set(req.domain_id),
        route_mode: Set(req.route_mode),
        hostname: Set(req
            .hostname
            .map(|h| h.trim().to_lowercase())
            .filter(|h| !h.is_empty())),
        path_prefix: Set(req
            .path_prefix
            .map(normalize_path)
            .filter(|p| !p.is_empty())),
        primary: Set(req.primary.unwrap_or(false)),
        enabled: Set(req.enabled.unwrap_or(true)),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    Ok(Json(assignment_response(model)))
}

#[utoipa::path(put, path = "/api/domains/assignments/{id}", tag = "Domains")]
pub async fn update_assignment(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<SettingsManage>,
    Json(req): Json<AppDomainAssignmentRequest>,
) -> Result<Json<AppDomainAssignmentResponse>> {
    let db = state.get_db().await?;
    validate_assignment(&db, &req, Some(id)).await?;
    let existing = AppDomainAssignment::find_by_id(id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Domain assignment {} not found", id)))?;
    if req.primary.unwrap_or(false) {
        clear_primary_assignment(&db, &req.app_name).await?;
    }
    let mut active: app_domain_assignment::ActiveModel = existing.into();
    active.app_name = Set(req.app_name.trim().to_string());
    active.domain_id = Set(req.domain_id);
    active.route_mode = Set(req.route_mode);
    active.hostname = Set(req
        .hostname
        .map(|h| h.trim().to_lowercase())
        .filter(|h| !h.is_empty()));
    active.path_prefix = Set(req
        .path_prefix
        .map(normalize_path)
        .filter(|p| !p.is_empty()));
    active.primary = Set(req.primary.unwrap_or(false));
    active.enabled = Set(req.enabled.unwrap_or(true));
    active.updated_at = Set(Utc::now());
    Ok(Json(assignment_response(active.update(&db).await?)))
}

#[utoipa::path(delete, path = "/api/domains/assignments/{id}", tag = "Domains")]
pub async fn delete_assignment(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<SettingsManage>,
) -> Result<Json<Value>> {
    let db = state.get_db().await?;
    AppDomainAssignment::delete_by_id(id).exec(&db).await?;
    Ok(Json(json!({ "success": true })))
}

fn ddns_response(model: dynamic_dns_profile::Model) -> Result<DynamicDnsProfileResponse> {
    Ok(DynamicDnsProfileResponse {
        id: model.id,
        name: model.name,
        provider: model.provider,
        capabilities: serde_json::from_str(&model.capabilities_json)?,
        config: redacted_ddns_config(
            serde_json::from_str(&model.config_json).unwrap_or_else(|_| json!({})),
        ),
        enabled: model.enabled,
        status: model.status,
        last_error: model.last_error,
        created_at: model.created_at,
        updated_at: model.updated_at,
    })
}

fn redacted_ddns_config(mut config: Value) -> Value {
    redact_config_key(&mut config, "token");
    redact_config_key(&mut config, "private_key");
    config
}

fn redact_config_key(config: &mut Value, key: &str) {
    if config
        .get(key)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        config[key] = Value::String("__redacted__".to_string());
    }
}

fn merge_ddns_config_for_update(mut config: Value, existing: &Value) -> Value {
    preserve_redacted_config_key(&mut config, existing, "token");
    preserve_redacted_config_key(&mut config, existing, "private_key");
    config
}

fn preserve_redacted_config_key(config: &mut Value, existing: &Value, key: &str) {
    if config
        .get(key)
        .and_then(Value::as_str)
        .is_some_and(|value| value == "__redacted__")
    {
        if let Some(existing_value) = existing.get(key) {
            config[key] = existing_value.clone();
        }
    }
}

fn le_response(model: letsencrypt_profile::Model) -> LetsEncryptProfileResponse {
    LetsEncryptProfileResponse {
        id: model.id,
        name: model.name,
        email: model.email,
        environment: model.environment,
        challenge_type: model.challenge_type,
        dns_profile_id: model.dns_profile_id,
        renewal_enabled: model.renewal_enabled,
        enabled: model.enabled,
        status: model.status,
        last_error: model.last_error,
        created_at: model.created_at,
        updated_at: model.updated_at,
    }
}

fn domain_response(model: domain::Model) -> DomainResponse {
    DomainResponse {
        id: model.id,
        domain: model.domain,
        kind: model.kind,
        scope: model.scope,
        primary: model.primary,
        enabled: model.enabled,
        dns_mode: model.dns_mode,
        ddns_profile_id: model.ddns_profile_id,
        dns_status: model.dns_status,
        tls_mode: model.tls_mode,
        letsencrypt_profile_id: model.letsencrypt_profile_id,
        tls_secret_name: model.tls_secret_name,
        certificate_status: model.certificate_status,
        certificate_expires_at: model.certificate_expires_at,
        created_at: model.created_at,
        updated_at: model.updated_at,
    }
}

fn assignment_response(model: app_domain_assignment::Model) -> AppDomainAssignmentResponse {
    AppDomainAssignmentResponse {
        id: model.id,
        app_name: model.app_name,
        domain_id: model.domain_id,
        route_mode: model.route_mode,
        hostname: model.hostname,
        path_prefix: model.path_prefix,
        primary: model.primary,
        enabled: model.enabled,
        created_at: model.created_at,
        updated_at: model.updated_at,
    }
}

async fn validate_letsencrypt(
    db: &sea_orm::DatabaseConnection,
    req: &LetsEncryptProfileRequest,
) -> Result<()> {
    validate_required(&req.name, "Profile name")?;
    validate_required(&req.email, "Email")?;
    if !matches!(req.environment.as_str(), "staging" | "production") {
        return Err(AppError::BadRequest(
            "Environment must be staging or production".to_string(),
        ));
    }
    if !matches!(req.challenge_type.as_str(), "http01" | "dns01") {
        return Err(AppError::BadRequest(
            "Challenge type must be http01 or dns01".to_string(),
        ));
    }
    if req.challenge_type == "dns01" {
        let id = req.dns_profile_id.ok_or_else(|| {
            AppError::BadRequest("DNS-01 requires a Dynamic DNS profile".to_string())
        })?;
        let profile = DynamicDnsProfile::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| {
                AppError::BadRequest("Selected Dynamic DNS profile does not exist".to_string())
            })?;
        let capabilities: Value = serde_json::from_str(&profile.capabilities_json)?;
        if capabilities.get("txt_records").and_then(Value::as_bool) != Some(true) {
            return Err(AppError::BadRequest(
                "DNS-01 requires a DNS profile with TXT record support".to_string(),
            ));
        }
    }
    Ok(())
}

async fn validate_domain(
    db: &sea_orm::DatabaseConnection,
    req: &DomainRequest,
    current_id: Option<i64>,
) -> Result<()> {
    let normalized = normalize_domain(&req.domain);
    validate_required(&normalized, "Domain")?;
    if !matches!(req.kind.as_str(), "root" | "wildcard" | "exact") {
        return Err(AppError::BadRequest(
            "Domain kind must be root, wildcard, or exact".to_string(),
        ));
    }
    if req.kind == "wildcard" && !normalized.starts_with("*.") {
        return Err(AppError::BadRequest(
            "Wildcard domains must start with *.".to_string(),
        ));
    }
    if !matches!(req.scope.as_str(), "public" | "private" | "both") {
        return Err(AppError::BadRequest(
            "Domain scope must be public, private, or both".to_string(),
        ));
    }
    if !matches!(req.dns_mode.as_str(), "manual" | "dynamic_dns") {
        return Err(AppError::BadRequest(
            "DNS mode must be manual or dynamic_dns".to_string(),
        ));
    }
    if req.dns_mode == "dynamic_dns" && req.ddns_profile_id.is_none() {
        return Err(AppError::BadRequest(
            "Dynamic DNS mode requires a Dynamic DNS profile".to_string(),
        ));
    }
    if !matches!(req.tls_mode.as_str(), "none" | "manual" | "letsencrypt") {
        return Err(AppError::BadRequest(
            "TLS mode must be none, manual, or letsencrypt".to_string(),
        ));
    }
    if req.tls_mode == "letsencrypt" {
        let id = req.letsencrypt_profile_id.ok_or_else(|| {
            AppError::BadRequest(
                "Let's Encrypt TLS mode requires a Let's Encrypt profile".to_string(),
            )
        })?;
        let profile = LetsEncryptProfile::find_by_id(id)
            .one(db)
            .await?
            .ok_or_else(|| {
                AppError::BadRequest("Selected Let's Encrypt profile does not exist".to_string())
            })?;
        if req.kind == "wildcard" && profile.challenge_type != "dns01" {
            return Err(AppError::BadRequest(
                "Wildcard certificates require a DNS-01 Let's Encrypt profile".to_string(),
            ));
        }
    }
    if let Some(existing) = Domain::find()
        .filter(domain::Column::Domain.eq(normalized))
        .one(db)
        .await?
    {
        if Some(existing.id) != current_id {
            return Err(AppError::Conflict("Domain already exists".to_string()));
        }
    }
    Ok(())
}

async fn validate_assignment(
    db: &sea_orm::DatabaseConnection,
    req: &AppDomainAssignmentRequest,
    current_id: Option<i64>,
) -> Result<()> {
    validate_required(&req.app_name, "App name")?;
    if !matches!(req.route_mode.as_str(), "path" | "subdomain" | "exact_host") {
        return Err(AppError::BadRequest(
            "Route mode must be path, subdomain, or exact_host".to_string(),
        ));
    }
    Domain::find_by_id(req.domain_id)
        .one(db)
        .await?
        .ok_or_else(|| AppError::BadRequest("Selected domain does not exist".to_string()))?;

    if req.route_mode == "path" {
        let prefix = req
            .path_prefix
            .as_deref()
            .map(|path| normalize_path(path.to_string()))
            .unwrap_or_default();
        validate_required(&prefix, "Path prefix")?;
        if reserved_path(&prefix) {
            return Err(AppError::Conflict(format!(
                "{} is a reserved Kubarr path",
                prefix
            )));
        }
    } else {
        let host = req
            .hostname
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        validate_required(&host, "Hostname")?;
        if let Some(existing) = AppDomainAssignment::find()
            .filter(app_domain_assignment::Column::Hostname.eq(host))
            .one(db)
            .await?
        {
            if Some(existing.id) != current_id {
                return Err(AppError::Conflict(
                    "Hostname is already assigned to another app".to_string(),
                ));
            }
        }
    }
    Ok(())
}

async fn clear_primary_domain(db: &sea_orm::DatabaseConnection) -> Result<()> {
    for model in Domain::find()
        .filter(domain::Column::Primary.eq(true))
        .all(db)
        .await?
    {
        let mut active: domain::ActiveModel = model.into();
        active.primary = Set(false);
        active.updated_at = Set(Utc::now());
        active.update(db).await?;
    }
    Ok(())
}

async fn clear_primary_assignment(db: &sea_orm::DatabaseConnection, app_name: &str) -> Result<()> {
    for model in AppDomainAssignment::find()
        .filter(app_domain_assignment::Column::AppName.eq(app_name.trim()))
        .filter(app_domain_assignment::Column::Primary.eq(true))
        .all(db)
        .await?
    {
        let mut active: app_domain_assignment::ActiveModel = model.into();
        active.primary = Set(false);
        active.updated_at = Set(Utc::now());
        active.update(db).await?;
    }
    Ok(())
}

fn validate_required(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(AppError::BadRequest(format!("{} is required", label)))
    } else {
        Ok(())
    }
}

fn normalize_domain(input: &str) -> String {
    input
        .trim()
        .to_lowercase()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn normalize_path(input: String) -> String {
    let trimmed = input.trim().trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{}", trimmed)
    }
}

fn reserved_path(path: &str) -> bool {
    let first = path
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or_default();
    matches!(
        first,
        "api"
            | "auth"
            | "assets"
            | "login"
            | "account"
            | "settings"
            | "users"
            | "storage"
            | "resources"
            | "apps"
            | "monitoring"
            | "logs"
            | "networking"
            | "security"
            | "app-error"
    )
}

fn default_dns_capabilities(provider: &str) -> Value {
    match provider {
        "transip" => json!({
            "a_records": true,
            "aaaa_records": true,
            "cname_records": true,
            "txt_records": true,
            "wildcard_records": true
        }),
        "manual" => json!({
            "a_records": false,
            "aaaa_records": false,
            "cname_records": false,
            "txt_records": false,
            "wildcard_records": false
        }),
        _ => json!({
            "a_records": true,
            "aaaa_records": true,
            "cname_records": false,
            "txt_records": false,
            "wildcard_records": false
        }),
    }
}
