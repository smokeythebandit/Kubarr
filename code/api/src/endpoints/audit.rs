use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};

use crate::error::Result;
use crate::middleware::permissions::{AuditManage, AuditView, Authorized};
use crate::services::audit::{
    clear_old_logs, get_audit_logs, get_audit_stats, AuditLogQuery, AuditLogResponse, AuditStats,
};
use crate::state::AppState;

/// Create audit routes
pub fn audit_routes(state: AppState) -> Router {
    Router::new()
        .route("/", get(list_audit_logs))
        .route("/stats", get(audit_stats))
        .route("/clear", axum::routing::post(clear_audit_logs))
        .with_state(state)
}

#[utoipa::path(
    get,
    path = "/api/audit",
    tag = "Audit",
    responses(
        (status = 200, description = "Audit logs with pagination", body = serde_json::Value)
    )
)]
/// List audit logs with filtering and pagination
async fn list_audit_logs(
    State(state): State<AppState>,
    _auth: Authorized<AuditView>,
    Query(query): Query<AuditLogQuery>,
) -> Result<Json<AuditLogResponse>> {
    let db = state.get_db().await?;
    let logs = get_audit_logs(&db, query).await?;
    Ok(Json(logs))
}

#[utoipa::path(
    get,
    path = "/api/audit/stats",
    tag = "Audit",
    responses(
        (status = 200, description = "Audit log statistics", body = serde_json::Value)
    )
)]
/// Get audit statistics
async fn audit_stats(
    State(state): State<AppState>,
    _auth: Authorized<AuditView>,
) -> Result<Json<AuditStats>> {
    let db = state.get_db().await?;
    let stats = get_audit_stats(&db).await?;
    Ok(Json(stats))
}

/// Clear old audit logs (admin only)
#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct ClearLogsRequest {
    pub days: Option<i64>,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct ClearLogsResponse {
    pub deleted: u64,
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/api/audit/clear",
    tag = "Audit",
    request_body = ClearLogsRequest,
    responses(
        (status = 200, description = "Result of clearing old audit logs", body = ClearLogsResponse)
    )
)]
async fn clear_audit_logs(
    State(state): State<AppState>,
    _auth: Authorized<AuditManage>,
    Json(request): Json<ClearLogsRequest>,
) -> Result<Json<ClearLogsResponse>> {
    let db = state.get_db().await?;
    let days = request.days.unwrap_or(90); // Default to 90 days retention
    let deleted = clear_old_logs(&db, days).await?;

    Ok(Json(ClearLogsResponse {
        deleted,
        message: format!(
            "Deleted {} audit log entries older than {} days",
            deleted, days
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_logs_request_deser_with_days() {
        let r: ClearLogsRequest = serde_json::from_str(r#"{"days": 30}"#).expect("deser");
        assert_eq!(r.days, Some(30));
    }

    #[test]
    fn clear_logs_request_deser_empty() {
        let r: ClearLogsRequest = serde_json::from_str("{}").expect("deser");
        assert_eq!(r.days, None);
    }

    #[test]
    fn clear_logs_response_ser() {
        let r = ClearLogsResponse {
            deleted: 42,
            message: "Deleted 42 audit log entries older than 90 days".to_string(),
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"deleted\":42"));
        assert!(json.contains("\"message\""));
    }

    #[test]
    fn clear_logs_response_zero_deleted() {
        let r = ClearLogsResponse {
            deleted: 0,
            message: "No entries deleted".to_string(),
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"deleted\":0"));
    }
}
