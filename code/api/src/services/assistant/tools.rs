//! Kubarr read tools for the assistant.
//!
//! Every tool runs with the requesting user's permissions and allowed apps:
//! a tool whose permission the user lacks is not offered, and each call
//! checks again. Apps outside the user's allowed apps read as "not found"
//! (administrators, `settings.manage`, see every app). Results are JSON,
//! scrubbed of inline secrets where they carry free text, and capped at
//! [`MAX_RESULT_BYTES`].

use chrono::{DateTime, Duration, Utc};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use serde_json::{json, Map, Value};

use crate::models::audit_log::{AuditAction, ResourceType};
use crate::models::{app_operation, app_state};
use crate::services::app_manager::{AppStateResponse, DESIRED_INSTALLED};
use crate::services::update_diagnostics::{scrub_inline, scrub_with, secret_values_for_namespace};
use crate::state::AppState;

/// Tool results larger than this are cut, with [`TRUNCATED_MARKER`] appended.
pub const MAX_RESULT_BYTES: usize = 20 * 1024;
pub const TRUNCATED_MARKER: &str = "\n[truncated: the result was longer than 20 KB]";
/// Free-text fields (messages, diagnostics, log lines) are cut to this many characters.
const MAX_FIELD_CHARS: usize = 2_000;
const MAX_LOG_LINE_CHARS: usize = 1_000;

const ADMIN_PERMISSION: &str = "settings.manage";
const METRICS_APP: &str = "victoriametrics";
const LOGS_APP: &str = "victorialogs";

/// The read tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    ListApps,
    GetApp,
    ListOperations,
    ListNotifications,
    GetStorage,
    GetUpdateSettings,
    SearchCatalog,
    QueryMetrics,
    QueryLogs,
}

impl Tool {
    /// In the order offered to the model (kept fixed for prompt caching).
    pub const ALL: [Tool; 9] = [
        Tool::ListApps,
        Tool::GetApp,
        Tool::ListOperations,
        Tool::ListNotifications,
        Tool::GetStorage,
        Tool::GetUpdateSettings,
        Tool::SearchCatalog,
        Tool::QueryMetrics,
        Tool::QueryLogs,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::ListApps => "list_apps",
            Tool::GetApp => "get_app",
            Tool::ListOperations => "list_operations",
            Tool::ListNotifications => "list_notifications",
            Tool::GetStorage => "get_storage",
            Tool::GetUpdateSettings => "get_update_settings",
            Tool::SearchCatalog => "search_catalog",
            Tool::QueryMetrics => "query_metrics",
            Tool::QueryLogs => "query_logs",
        }
    }

    pub fn parse(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.name() == name)
    }

    /// The permission needed besides `assistant.use`; `None` for the
    /// user's own data.
    pub fn permission(self) -> Option<&'static str> {
        match self {
            Tool::ListApps | Tool::GetApp | Tool::ListOperations | Tool::SearchCatalog => {
                Some("apps.view")
            }
            Tool::ListNotifications => None,
            Tool::GetStorage => Some("storage.view"),
            Tool::GetUpdateSettings => Some("settings.view"),
            Tool::QueryMetrics => Some("monitoring.view"),
            Tool::QueryLogs => Some("logs.view"),
        }
    }

    fn description(self) -> &'static str {
        match self {
            Tool::ListApps => {
                "List the apps installed in Kubarr that the user can see, with their \
                 state, health, installed and available chart version, whether an update is \
                 available (and its kind) and whether automatic updates are on."
            }
            Tool::GetApp => {
                "Details of one app: its catalog entry, current state and health, versions, \
                 automatic update setting and its five most recent operations."
            }
            Tool::ListOperations => {
                "Recent Kubarr operations (install, update, restart, delete, rollback) with \
                 their status, outcome message, error and redacted diagnostics, newest first. \
                 Optionally for one app."
            }
            Tool::ListNotifications => "The user's own notification inbox, newest first.",
            Tool::GetStorage => {
                "Disk usage of the shared media storage: total, used and free bytes."
            }
            Tool::GetUpdateSettings => {
                "Kubarr's automatic update settings: whether they are on, the weekly schedule \
                 (7 days, Monday first, 24 local hours each), timezone, health timeout and \
                 when the next update window starts."
            }
            Tool::SearchCatalog => {
                "Search the catalog of installable apps by name, description or category."
            }
            Tool::QueryMetrics => {
                "Resource usage of one app over a time range from VictoriaMetrics: CPU \
                 (cores), memory (bytes), network receive/transmit (bytes per second) or \
                 container restarts. Returns latest, min, max, average and a short series."
            }
            Tool::QueryLogs => {
                "Recent log lines of one app from VictoriaLogs, newest first, optionally only \
                 lines containing a phrase. Secrets are redacted."
            }
        }
    }

    fn input_schema(self) -> Value {
        let app = json!({"type": "string", "description": "App name as listed by list_apps, e.g. \"jellyfin\""});
        let (properties, required): (Value, Vec<&str>) = match self {
            Tool::ListApps | Tool::GetStorage | Tool::GetUpdateSettings => (json!({}), vec![]),
            Tool::GetApp => (json!({ "name": app }), vec!["name"]),
            Tool::ListOperations => (
                json!({
                    "app": {"type": "string", "description": "Only operations of this app"},
                    "limit": {"type": "integer", "description": "How many (default 10, at most 50)"}
                }),
                vec![],
            ),
            Tool::ListNotifications => (
                json!({
                    "limit": {"type": "integer", "description": "How many (default 10, at most 50)"},
                    "unread_only": {"type": "boolean", "description": "Only unread notifications"}
                }),
                vec![],
            ),
            Tool::SearchCatalog => (
                json!({ "query": {"type": "string", "description": "Words to search for"} }),
                vec!["query"],
            ),
            Tool::QueryMetrics => (
                json!({
                    "app": app,
                    "metric": {"type": "string", "enum": ["cpu", "memory", "network_receive", "network_transmit", "restarts"]},
                    "range": {"type": "string", "enum": ["15m", "1h", "6h", "24h"], "description": "Time range (default 1h)"}
                }),
                vec!["app", "metric"],
            ),
            Tool::QueryLogs => (
                json!({
                    "app": app,
                    "since": {"type": "string", "enum": ["15m", "1h", "6h", "24h"], "description": "How far back (default 1h)"},
                    "contains": {"type": "string", "description": "Only lines containing this phrase"},
                    "limit": {"type": "integer", "description": "How many lines (default 50, at most 200)"}
                }),
                vec!["app"],
            ),
        };
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        })
    }

    /// The Messages API tool definition (strict schema).
    pub fn definition(self) -> Value {
        json!({
            "name": self.name(),
            "description": self.description(),
            "input_schema": self.input_schema(),
            "strict": true
        })
    }
}

/// Which optional monitoring apps are installed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Monitoring {
    pub metrics: bool,
    pub logs: bool,
}

impl Monitoring {
    /// VictoriaMetrics / VictoriaLogs count as installed when Kubarr wants
    /// them installed.
    pub async fn detect(db: &DatabaseConnection) -> Monitoring {
        let installed = |name: &'static str| async move {
            app_state::Entity::find_by_id(name.to_string())
                .one(db)
                .await
                .ok()
                .flatten()
                .is_some_and(|state| state.desired_state == DESIRED_INSTALLED)
        };
        Monitoring {
            metrics: installed(METRICS_APP).await,
            logs: installed(LOGS_APP).await,
        }
    }
}

/// Who a tool runs for.
#[derive(Clone)]
pub struct ToolContext {
    pub state: AppState,
    pub db: DatabaseConnection,
    pub user_id: i64,
    pub username: String,
    pub permissions: Vec<String>,
    pub conversation_id: String,
    pub monitoring: Monitoring,
}

impl ToolContext {
    pub fn has(&self, permission: &str) -> bool {
        self.permissions.iter().any(|p| p == permission)
    }

    pub fn is_admin(&self) -> bool {
        self.has(ADMIN_PERMISSION)
    }

    /// The existing allowed-apps model: `app.*`, `app.<name>`, or admin.
    pub fn sees_app(&self, app: &str) -> bool {
        self.is_admin() || self.has("app.*") || self.has(&format!("app.{app}"))
    }

    /// The apps a non-admin may see (`app.<name>` grants); `None` when the
    /// user sees every app.
    pub fn visible_app_names(&self) -> Option<Vec<String>> {
        if self.is_admin() || self.has("app.*") {
            return None;
        }
        Some(
            self.permissions
                .iter()
                .filter_map(|p| p.strip_prefix("app."))
                .map(str::to_string)
                .collect(),
        )
    }

    /// The tools this user is offered, in fixed order.
    pub fn tools(&self) -> Vec<Tool> {
        Tool::ALL
            .into_iter()
            .filter(|tool| self.allowed(*tool))
            .collect()
    }

    fn allowed(&self, tool: Tool) -> bool {
        let installed = match tool {
            Tool::QueryMetrics => self.monitoring.metrics,
            Tool::QueryLogs => self.monitoring.logs,
            _ => true,
        };
        installed && tool.permission().is_none_or(|p| self.has(p))
    }
}

/// What the person sees about a tool call (never its result).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Source {
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub label: String,
}

/// The outcome of one tool call.
#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
    pub source: Source,
}

impl ToolOutcome {
    /// The `tool_result` block sent back to the model.
    pub fn result_block(&self, tool_use_id: &str) -> Value {
        let mut block = json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": self.content,
        });
        if self.is_error {
            block["is_error"] = json!(true);
        }
        block
    }
}

/// The app a call is about, if any (for status lines, sources and audit).
fn target_of(input: &Value) -> Option<String> {
    input
        .get("app")
        .or_else(|| input.get("name"))
        .and_then(Value::as_str)
        .filter(|app| is_app_name(app))
        .map(str::to_string)
}

/// A short "looking at…" line for the UI.
pub fn status_line(name: &str, input: Option<&Value>) -> String {
    let target = input.and_then(target_of);
    let what = match (Tool::parse(name), target) {
        (Some(Tool::ListApps), _) => "installed apps".to_string(),
        (Some(Tool::GetApp), Some(app)) => app,
        (Some(Tool::ListOperations), Some(app)) => format!("recent operations of {app}"),
        (Some(Tool::ListOperations), None) => "recent operations".to_string(),
        (Some(Tool::ListNotifications), _) => "your notifications".to_string(),
        (Some(Tool::GetStorage), _) => "storage usage".to_string(),
        (Some(Tool::GetUpdateSettings), _) => "automatic update settings".to_string(),
        (Some(Tool::SearchCatalog), _) => "the app catalog".to_string(),
        (Some(Tool::QueryMetrics), Some(app)) => format!("metrics of {app}"),
        (Some(Tool::QueryLogs), Some(app)) => format!("logs of {app}"),
        _ => "Kubarr".to_string(),
    };
    format!("Looking at {what}…")
}

fn source_for(name: &str, input: Option<&Value>) -> Source {
    let target = input.and_then(target_of);
    let label = match (Tool::parse(name), &target) {
        (Some(Tool::ListApps), _) => "Installed apps".to_string(),
        (Some(Tool::GetApp), Some(app)) => format!("App {app}"),
        (Some(Tool::ListOperations), Some(app)) => format!("Operations of {app}"),
        (Some(Tool::ListOperations), None) => "Operations".to_string(),
        (Some(Tool::ListNotifications), _) => "Your notifications".to_string(),
        (Some(Tool::GetStorage), _) => "Storage".to_string(),
        (Some(Tool::GetUpdateSettings), _) => "Update settings".to_string(),
        (Some(Tool::SearchCatalog), _) => "App catalog".to_string(),
        (Some(Tool::QueryMetrics), Some(app)) => format!("Metrics of {app}"),
        (Some(Tool::QueryLogs), Some(app)) => format!("Logs of {app}"),
        _ => name.to_string(),
    };
    Source {
        tool: name.to_string(),
        target,
        label,
    }
}

fn is_app_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Check `input` against the tool's schema: an object, only known
/// properties, every required one present, and each of the right type (and
/// in its enum).
pub fn validate(tool: Tool, input: &Value) -> Result<(), String> {
    let schema = tool.input_schema();
    let Some(object) = input.as_object() else {
        return Err("input must be an object".into());
    };
    let properties = schema["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for (key, value) in object {
        let Some(property) = properties.get(key) else {
            return Err(format!("unknown parameter `{key}`"));
        };
        let ok = match property["type"].as_str() {
            Some("string") => value.is_string(),
            Some("integer") => value.is_i64() || value.is_u64(),
            Some("boolean") => value.is_boolean(),
            _ => false,
        };
        if !ok {
            return Err(format!("parameter `{key}` has the wrong type"));
        }
        if let Some(allowed) = property["enum"].as_array() {
            if !allowed.contains(value) {
                return Err(format!(
                    "parameter `{key}` is not one of the allowed values"
                ));
            }
        }
    }
    for required in schema["required"].as_array().into_iter().flatten() {
        if let Some(key) = required.as_str() {
            if !object.contains_key(key) {
                return Err(format!("missing parameter `{key}`"));
            }
        }
    }
    Ok(())
}

/// Cut `text` to [`MAX_RESULT_BYTES`] on a character boundary, with a marker.
pub fn truncate_result(text: String) -> String {
    if text.len() <= MAX_RESULT_BYTES {
        return text;
    }
    let mut end = MAX_RESULT_BYTES - TRUNCATED_MARKER.len();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &text[..end], TRUNCATED_MARKER)
}

fn clip(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

fn scrubbed(text: Option<&str>) -> Value {
    text.map(|t| Value::String(clip(&scrub_inline(t), MAX_FIELD_CHARS)))
        .unwrap_or(Value::Null)
}

fn limit_of(input: &Value, default: u64, max: u64) -> u64 {
    input["limit"].as_u64().unwrap_or(default).clamp(1, max)
}

/// A failure the model may see: fixed wording, no internal detail.
struct ToolError(String);

impl ToolError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl From<crate::error::AppError> for ToolError {
    fn from(e: crate::error::AppError) -> Self {
        tracing::debug!(error = %e, "Assistant tool failed");
        ToolError::new("Kubarr could not read this data right now.")
    }
}

impl From<sea_orm::DbErr> for ToolError {
    fn from(e: sea_orm::DbErr) -> Self {
        crate::error::AppError::from(e).into()
    }
}

type ToolResult = Result<Value, ToolError>;

fn not_found(app: &str) -> ToolError {
    ToolError::new(format!("No app named \"{app}\" is visible to this user."))
}

/// Run one tool call and audit it.
pub async fn execute(ctx: &ToolContext, name: &str, input: Option<&Value>) -> ToolOutcome {
    let source = source_for(name, input);
    let result = match (Tool::parse(name), input) {
        (None, _) => Err(ToolError::new(format!("Unknown tool `{name}`."))),
        (Some(_), None) => Err(ToolError::new(
            "The tool input was not valid JSON. Call the tool again with valid input.",
        )),
        (Some(tool), Some(_)) if !ctx.allowed(tool) => {
            Err(ToolError::new("This tool is not available to this user."))
        }
        (Some(tool), Some(input)) => match validate(tool, input) {
            Err(e) => Err(ToolError::new(format!("Invalid input: {e}."))),
            Ok(()) => run(ctx, tool, input).await,
        },
    };
    let (content, is_error) = match result {
        Ok(value) => (truncate_result(value.to_string()), false),
        Err(ToolError(message)) => (message, true),
    };
    audit(ctx, name, source.target.as_deref(), !is_error).await;
    ToolOutcome {
        content,
        is_error,
        source,
    }
}

async fn audit(ctx: &ToolContext, tool: &str, target: Option<&str>, success: bool) {
    let mut details = json!({"tool": tool, "conversation_id": ctx.conversation_id});
    if let Some(target) = target {
        details["target"] = json!(target);
    }
    if let Err(e) = ctx
        .state
        .audit
        .log(
            AuditAction::AssistantToolCall,
            ResourceType::System,
            Some("assistant".into()),
            Some(ctx.user_id),
            Some(ctx.username.clone()),
            Some(details),
            None,
            None,
            success,
            None,
        )
        .await
    {
        tracing::warn!("Failed to audit an assistant tool call: {e}");
    }
}

async fn run(ctx: &ToolContext, tool: Tool, input: &Value) -> ToolResult {
    match tool {
        Tool::ListApps => list_apps(ctx).await,
        Tool::GetApp => get_app(ctx, input["name"].as_str().unwrap_or_default()).await,
        Tool::ListOperations => list_operations(ctx, input).await,
        Tool::ListNotifications => list_notifications(ctx, input).await,
        Tool::GetStorage => get_storage(ctx).await,
        Tool::GetUpdateSettings => get_update_settings(ctx).await,
        Tool::SearchCatalog => {
            search_catalog(ctx, input["query"].as_str().unwrap_or_default()).await
        }
        Tool::QueryMetrics => query_metrics(ctx, input).await,
        Tool::QueryLogs => query_logs(ctx, input).await,
    }
}

fn app_json(state: app_state::Model) -> Value {
    let message = state.message.clone();
    let response = AppStateResponse::from(state);
    json!({
        "name": response.app_name,
        "namespace": response.namespace,
        "desired_state": response.desired_state,
        "observed_state": response.observed_state,
        "healthy": response.healthy,
        "message": scrubbed(message.as_deref()),
        "installed_version": response.installed_chart_version,
        "available_version": response.available_chart_version,
        "update_available": response.update_available,
        "update_kind": response.available_update_kind,
        "auto_update": response.auto_update,
        "last_checked_at": response.last_checked_at,
    })
}

fn operation_json(op: app_operation::Model) -> Value {
    json!({
        "id": op.id,
        "app": op.app_name,
        "operation": op.operation,
        "status": op.status,
        "trigger": op.trigger,
        "message": scrubbed(op.message.as_deref()),
        "error": scrubbed(op.error.as_deref()),
        "diagnostics": scrubbed(op.diagnostics.as_deref()),
        "attempts": op.attempts,
        "created_at": op.created_at,
        "finished_at": op.finished_at,
    })
}

async fn visible_app_state(ctx: &ToolContext, app: &str) -> Result<app_state::Model, ToolError> {
    if !is_app_name(app) || !ctx.sees_app(app) {
        return Err(not_found(app));
    }
    app_state::Entity::find_by_id(app.to_string())
        .one(&ctx.db)
        .await?
        .ok_or_else(|| not_found(app))
}

async fn list_apps(ctx: &ToolContext) -> ToolResult {
    let states = app_state::Entity::find()
        .order_by_asc(app_state::Column::AppName)
        .all(&ctx.db)
        .await?;
    let apps: Vec<Value> = states
        .into_iter()
        .filter(|state| ctx.sees_app(&state.app_name))
        .map(app_json)
        .collect();
    Ok(json!({ "apps": apps }))
}

async fn get_app(ctx: &ToolContext, name: &str) -> ToolResult {
    if !is_app_name(name) || !ctx.sees_app(name) {
        return Err(not_found(name));
    }
    let state = app_state::Entity::find_by_id(name.to_string())
        .one(&ctx.db)
        .await?;
    let catalog_entry = {
        let catalog = ctx.state.catalog.read().await;
        catalog
            .get_app(name)
            .filter(|app| !app.is_hidden)
            .map(|app| {
                json!({
                    "display_name": app.display_name,
                    "description": clip(&app.description, 500),
                    "category": app.category,
                    "system_app": app.is_system,
                    "catalog_version": catalog.chart_version(name),
                })
            })
    };
    if state.is_none() && catalog_entry.is_none() {
        return Err(not_found(name));
    }
    let operations: Vec<Value> = app_operation::Entity::find()
        .filter(app_operation::Column::AppName.eq(name))
        .order_by_desc(app_operation::Column::CreatedAt)
        .order_by_desc(app_operation::Column::Id)
        .limit(5)
        .all(&ctx.db)
        .await?
        .into_iter()
        .map(operation_json)
        .collect();
    Ok(json!({
        "name": name,
        "installed": state.is_some(),
        "catalog": catalog_entry,
        "state": state.map(app_json),
        "recent_operations": operations,
    }))
}

async fn list_operations(ctx: &ToolContext, input: &Value) -> ToolResult {
    let limit = limit_of(input, 10, 50);
    let mut query = app_operation::Entity::find()
        .order_by_desc(app_operation::Column::CreatedAt)
        .order_by_desc(app_operation::Column::Id)
        .limit(limit);
    if let Some(app) = input["app"].as_str() {
        if !is_app_name(app) || !ctx.sees_app(app) {
            return Err(not_found(app));
        }
        query = query.filter(app_operation::Column::AppName.eq(app));
    } else if let Some(apps) = ctx.visible_app_names() {
        query = query.filter(app_operation::Column::AppName.is_in(apps));
    }
    let operations: Vec<Value> = query
        .all(&ctx.db)
        .await?
        .into_iter()
        .filter(|op| ctx.sees_app(&op.app_name))
        .map(operation_json)
        .collect();
    Ok(json!({ "operations": operations }))
}

async fn list_notifications(ctx: &ToolContext, input: &Value) -> ToolResult {
    let limit = limit_of(input, 10, 50) as usize;
    let unread_only = input["unread_only"].as_bool().unwrap_or(false);
    let notifications: Vec<Value> = ctx
        .state
        .notification
        .get_user_notifications(ctx.user_id, 100, 0)
        .await?
        .into_iter()
        .filter(|n| !unread_only || !n.read)
        .take(limit)
        .map(|n| {
            json!({
                "title": n.title,
                "message": scrubbed(Some(&n.message)),
                "event_type": n.event_type,
                "severity": n.severity,
                "read": n.read,
                "created_at": n.created_at,
            })
        })
        .collect();
    Ok(json!({ "notifications": notifications }))
}

async fn get_storage(ctx: &ToolContext) -> ToolResult {
    let stats = crate::endpoints::storage::storage_stats(&ctx.db).await?;
    Ok(json!({
        "total_bytes": stats.total_bytes,
        "used_bytes": stats.used_bytes,
        "free_bytes": stats.free_bytes,
        "usage_percent": stats.usage_percent,
    }))
}

async fn get_update_settings(ctx: &ToolContext) -> ToolResult {
    use crate::services::auto_update_settings;
    let settings = auto_update_settings::load(&ctx.db).await?;
    let timezone_saved = auto_update_settings::timezone_saved(&ctx.db).await?;
    let next = settings.next_window_start(Utc::now());
    let mut value = serde_json::to_value(&settings)
        .map_err(|_| ToolError::new("Kubarr could not read this data right now."))?;
    value["next_window_start"] = json!(next);
    value["timezone_saved"] = json!(timezone_saved);
    Ok(value)
}

async fn search_catalog(ctx: &ToolContext, query: &str) -> ToolResult {
    let words: Vec<String> = query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|w| !w.is_empty())
        .collect();
    let installed: std::collections::HashSet<String> = app_state::Entity::find()
        .filter(app_state::Column::DesiredState.eq(DESIRED_INSTALLED))
        .all(&ctx.db)
        .await?
        .into_iter()
        .map(|state| state.app_name)
        .collect();
    let catalog = ctx.state.catalog.read().await;
    let mut matches: Vec<Value> = catalog
        .get_all_apps()
        .into_iter()
        .filter(|app| !app.is_hidden)
        .filter(|app| {
            let haystack = format!(
                "{} {} {} {}",
                app.name, app.display_name, app.description, app.category
            )
            .to_lowercase();
            words.iter().all(|w| haystack.contains(w.as_str()))
        })
        .map(|app| {
            let mut entry = json!({
                "name": app.name,
                "display_name": app.display_name,
                "category": app.category,
                "description": clip(&app.description, 300),
                "version": catalog.chart_version(&app.name),
            });
            if ctx.sees_app(&app.name) {
                entry["installed"] = json!(installed.contains(&app.name));
            }
            entry
        })
        .collect();
    matches.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    matches.truncate(20);
    Ok(json!({ "apps": matches }))
}

fn range_seconds(range: &str) -> i64 {
    match range {
        "15m" => 15 * 60,
        "6h" => 6 * 3600,
        "24h" => 24 * 3600,
        _ => 3600,
    }
}

/// The named metric queries; `namespace` is a validated app namespace.
fn metric_query(metric: &str, namespace: &str) -> Option<(String, &'static str)> {
    let q = match metric {
        "cpu" => (
            format!(
                r#"sum(rate(container_cpu_usage_seconds_total{{namespace="{namespace}",container!="",container!="POD"}}[5m]))"#
            ),
            "cores",
        ),
        "memory" => (
            format!(
                r#"sum(container_memory_working_set_bytes{{namespace="{namespace}",container!="",container!="POD"}})"#
            ),
            "bytes",
        ),
        "network_receive" => (
            format!(
                r#"sum(rate(container_network_receive_bytes_total{{namespace="{namespace}",interface!="lo"}}[5m]))"#
            ),
            "bytes_per_second",
        ),
        "network_transmit" => (
            format!(
                r#"sum(rate(container_network_transmit_bytes_total{{namespace="{namespace}",interface!="lo"}}[5m]))"#
            ),
            "bytes_per_second",
        ),
        "restarts" => (
            format!(r#"sum(kube_pod_container_status_restarts_total{{namespace="{namespace}"}})"#),
            "restarts_total",
        ),
        _ => return None,
    };
    Some(q)
}

/// Summary statistics and at most `max_points` evenly spaced samples.
pub fn summarize_series(points: &[(f64, f64)], max_points: usize) -> Value {
    if points.is_empty() {
        return json!({"points": 0});
    }
    let values: Vec<f64> = points.iter().map(|p| p.1).collect();
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let avg = values.iter().sum::<f64>() / values.len() as f64;
    let step = points.len().div_ceil(max_points.max(1));
    let series: Vec<Value> = points
        .iter()
        .step_by(step.max(1))
        .map(|(ts, v)| {
            let at = DateTime::<Utc>::from_timestamp(*ts as i64, 0)
                .map(|t| t.to_rfc3339())
                .unwrap_or_default();
            json!([at, round(*v)])
        })
        .collect();
    json!({
        "points": points.len(),
        "latest": round(values[values.len() - 1]),
        "min": round(min),
        "max": round(max),
        "avg": round(avg),
        "series": series,
    })
}

fn round(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

/// Namespaces of the platform and the cluster itself.
pub fn system_namespace(namespace: &str) -> bool {
    namespace.starts_with("kubarr")
        || namespace.starts_with("kube-")
        || namespace.starts_with("victoria")
        || matches!(
            namespace,
            "default" | "cnpg-system" | "cert-manager" | "monitoring" | "kube-system"
        )
}

const ADMIN_ONLY_NAMESPACE: &str =
    "Metrics and logs of system or shared namespaces are only available to administrators.";

/// The namespace an app's metrics or logs may be read from. Non-admins only
/// get namespaces that are owned by exactly this one app and are not system
/// namespaces (and not of a catalog system app).
async fn scoped_namespace(ctx: &ToolContext, app: &str) -> Result<String, ToolError> {
    let state = visible_app_state(ctx, app).await?;
    let namespace = state.namespace;
    if !is_app_name(&namespace) {
        return Err(not_found(app));
    }
    if ctx.is_admin() {
        return Ok(namespace);
    }
    let system_app = ctx
        .state
        .catalog
        .read()
        .await
        .get_app(app)
        .is_some_and(|config| config.is_system);
    let owners = app_state::Entity::find()
        .filter(app_state::Column::Namespace.eq(namespace.as_str()))
        .count(&ctx.db)
        .await?;
    if system_app || system_namespace(&namespace) || owners != 1 {
        return Err(ToolError::new(ADMIN_ONLY_NAMESPACE));
    }
    Ok(namespace)
}

async fn query_metrics(ctx: &ToolContext, input: &Value) -> ToolResult {
    let app = input["app"].as_str().unwrap_or_default();
    let namespace = scoped_namespace(ctx, app).await?;
    let metric = input["metric"].as_str().unwrap_or_default();
    let range = input["range"].as_str().unwrap_or("1h");
    let Some((query, unit)) = metric_query(metric, &namespace) else {
        return Err(ToolError::new("Unknown metric."));
    };
    let seconds = range_seconds(range);
    let end = Utc::now().timestamp() as f64;
    let start = end - seconds as f64;
    let step = format!("{}s", (seconds / 60).max(15));
    let results =
        crate::endpoints::monitoring::query_vm_range_with_timeout(&query, start, end, &step, 10)
            .await;
    let points: Vec<(f64, f64)> = results
        .first()
        .and_then(|r| r["values"].as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|v| Some((v[0].as_f64()?, v[1].as_str()?.parse().ok()?)))
                .collect()
        })
        .unwrap_or_default();
    let mut summary = summarize_series(&points, 24);
    summary["app"] = json!(app);
    summary["metric"] = json!(metric);
    summary["unit"] = json!(unit);
    summary["range"] = json!(range);
    if points.is_empty() {
        summary["note"] = json!("No data: VictoriaMetrics returned no samples for this range.");
    }
    Ok(summary)
}

/// A LogsQL phrase filter for `text` (quotes and backslashes escaped).
fn logsql_phrase(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

async fn query_logs(ctx: &ToolContext, input: &Value) -> ToolResult {
    let app = input["app"].as_str().unwrap_or_default();
    let namespace = scoped_namespace(ctx, app).await?;
    let since = input["since"].as_str().unwrap_or("1h");
    let limit = limit_of(input, 50, 200);
    let mut query = format!("namespace:\"{namespace}\"");
    if let Some(contains) = input["contains"].as_str().filter(|c| !c.trim().is_empty()) {
        let contains: String = contains.chars().take(200).collect();
        query.push(' ');
        query.push_str(&logsql_phrase(&contains));
    }
    let now = Utc::now();
    let params = crate::endpoints::logs::VLogsQueryParams {
        query,
        start: Some((now - Duration::seconds(range_seconds(since))).to_rfc3339()),
        end: Some(now.to_rfc3339()),
        limit: i32::try_from(limit).unwrap_or(50),
    };
    let response = crate::endpoints::logs::fetch_vlogs(params)
        .await
        .map_err(|e| {
            tracing::debug!(error = %e, "Assistant log query failed");
            ToolError::new("VictoriaLogs could not be queried right now.")
        })?;
    let mut entries: Vec<(String, String, String)> = response
        .streams
        .into_iter()
        .flat_map(|stream| {
            let pod = stream.labels.get("pod").cloned().unwrap_or_default();
            stream
                .entries
                .into_iter()
                .map(move |entry| (entry.timestamp, pod.clone(), entry.line))
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0));
    entries.truncate(limit as usize);
    let secrets = match ctx.state.k8s_client.read().await.as_ref() {
        Some(k8s) => secret_values_for_namespace(k8s, &namespace).await,
        None => Vec::new(),
    };
    // Scrub each whole line before clipping, so a clip can never cut a
    // secret in half and leave a recognisable prefix.
    let lines: Vec<String> = entries
        .iter()
        .map(|(ts, pod, line)| {
            let clean = scrub_with(line, &secrets);
            format!("{ts} {pod} {}", clip(&clean, MAX_LOG_LINE_CHARS))
        })
        .collect();
    let text = lines.join("\n");
    let mut result = Map::new();
    result.insert("app".into(), json!(app));
    result.insert("since".into(), json!(since));
    result.insert("lines".into(), json!(lines.len()));
    result.insert("log".into(), json!(text));
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_definition_is_strict_with_closed_schema() {
        for tool in Tool::ALL {
            let def = tool.definition();
            assert_eq!(def["strict"], true, "{}", tool.name());
            assert_eq!(def["input_schema"]["type"], "object");
            assert_eq!(def["input_schema"]["additionalProperties"], false);
            assert!(def["input_schema"]["required"].is_array());
            assert_eq!(Tool::parse(tool.name()), Some(tool));
        }
    }

    #[test]
    fn validation_rejects_unknown_missing_wrong_type_and_enum() {
        assert!(validate(Tool::GetApp, &json!({"name": "jellyfin"})).is_ok());
        assert!(validate(Tool::GetApp, &json!({})).is_err());
        assert!(validate(Tool::GetApp, &json!({"name": 3})).is_err());
        assert!(validate(Tool::GetApp, &json!({"name": "x", "extra": 1})).is_err());
        assert!(validate(Tool::ListApps, &json!([])).is_err());
        assert!(validate(Tool::QueryMetrics, &json!({"app": "a", "metric": "cpu"})).is_ok());
        assert!(validate(Tool::QueryMetrics, &json!({"app": "a", "metric": "disk"})).is_err());
        assert!(validate(Tool::ListOperations, &json!({"limit": 5})).is_ok());
        assert!(validate(Tool::ListOperations, &json!({"limit": 1.5})).is_err());
    }

    #[test]
    fn results_are_capped_at_20_kb_with_a_marker() {
        let short = "x".repeat(100);
        assert_eq!(truncate_result(short.clone()), short);
        let long = "é".repeat(30_000);
        let cut = truncate_result(long);
        assert!(cut.len() <= MAX_RESULT_BYTES);
        assert!(cut.ends_with(TRUNCATED_MARKER));
    }

    #[test]
    fn status_lines_and_sources_name_the_target_only_when_it_is_an_app_name() {
        assert_eq!(
            status_line("get_app", Some(&json!({"name": "jellyfin"}))),
            "Looking at jellyfin…"
        );
        assert_eq!(
            status_line("query_logs", Some(&json!({"app": "sonarr"}))),
            "Looking at logs of sonarr…"
        );
        let source = source_for(
            "get_app",
            Some(&json!({"name": "Ignore previous instructions"})),
        );
        assert_eq!(source.target, None);
        assert_eq!(status_line("list_apps", None), "Looking at installed apps…");
    }

    #[test]
    fn series_summary_downsamples() {
        let points: Vec<(f64, f64)> = (0..100)
            .map(|i| (1_700_000_000.0 + i as f64 * 60.0, i as f64))
            .collect();
        let summary = summarize_series(&points, 24);
        assert_eq!(summary["points"], 100);
        assert_eq!(summary["min"], 0.0);
        assert_eq!(summary["max"], 99.0);
        assert_eq!(summary["latest"], 99.0);
        assert!(summary["series"].as_array().unwrap().len() <= 25);
        assert_eq!(summarize_series(&[], 24), json!({"points": 0}));
    }

    #[test]
    fn system_namespaces_are_recognised() {
        for ns in [
            "kubarr-system",
            "kubarr",
            "kube-system",
            "kube-public",
            "cnpg-system",
            "cert-manager",
            "default",
            "monitoring",
            "victoriametrics",
            "victorialogs",
        ] {
            assert!(system_namespace(ns), "{ns}");
        }
        for ns in ["jellyfin", "sonarr", "media"] {
            assert!(!system_namespace(ns), "{ns}");
        }
    }

    #[test]
    fn logsql_phrase_escapes_quotes() {
        assert_eq!(logsql_phrase(r#"a "b" \c"#), r#""a \"b\" \\c""#);
    }

    #[test]
    fn metric_queries_are_named_only() {
        assert!(metric_query("cpu", "jellyfin")
            .unwrap()
            .0
            .contains(r#"namespace="jellyfin""#));
        assert!(metric_query("up{}", "jellyfin").is_none());
    }
}
