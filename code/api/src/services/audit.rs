use sea_orm::{
    sea_query::Expr, ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::db::DbConn;
use crate::error::{AppError, Result};
use crate::models::audit_log::{self, AuditAction, ResourceType};

/// Audit service for logging system events
#[derive(Clone, Default)]
pub struct AuditService {
    db: Arc<RwLock<Option<DbConn>>>,
}

impl AuditService {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn set_db(&self, db: DbConn) {
        *self.db.write().await = Some(db);
    }

    /// Log an audit event
    #[allow(clippy::too_many_arguments)]
    pub async fn log(
        &self,
        action: AuditAction,
        resource_type: ResourceType,
        resource_id: Option<String>,
        user_id: Option<i64>,
        username: Option<String>,
        details: Option<serde_json::Value>,
        ip_address: Option<String>,
        user_agent: Option<String>,
        success: bool,
        error_message: Option<String>,
    ) -> Result<()> {
        let db_guard = self.db.read().await;
        let db = match db_guard.as_ref() {
            Some(db) => db,
            None => {
                return Err(AppError::ServiceUnavailable(
                    "Audit database not initialized".to_string(),
                ));
            }
        };

        log_on_transaction(
            db,
            action,
            resource_type,
            resource_id,
            user_id,
            username,
            details,
            ip_address,
            user_agent,
            success,
            error_message,
        )
        .await?;
        Ok(())
    }

    /// Log a successful action
    #[allow(clippy::too_many_arguments)]
    pub async fn log_success(
        &self,
        action: AuditAction,
        resource_type: ResourceType,
        resource_id: Option<String>,
        user_id: Option<i64>,
        username: Option<String>,
        details: Option<serde_json::Value>,
        ip_address: Option<String>,
        user_agent: Option<String>,
    ) -> Result<()> {
        self.log(
            action,
            resource_type,
            resource_id,
            user_id,
            username,
            details,
            ip_address,
            user_agent,
            true,
            None,
        )
        .await
    }

    /// Log a failed action
    #[allow(clippy::too_many_arguments)]
    pub async fn log_failure(
        &self,
        action: AuditAction,
        resource_type: ResourceType,
        resource_id: Option<String>,
        user_id: Option<i64>,
        username: Option<String>,
        details: Option<serde_json::Value>,
        ip_address: Option<String>,
        user_agent: Option<String>,
        error: &str,
    ) -> Result<()> {
        self.log(
            action,
            resource_type,
            resource_id,
            user_id,
            username,
            details,
            ip_address,
            user_agent,
            false,
            Some(error.to_string()),
        )
        .await
    }
}

const MAX_DETAILS_BYTES: usize = 8192;
// Audit actor and resource columns remain VARCHAR(255) in PostgreSQL.
// Bound every free-form field by bytes so valid requests cannot fail at commit.
const MAX_FIELD_BYTES: usize = 255;
const REDACTED: &str = "[REDACTED]";

fn limited(value: String, max: usize) -> String {
    if value.len() <= max {
        return value;
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn sensitive(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace(['-', '_', ' '], "");
    [
        "password",
        "passwd",
        "token",
        "secret",
        "credential",
        "privatekey",
        "apikey",
        "invitecode",
        "totp",
        "recoverycode",
        "cookie",
        "authorization",
        "authheader",
        "session",
        "bearer",
        "accesskey",
    ]
    .iter()
    .any(|word| key.contains(word))
}

// Audit details are not a general-purpose JSON dump. Only typed identifiers and
// explicitly enumerated labels may survive; key-name matching alone cannot stop
// a password pasted into e.g. `name`, `reason`, or a nested arbitrary string.
fn allowed_label(key: &str, value: &str) -> bool {
    match key {
        "operation" => matches!(
            value,
            "audit_clear"
                | "automatic_audit_retention"
                | "session_revoked"
                | "install"
                | "update"
                | "delete"
                | "restart"
        ),
        "phase" => matches!(
            value,
            "queued" | "completed" | "failed" | "indeterminate" | "requested_access"
        ),
        "fields" => matches!(
            value,
            "profile"
                | "preferences"
                | "email"
                | "is_active"
                | "is_approved"
                | "role_ids"
                | "2fa_setup"
                | "name"
                | "service_provider"
                | "credentials"
                | "enabled"
                | "kill_switch"
                | "firewall_outbound_subnets"
        ),
        "reason" => matches!(
            value,
            "rejected"
                | "invalid_credentials"
                | "account_disabled"
                | "approval_required"
                | "setup_required"
                | "totp_required"
                | "invalid_totp"
        ),
        "method" => matches!(value, "totp" | "recovery_code" | "password"),
        "changed" => matches!(value, "apps" | "permissions"),
        "operation_id" => uuid::Uuid::parse_str(value).is_ok(),
        _ => false,
    }
}

fn allowed_detail_key(key: &str) -> bool {
    matches!(
        key,
        "operation"
            | "phase"
            | "fields"
            | "reason"
            | "method"
            | "changed"
            | "operation_id"
            | "days"
            | "deleted"
            | "attempts"
            | "provider_id"
            | "target_user_id"
            | "role_ids"
            | "enabled"
            | "admin_reset"
    )
}

fn app_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn vpn_app_action(action: &AuditAction, resource_type: &ResourceType) -> bool {
    matches!(resource_type, ResourceType::Vpn)
        && matches!(action, AuditAction::VpnAssigned | AuditAction::VpnRemoved)
}

fn sanitize(value: &mut serde_json::Value, key: &str, vpn_app_name: bool) {
    match value {
        serde_json::Value::Object(map) => {
            // Keys are attacker-controlled too. Drop unknown keys rather than
            // persisting a pasted secret as a JSON property name.
            map.retain(|key, _| {
                (allowed_detail_key(key) || (vpn_app_name && key == "app_name")) && !sensitive(key)
            });
            for (key, child) in map {
                let allow_slug = vpn_app_name && key == "app_name" && child.is_string();
                sanitize(child, key, allow_slug);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                sanitize(child, key, false);
            }
        }
        serde_json::Value::String(s) => {
            if !(vpn_app_name && key == "app_name" && app_slug(s)) && !allowed_label(key, s) {
                *s = REDACTED.into();
            }
        }
        serde_json::Value::Number(_)
            if !matches!(
                key,
                "days" | "deleted" | "attempts" | "provider_id" | "target_user_id" | "role_ids"
            ) =>
        {
            *value = serde_json::Value::String(REDACTED.into());
        }
        serde_json::Value::Bool(_) if !matches!(key, "enabled" | "admin_reset") => {
            *value = serde_json::Value::String(REDACTED.into());
        }
        _ => {}
    }
}

fn safe_error_label(label: &str) -> bool {
    matches!(
        label,
        "invalid_credentials"
            | "account_disabled"
            | "approval_required"
            | "totp_required"
            | "invalid_totp"
            | "setup_required"
            | "verification_failed"
            | "app_operation_failed"
            | "outcome_indeterminate_after_worker_interruption"
    )
}

fn safe_resource_id(action: &AuditAction, resource_type: &ResourceType, id: String) -> String {
    let valid = match resource_type {
        // App identities are catalog slugs; never accept arbitrary paths or headers.
        ResourceType::App => app_slug(&id),
        ResourceType::Vpn if vpn_app_action(action, resource_type) => app_slug(&id),
        ResourceType::System => matches!(
            id.as_str(),
            "registration_enabled" | "registration_require_approval"
        ),
        _ => id.parse::<i64>().is_ok() && id.len() <= 20,
    };
    if valid {
        id
    } else {
        REDACTED.into()
    }
}

/// Write an audit event on an existing connection/transaction. Callers can commit their
/// business change and this insert together; errors must abort the surrounding transaction.
#[allow(clippy::too_many_arguments)]
pub async fn log_on_transaction<C: ConnectionTrait>(
    db: &C,
    action: AuditAction,
    resource_type: ResourceType,
    resource_id: Option<String>,
    user_id: Option<i64>,
    username: Option<String>,
    details: Option<serde_json::Value>,
    ip_address: Option<String>,
    user_agent: Option<String>,
    success: bool,
    error_message: Option<String>,
) -> Result<audit_log::Model> {
    let details = details.map(|mut value| {
        sanitize(&mut value, "", vpn_app_action(&action, &resource_type));
        let encoded = value.to_string();
        if encoded.len() > MAX_DETAILS_BYTES {
            "\"[TRUNCATED]\"".to_string()
        } else {
            encoded
        }
    });
    let log_entry = audit_log::ActiveModel {
        timestamp: Set(chrono::Utc::now()),
        user_id: Set(user_id),
        username: Set(username.map(|s| limited(s, MAX_FIELD_BYTES))),
        action: Set(action.to_string()),
        resource_type: Set(resource_type.to_string()),
        resource_id: Set(resource_id.map(|s| safe_resource_id(&action, &resource_type, s))),
        details: Set(details),
        ip_address: Set(ip_address.map(|s| {
            if s.parse::<std::net::IpAddr>().is_ok() {
                s
            } else {
                REDACTED.into()
            }
        })),
        // A User-Agent header is fully attacker-controlled and can contain credentials.
        user_agent: Set(user_agent.map(|_| REDACTED.into())),
        success: Set(success),
        error_message: Set(error_message.map(|s| {
            if safe_error_label(&s) {
                s
            } else {
                REDACTED.into()
            }
        })),
        ..Default::default()
    };
    Ok(log_entry.insert(db).await?)
}

/// Query parameters for fetching audit logs
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct AuditLogQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub user_id: Option<i64>,
    pub action: Option<String>,
    pub resource_type: Option<String>,
    pub success: Option<bool>,
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    pub search: Option<String>,
}

/// Paginated audit log response
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AuditLogResponse {
    pub logs: Vec<audit_log::Model>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
    pub total_pages: u64,
}

/// Get audit logs with filtering and pagination
pub async fn get_audit_logs(db: &DbConn, query: AuditLogQuery) -> Result<AuditLogResponse> {
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).clamp(1, 100);
    let offset = (page - 1)
        .checked_mul(per_page)
        .ok_or_else(|| AppError::BadRequest("Page offset is too large".into()))?;

    let mut select = audit_log::Entity::find();

    // Apply filters
    if let Some(user_id) = query.user_id {
        select = select.filter(audit_log::Column::UserId.eq(user_id));
    }

    if let Some(action) = &query.action {
        select = select.filter(audit_log::Column::Action.eq(action.clone()));
    }

    if let Some(resource_type) = &query.resource_type {
        select = select.filter(audit_log::Column::ResourceType.eq(resource_type.clone()));
    }

    if let Some(success) = query.success {
        select = select.filter(audit_log::Column::Success.eq(success));
    }

    if let Some(from) = query.from {
        select = select.filter(audit_log::Column::Timestamp.gte(from));
    }

    if let Some(to) = query.to {
        select = select.filter(audit_log::Column::Timestamp.lte(to));
    }

    if let Some(search) = &query.search {
        select = select.filter(
            audit_log::Column::Username
                .contains(search)
                .or(audit_log::Column::Action.contains(search))
                .or(audit_log::Column::ResourceId.contains(search))
                .or(audit_log::Column::Details.contains(search)),
        );
    }

    // Get total count
    let total = select.clone().count(db).await?;

    // Get paginated results ordered by timestamp descending
    let logs = select
        .order_by_desc(audit_log::Column::Timestamp)
        .offset(offset)
        .limit(per_page)
        .all(db)
        .await?;

    let total_pages = total / per_page + u64::from(total % per_page != 0);

    Ok(AuditLogResponse {
        logs,
        total,
        page,
        per_page,
        total_pages,
    })
}

/// Get audit log statistics
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AuditStats {
    pub total_events: u64,
    pub successful_events: u64,
    pub failed_events: u64,
    pub events_today: u64,
    pub events_this_week: u64,
    pub top_actions: Vec<ActionCount>,
    pub recent_failures: Vec<audit_log::Model>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ActionCount {
    pub action: String,
    pub count: u64,
}

pub async fn get_audit_stats(db: &DbConn) -> Result<AuditStats> {
    let total_events = audit_log::Entity::find().count(db).await?;

    let successful_events = audit_log::Entity::find()
        .filter(audit_log::Column::Success.eq(true))
        .count(db)
        .await?;

    let failed_events = audit_log::Entity::find()
        .filter(audit_log::Column::Success.eq(false))
        .count(db)
        .await?;

    let today = chrono::Utc::now().date_naive();
    let today_start = today.and_hms_opt(0, 0, 0).unwrap_or_default();
    let today_start_utc =
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(today_start, chrono::Utc);

    let events_today = audit_log::Entity::find()
        .filter(audit_log::Column::Timestamp.gte(today_start_utc))
        .count(db)
        .await?;

    let week_ago = chrono::Utc::now() - chrono::Duration::days(7);
    let events_this_week = audit_log::Entity::find()
        .filter(audit_log::Column::Timestamp.gte(week_ago))
        .count(db)
        .await?;

    // Get recent failures
    let recent_failures = audit_log::Entity::find()
        .filter(audit_log::Column::Success.eq(false))
        .order_by_desc(audit_log::Column::Timestamp)
        .limit(10)
        .all(db)
        .await?;

    #[derive(FromQueryResult)]
    struct GroupedAction {
        action: String,
        count: i64,
    }
    let grouped: Vec<GroupedAction> = audit_log::Entity::find()
        .select_only()
        .column(audit_log::Column::Action)
        .column_as(Expr::col(audit_log::Column::Id).count(), "count")
        .group_by(audit_log::Column::Action)
        .order_by_desc(Expr::col(audit_log::Column::Id).count())
        .limit(10)
        .into_model::<GroupedAction>()
        .all(db)
        .await?;
    let top_actions = grouped
        .into_iter()
        .map(|row| ActionCount {
            action: row.action,
            count: row.count as u64,
        })
        .collect();

    Ok(AuditStats {
        total_events,
        successful_events,
        failed_events,
        events_today,
        events_this_week,
        top_actions,
        recent_failures,
    })
}

/// Clear old audit logs (retention policy)
pub async fn clear_old_logs(db: &DbConn, days: i64) -> Result<u64> {
    clear_old_logs_with(db, days).await
}

/// Delete expired audit rows on a connection or an existing transaction.
/// Use a transaction when the deletion must be recorded atomically.
pub async fn clear_old_logs_with<C: ConnectionTrait>(db: &C, days: i64) -> Result<u64> {
    validate_retention(days)?;
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days);

    let result = audit_log::Entity::delete_many()
        .filter(audit_log::Column::Timestamp.lt(cutoff))
        .exec(db)
        .await?;

    Ok(result.rows_affected)
}

fn validate_retention(days: i64) -> Result<()> {
    if !(1..=3650).contains(&days) {
        return Err(AppError::BadRequest(
            "Retention days must be between 1 and 3650".into(),
        ));
    }
    Ok(())
}

/// Atomically delete expired rows and write an attributed audit event; no deletion
/// persists if the audit insert fails. The new event is never eligible for deletion.
pub async fn clear_old_logs_audited(
    db: &DbConn,
    days: i64,
    user_id: i64,
    username: String,
) -> Result<(u64, i64)> {
    validate_retention(days)?;
    let txn = db.begin().await?;
    let deleted = clear_old_logs_with(&txn, days).await?;
    let event = log_on_transaction(
        &txn,
        AuditAction::SystemSettingChanged,
        ResourceType::System,
        None,
        Some(user_id),
        Some(username),
        Some(serde_json::json!({"operation": "audit_clear", "days": days, "deleted": deleted})),
        None,
        None,
        true,
        None,
    )
    .await?;
    txn.commit().await?;
    Ok((deleted, event.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn vpn_resource_ids_are_slugs_only_for_assignment_and_removal() {
        for action in [AuditAction::VpnAssigned, AuditAction::VpnRemoved] {
            assert_eq!(
                safe_resource_id(&action, &ResourceType::Vpn, "sonarr-2".into()),
                "sonarr-2"
            );
            for invalid in ["", "Sonarr", "a/b", "secret pasted", "é", &"a".repeat(65)] {
                assert_eq!(
                    safe_resource_id(&action, &ResourceType::Vpn, invalid.into()),
                    REDACTED
                );
            }
        }
        assert_eq!(
            safe_resource_id(
                &AuditAction::VpnProviderCreated,
                &ResourceType::Vpn,
                "42".into()
            ),
            "42"
        );
        assert_eq!(
            safe_resource_id(
                &AuditAction::VpnProviderCreated,
                &ResourceType::Vpn,
                "sonarr".into()
            ),
            REDACTED
        );
        assert_eq!(
            safe_resource_id(
                &AuditAction::VpnAssigned,
                &ResourceType::User,
                "sonarr".into()
            ),
            REDACTED
        );
    }

    #[test]
    fn vpn_app_name_details_are_scoped_and_validated() {
        for action in [AuditAction::VpnAssigned, AuditAction::VpnRemoved] {
            let mut details = serde_json::json!({"app_name": "sonarr-2", "name": "pasted-secret", "unknown": "pasted-secret"});
            sanitize(
                &mut details,
                "",
                vpn_app_action(&action, &ResourceType::Vpn),
            );
            assert_eq!(details, serde_json::json!({"app_name": "sonarr-2"}));
            for invalid in ["", "Sonarr", "a/b", "pasted secret", "é", &"a".repeat(65)] {
                let mut details = serde_json::json!({"app_name": invalid});
                sanitize(
                    &mut details,
                    "",
                    vpn_app_action(&action, &ResourceType::Vpn),
                );
                assert_eq!(details["app_name"], REDACTED);
            }
        }
        for (action, resource_type) in [
            (AuditAction::VpnProviderCreated, ResourceType::Vpn),
            (AuditAction::VpnAssigned, ResourceType::App),
        ] {
            let mut details = serde_json::json!({"app_name": "sonarr", "provider_id": 42});
            sanitize(&mut details, "", vpn_app_action(&action, &resource_type));
            assert_eq!(details, serde_json::json!({"provider_id": 42}));
        }
    }

    fn make_audit_model() -> crate::models::audit_log::Model {
        crate::models::audit_log::Model {
            id: 1,
            timestamp: Utc::now(),
            user_id: Some(1),
            username: Some("admin".to_string()),
            action: "user.created".to_string(),
            resource_type: "user".to_string(),
            resource_id: Some("42".to_string()),
            details: None,
            ip_address: Some("127.0.0.1".to_string()),
            user_agent: None,
            success: true,
            error_message: None,
        }
    }

    #[test]
    fn audit_service_new_works() {
        let _svc = AuditService::new();
    }

    #[test]
    fn audit_log_query_deser_empty() {
        let q: AuditLogQuery = serde_json::from_str("{}").expect("deser");
        assert!(q.page.is_none());
        assert!(q.per_page.is_none());
        assert!(q.user_id.is_none());
        assert!(q.action.is_none());
        assert!(q.success.is_none());
    }

    #[test]
    fn audit_log_query_deser_full() {
        let json = r#"{"page":2,"per_page":25,"user_id":5,"action":"login","resource_type":"user","success":true,"search":"admin"}"#;
        let q: AuditLogQuery = serde_json::from_str(json).expect("deser");
        assert_eq!(q.page, Some(2));
        assert_eq!(q.per_page, Some(25));
        assert_eq!(q.user_id, Some(5));
        assert_eq!(q.action.as_deref(), Some("login"));
        assert_eq!(q.success, Some(true));
        assert_eq!(q.search.as_deref(), Some("admin"));
    }

    #[test]
    fn action_count_ser() {
        let ac = ActionCount {
            action: "login".to_string(),
            count: 42,
        };
        let json = serde_json::to_string(&ac).expect("ser");
        assert!(json.contains("\"action\":\"login\""));
        assert!(json.contains("\"count\":42"));
    }

    #[test]
    fn audit_log_response_ser() {
        let r = AuditLogResponse {
            logs: vec![make_audit_model()],
            total: 1,
            page: 1,
            per_page: 50,
            total_pages: 1,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"total\":1"));
        assert!(json.contains("\"per_page\":50"));
    }

    #[test]
    fn audit_log_response_empty_ser() {
        let r = AuditLogResponse {
            logs: vec![],
            total: 0,
            page: 1,
            per_page: 50,
            total_pages: 0,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"logs\":[]"));
    }

    #[test]
    fn audit_stats_ser() {
        let r = AuditStats {
            total_events: 100,
            successful_events: 90,
            failed_events: 10,
            events_today: 5,
            events_this_week: 30,
            top_actions: vec![ActionCount {
                action: "login".to_string(),
                count: 50,
            }],
            recent_failures: vec![make_audit_model()],
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"total_events\":100"));
        assert!(json.contains("\"failed_events\":10"));
        assert!(json.contains("\"top_actions\""));
    }

    async fn make_db() -> DbConn {
        crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("in-memory db")
    }

    // -------------------------------------------------------------------------
    // AuditService.log / log_success / log_failure — require DB
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn log_without_db_is_error() {
        let svc = AuditService::new();
        let result = svc
            .log(
                AuditAction::Login,
                ResourceType::User,
                None,
                None,
                None,
                None,
                None,
                None,
                true,
                None,
            )
            .await;
        assert!(matches!(result, Err(AppError::ServiceUnavailable(_))));
    }

    #[tokio::test]
    async fn log_with_db_inserts_record() {
        let db = make_db().await;
        let svc = AuditService::new();
        svc.set_db(db.clone()).await;

        svc.log(
            AuditAction::Login,
            ResourceType::User,
            Some("user42".to_string()),
            Some(1),
            Some("alice".to_string()),
            Some(serde_json::json!({"ip": "127.0.0.1"})),
            Some("127.0.0.1".to_string()),
            Some("Mozilla/5.0".to_string()),
            true,
            None,
        )
        .await
        .expect("log");

        use crate::models::audit_log::Entity;
        use sea_orm::EntityTrait;
        let logs = Entity::find().all(&db).await.expect("find");
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].action, "login");
        assert_eq!(logs[0].success, true);
        assert_eq!(logs[0].username.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn log_success_inserts_with_success_true() {
        let db = make_db().await;
        let svc = AuditService::new();
        svc.set_db(db.clone()).await;

        svc.log_success(
            AuditAction::UserCreated,
            ResourceType::User,
            Some("99".to_string()),
            Some(2),
            Some("admin".to_string()),
            None,
            None,
            None,
        )
        .await
        .expect("log_success");

        use crate::models::audit_log::Entity;
        use sea_orm::EntityTrait;
        let logs = Entity::find().all(&db).await.expect("find");
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].success, true);
        assert!(logs[0].error_message.is_none());
    }

    #[tokio::test]
    async fn log_failure_inserts_with_success_false() {
        let db = make_db().await;
        let svc = AuditService::new();
        svc.set_db(db.clone()).await;

        svc.log_failure(
            AuditAction::LoginFailed,
            ResourceType::User,
            None,
            None,
            Some("bob".to_string()),
            None,
            Some("10.0.0.1".to_string()),
            None,
            "invalid password",
        )
        .await
        .expect("log_failure");

        use crate::models::audit_log::Entity;
        use sea_orm::EntityTrait;
        let logs = Entity::find().all(&db).await.expect("find");
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].success, false);
        assert_eq!(logs[0].error_message.as_deref(), Some("[REDACTED]"));
    }

    // -------------------------------------------------------------------------
    // get_audit_logs — various filter combinations
    // -------------------------------------------------------------------------

    async fn insert_log(
        db: &DbConn,
        action: &str,
        resource_type: &str,
        user_id: Option<i64>,
        username: Option<&str>,
        success: bool,
    ) {
        use crate::models::audit_log;
        use sea_orm::Set;

        audit_log::ActiveModel {
            timestamp: Set(chrono::Utc::now()),
            user_id: Set(user_id),
            username: Set(username.map(|s| s.to_string())),
            action: Set(action.to_string()),
            resource_type: Set(resource_type.to_string()),
            resource_id: Set(None),
            details: Set(None),
            ip_address: Set(None),
            user_agent: Set(None),
            success: Set(success),
            error_message: Set(None),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert");
    }

    #[tokio::test]
    async fn get_audit_logs_no_filters_returns_all() {
        let db = make_db().await;
        insert_log(&db, "login", "user", Some(1), Some("alice"), true).await;
        insert_log(&db, "login", "user", Some(2), Some("bob"), true).await;
        insert_log(&db, "logout", "user", Some(1), Some("alice"), true).await;

        let query = AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: None,
        };
        let result = get_audit_logs(&db, query).await.expect("get_audit_logs");
        assert_eq!(result.total, 3);
        assert_eq!(result.logs.len(), 3);
        assert_eq!(result.page, 1);
        assert_eq!(result.per_page, 50);
    }

    #[tokio::test]
    async fn get_audit_logs_filter_by_user_id() {
        let db = make_db().await;
        insert_log(&db, "login", "user", Some(1), Some("alice"), true).await;
        insert_log(&db, "login", "user", Some(2), Some("bob"), true).await;

        let query = AuditLogQuery {
            page: None,
            per_page: None,
            user_id: Some(1),
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: None,
        };
        let result = get_audit_logs(&db, query).await.expect("get_audit_logs");
        assert_eq!(result.total, 1);
        assert_eq!(result.logs[0].username.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn get_audit_logs_filter_by_action() {
        let db = make_db().await;
        insert_log(&db, "login", "user", Some(1), Some("alice"), true).await;
        insert_log(&db, "logout", "user", Some(1), Some("alice"), true).await;

        let query = AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: Some("login".to_string()),
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: None,
        };
        let result = get_audit_logs(&db, query).await.expect("get_audit_logs");
        assert_eq!(result.total, 1);
        assert_eq!(result.logs[0].action, "login");
    }

    #[tokio::test]
    async fn get_audit_logs_filter_by_resource_type() {
        let db = make_db().await;
        insert_log(&db, "login", "user", Some(1), Some("alice"), true).await;
        insert_log(
            &db,
            "app.installed",
            "application",
            Some(1),
            Some("alice"),
            true,
        )
        .await;

        let query = AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: Some("application".to_string()),
            success: None,
            from: None,
            to: None,
            search: None,
        };
        let result = get_audit_logs(&db, query).await.expect("get_audit_logs");
        assert_eq!(result.total, 1);
        assert_eq!(result.logs[0].resource_type, "application");
    }

    #[tokio::test]
    async fn get_audit_logs_filter_by_success() {
        let db = make_db().await;
        insert_log(&db, "login", "user", Some(1), Some("alice"), true).await;
        insert_log(&db, "login", "user", Some(1), Some("alice"), false).await;

        let query = AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: Some(false),
            from: None,
            to: None,
            search: None,
        };
        let result = get_audit_logs(&db, query).await.expect("get_audit_logs");
        assert_eq!(result.total, 1);
        assert_eq!(result.logs[0].success, false);
    }

    #[tokio::test]
    async fn get_audit_logs_filter_by_time_range() {
        let db = make_db().await;
        insert_log(&db, "login", "user", Some(1), Some("alice"), true).await;

        let future = chrono::Utc::now() + chrono::Duration::hours(1);
        let past = chrono::Utc::now() - chrono::Duration::hours(1);

        // from filter: events after 'past' → should include the record
        let query = AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: Some(past),
            to: Some(future),
            search: None,
        };
        let result = get_audit_logs(&db, query).await.expect("get_audit_logs");
        assert_eq!(result.total, 1);
    }

    #[tokio::test]
    async fn get_audit_logs_search_filter() {
        let db = make_db().await;
        insert_log(&db, "login", "user", Some(1), Some("alice"), true).await;
        insert_log(&db, "login", "user", Some(2), Some("bob"), true).await;

        let query = AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: Some("alice".to_string()),
        };
        let result = get_audit_logs(&db, query).await.expect("get_audit_logs");
        assert_eq!(result.total, 1);
        assert_eq!(result.logs[0].username.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn get_audit_logs_pagination() {
        let db = make_db().await;
        for i in 0..5 {
            insert_log(&db, "login", "user", Some(i), Some("user"), true).await;
        }

        let query = AuditLogQuery {
            page: Some(1),
            per_page: Some(2),
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: None,
        };
        let result = get_audit_logs(&db, query).await.expect("get_audit_logs");
        assert_eq!(result.total, 5);
        assert_eq!(result.logs.len(), 2);
        assert_eq!(result.per_page, 2);
        assert_eq!(result.total_pages, 3);
    }

    // -------------------------------------------------------------------------
    // get_audit_stats
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn get_audit_stats_empty_db() {
        let db = make_db().await;
        let stats = get_audit_stats(&db).await.expect("get_audit_stats");
        assert_eq!(stats.total_events, 0);
        assert_eq!(stats.successful_events, 0);
        assert_eq!(stats.failed_events, 0);
        assert_eq!(stats.top_actions.len(), 0);
        assert_eq!(stats.recent_failures.len(), 0);
    }

    #[tokio::test]
    async fn get_audit_stats_with_data() {
        let db = make_db().await;
        insert_log(&db, "login", "user", Some(1), Some("alice"), true).await;
        insert_log(&db, "login", "user", Some(2), Some("bob"), true).await;
        insert_log(&db, "logout", "user", Some(1), Some("alice"), false).await;

        let stats = get_audit_stats(&db).await.expect("get_audit_stats");
        assert_eq!(stats.total_events, 3);
        assert_eq!(stats.successful_events, 2);
        assert_eq!(stats.failed_events, 1);
        assert_eq!(stats.events_today, 3, "all inserted today");
        assert_eq!(stats.events_this_week, 3);
        assert!(!stats.top_actions.is_empty());
        assert_eq!(stats.recent_failures.len(), 1);
    }

    // -------------------------------------------------------------------------
    // clear_old_logs
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn clear_old_logs_removes_old_records() {
        let db = make_db().await;

        // Insert an "old" log by setting its timestamp manually
        use crate::models::audit_log;
        use sea_orm::Set;
        let old_time = chrono::Utc::now() - chrono::Duration::days(40);
        audit_log::ActiveModel {
            timestamp: Set(old_time),
            user_id: Set(None),
            username: Set(None),
            action: Set("login".to_string()),
            resource_type: Set("user".to_string()),
            resource_id: Set(None),
            details: Set(None),
            ip_address: Set(None),
            user_agent: Set(None),
            success: Set(true),
            error_message: Set(None),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert old");

        // Insert a recent log (should not be deleted)
        insert_log(&db, "logout", "user", None, None, true).await;

        let deleted = clear_old_logs(&db, 30).await.expect("clear_old_logs");
        assert_eq!(deleted, 1, "one old log should be deleted");

        use crate::models::audit_log::Entity;
        use sea_orm::EntityTrait;
        let remaining = Entity::find().all(&db).await.expect("find");
        assert_eq!(remaining.len(), 1, "one recent log remains");
    }

    #[tokio::test]
    async fn clear_old_logs_empty_db_returns_zero() {
        let db = make_db().await;
        let deleted = clear_old_logs(&db, 30).await.expect("clear");
        assert_eq!(deleted, 0);
    }
}
