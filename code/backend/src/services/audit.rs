use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::db::DbConn;
use crate::error::Result;
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
                tracing::warn!("Audit service: database not initialized, skipping log");
                return Ok(());
            }
        };

        let now = chrono::Utc::now();
        let details_str = details.map(|d| d.to_string());

        let log_entry = audit_log::ActiveModel {
            timestamp: Set(now),
            user_id: Set(user_id),
            username: Set(username),
            action: Set(action.to_string()),
            resource_type: Set(resource_type.to_string()),
            resource_id: Set(resource_id),
            details: Set(details_str),
            ip_address: Set(ip_address),
            user_agent: Set(user_agent),
            success: Set(success),
            error_message: Set(error_message),
            ..Default::default()
        };

        log_entry.insert(db).await?;
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
    let per_page = query.per_page.unwrap_or(50).min(100);
    let offset = (page - 1) * per_page;

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
        let search_pattern = format!("%{}%", search);
        select = select.filter(
            audit_log::Column::Username
                .contains(&search_pattern)
                .or(audit_log::Column::Action.contains(&search_pattern))
                .or(audit_log::Column::ResourceId.contains(&search_pattern))
                .or(audit_log::Column::Details.contains(&search_pattern)),
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

    let total_pages = (total as f64 / per_page as f64).ceil() as u64;

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
    use sea_orm::QuerySelect;

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

    // For top actions, we'll do a simple approach since SeaORM grouping is complex
    // Fetch all logs to count actions (select all columns to avoid partial model issues)
    let all_logs = audit_log::Entity::find().all(db).await?;

    let mut action_counts: std::collections::HashMap<String, u64> =
        std::collections::HashMap::new();
    for log in all_logs {
        *action_counts.entry(log.action.clone()).or_insert(0) += 1;
    }

    let mut top_actions: Vec<ActionCount> = action_counts
        .into_iter()
        .map(|(action, count)| ActionCount { action, count })
        .collect();
    top_actions.sort_by(|a, b| b.count.cmp(&a.count));
    top_actions.truncate(10);

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
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days);

    let result = audit_log::Entity::delete_many()
        .filter(audit_log::Column::Timestamp.lt(cutoff))
        .exec(db)
        .await?;

    Ok(result.rows_affected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

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
}
