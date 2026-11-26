//! Actions the assistant may propose, and the Run / Cancel that carry them out.
//!
//! The model can only *propose* an action (the `propose_action` tool). That
//! validates it against the user's permissions and the same rules the app
//! endpoints enforce, stores a pending action with a summary Kubarr computes
//! itself, and shows the person a card. Nothing happens until the person
//! clicks Run: `POST /api/assistant/actions/{id}/run` is the only executor,
//! it is not a tool, and it checks everything again at run time.
//!
//! A pending action belongs to the user of the conversation (any of their
//! sessions may run it), runs at most once (a conditional `pending → running`
//! update), and expires after [`ACTION_TTL_MINUTES`] minutes. Work goes
//! through Kubarr's operation queue, so the usual health checks (and the
//! automatic rollback of failed updates) apply.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::conversations::MessageContent;
use super::tools::{is_app_name, ToolContext};
use crate::error::{AppError, Result};
use crate::models::assistant_pending_action::{
    self, STATUS_CANCELLED, STATUS_DONE, STATUS_EXPIRED, STATUS_FAILED, STATUS_PENDING,
    STATUS_RUNNING,
};
use crate::models::audit_log::{AuditAction, ResourceType};
use crate::models::{app_operation, app_state, assistant_conversation, assistant_message};
use crate::services::app_install::{self, InstallChoices};
use crate::services::app_manager::{
    check_retry_allowed, AppManager, DESIRED_INSTALLED, OP_DELETE, OP_RESTART, OP_ROLLBACK,
    OP_UPDATE,
};
use crate::services::update_policy::{update_kind, UpdateKind};
use crate::state::AppState;

/// A proposed action can be run for this long.
pub const ACTION_TTL_MINUTES: i64 = 10;
/// The refusal for major updates (the owner's rule: majors always go
/// through the app page by hand).
pub const MAJOR_REFUSAL: &str = "Major updates go through the app page: open the app in Kubarr, \
review the release notes and click Update there.";

const ADMIN_PERMISSION: &str = "settings.manage";
const ASSISTANT_USE: &str = "assistant.use";
const APPS_INSTALL: &str = "apps.install";
const APPS_RESTART: &str = "apps.restart";

/// What the assistant may propose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    RestartApp,
    UpdateApp,
    RollbackApp,
    InstallApp,
    SetAppAutoUpdate,
    RetryOperation,
}

impl ActionKind {
    pub const ALL: [ActionKind; 6] = [
        ActionKind::RestartApp,
        ActionKind::UpdateApp,
        ActionKind::RollbackApp,
        ActionKind::InstallApp,
        ActionKind::SetAppAutoUpdate,
        ActionKind::RetryOperation,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ActionKind::RestartApp => "restart_app",
            ActionKind::UpdateApp => "update_app",
            ActionKind::RollbackApp => "rollback_app",
            ActionKind::InstallApp => "install_app",
            ActionKind::SetAppAutoUpdate => "set_app_auto_update",
            ActionKind::RetryOperation => "retry_operation",
        }
    }

    pub fn parse(name: &str) -> Option<ActionKind> {
        ActionKind::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// Who proposes or runs an action.
#[derive(Debug, Clone)]
pub struct Actor {
    pub user_id: i64,
    pub username: String,
    pub permissions: Vec<String>,
}

impl Actor {
    pub fn has(&self, permission: &str) -> bool {
        self.permissions.iter().any(|p| p == permission)
    }

    /// The allowed-apps model: `app.*`, `app.<name>`, or admin.
    pub fn sees_app(&self, app: &str) -> bool {
        self.has(ADMIN_PERMISSION) || self.has("app.*") || self.has(&format!("app.{app}"))
    }

    /// Whether any action could be proposed for this user at all.
    pub fn may_propose(&self) -> bool {
        self.has(APPS_INSTALL) || self.has(APPS_RESTART)
    }

    fn require(&self, permission: &str) -> Result<()> {
        if self.has(permission) {
            Ok(())
        } else {
            Err(AppError::Forbidden(format!(
                "You need the {permission} permission for this."
            )))
        }
    }
}

/// The card text, computed by Kubarr (never by the model).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ActionSummary {
    /// One line, e.g. "Update jellyfin".
    pub title: String,
    /// What will happen when the person clicks Run.
    pub effect: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_version: Option<String>,
}

/// The parameters a kind takes (all optional at the schema level; each kind
/// checks what it needs).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}

/// A validated action: what Run will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub kind: ActionKind,
    pub app: String,
    /// Normalised parameters (the version or operation Run will use).
    pub params: ActionParams,
    pub summary: ActionSummary,
}

fn not_visible(app: &str) -> AppError {
    AppError::NotFound(format!("No app named \"{app}\" is visible to you."))
}

async fn installed_state(
    db: &DatabaseConnection,
    actor: &Actor,
    app: &str,
) -> Result<app_state::Model> {
    app_state::Entity::find_by_id(app.to_string())
        .one(db)
        .await?
        .filter(|s| s.desired_state == DESIRED_INSTALLED)
        .ok_or_else(|| {
            if actor.sees_app(app) {
                AppError::Conflict(format!("{app} is not installed."))
            } else {
                not_visible(app)
            }
        })
}

fn in_catalog(catalog: &crate::services::catalog::AppCatalog, app: &str) -> Result<bool> {
    catalog
        .get_app(app)
        .map(|a| a.is_system)
        .ok_or_else(|| AppError::NotFound(format!("{app} is not in the Kubarr catalog.")))
}

/// The update a manual update would apply now: `(from, to, kind)`, using
/// the same target as the update endpoint (the chart on disk, else the
/// last known available version). `None` when there is nothing newer.
async fn offered_update(
    state: &AppState,
    db: &DatabaseConnection,
    current: &app_state::Model,
) -> Result<Option<(String, String, UpdateKind)>> {
    let manager = AppManager::new(db.clone(), state.k8s_client.clone(), state.catalog.clone());
    let target = manager.update_target(&current.app_name).await?;
    Ok(match (current.installed_chart_version.as_deref(), target) {
        (Some(from), Some(to)) if from != to => {
            let kind = update_kind(from, &to);
            Some((from.to_string(), to, kind))
        }
        _ => None,
    })
}

/// Refuse update kinds the assistant may not start: majors (always by hand)
/// and anything Kubarr cannot classify.
fn check_update_kind(kind: UpdateKind) -> Result<()> {
    match kind {
        UpdateKind::Revision | UpdateKind::Patch | UpdateKind::Minor => Ok(()),
        UpdateKind::Major => Err(AppError::Forbidden(MAJOR_REFUSAL.into())),
        UpdateKind::Downgrade | UpdateKind::Unknown => Err(AppError::Forbidden(
            "Kubarr cannot tell what kind of update this is, so it goes through the app page."
                .into(),
        )),
    }
}

fn kind_label(kind: UpdateKind) -> &'static str {
    match kind {
        UpdateKind::Revision => "revision",
        UpdateKind::Patch => "patch",
        UpdateKind::Minor => "minor",
        UpdateKind::Major => "major",
        UpdateKind::Downgrade => "downgrade",
        UpdateKind::Unknown => "unclassified",
    }
}

/// The version before the latest successful update, from its outcome
/// message (`Updated <app> <from> → <to>`), if Kubarr recorded one.
fn version_before_update(app: &str, message: Option<&str>) -> Option<String> {
    let rest = message?.strip_prefix(&format!("Updated {app} "))?;
    let (from, _) = rest.split_once(" → ")?;
    (!from.is_empty() && from.len() <= 64).then(|| from.to_string())
}

/// Validate a proposed (or about to run) action for `actor` against the same
/// rules the endpoints apply, and compute its summary.
pub async fn plan(
    state: &AppState,
    db: &DatabaseConnection,
    actor: &Actor,
    kind: ActionKind,
    app: &str,
    params: &ActionParams,
) -> Result<Plan> {
    if !is_app_name(app) {
        return Err(AppError::BadRequest(
            "app must be an app name such as \"jellyfin\".".into(),
        ));
    }
    actor.require(ASSISTANT_USE)?;
    let plan = |params: ActionParams, summary: ActionSummary| Plan {
        kind,
        app: app.to_string(),
        params,
        summary,
    };
    match kind {
        ActionKind::RestartApp => {
            actor.require(APPS_RESTART)?;
            if !actor.sees_app(app) {
                return Err(not_visible(app));
            }
            in_catalog(&*state.catalog.read().await, app)?;
            installed_state(db, actor, app).await?;
            Ok(plan(
                ActionParams::default(),
                ActionSummary {
                    title: format!("Restart {app}"),
                    effect: format!(
                        "Restarts the pods of {app}. It is unavailable for a moment while they \
                         come back."
                    ),
                    from_version: None,
                    to_version: None,
                },
            ))
        }
        ActionKind::UpdateApp => {
            actor.require(APPS_INSTALL)?;
            if !actor.sees_app(app) {
                return Err(not_visible(app));
            }
            in_catalog(&*state.catalog.read().await, app)?;
            let current = installed_state(db, actor, app).await?;
            let Some((from, to, update)) = offered_update(state, db, &current).await? else {
                return Err(AppError::Conflict(format!(
                    "{app} has no update available."
                )));
            };
            if let Some(version) = params.version.as_deref() {
                if version != to {
                    return Err(AppError::Conflict(format!(
                        "{app} can only be updated to the available version {to}, not {version}."
                    )));
                }
            }
            check_update_kind(update)?;
            Ok(plan(
                ActionParams {
                    version: Some(to.clone()),
                    ..Default::default()
                },
                ActionSummary {
                    title: format!("Update {app}"),
                    effect: format!(
                        "Updates {app} from {from} to {to} ({} update). Kubarr checks that it is \
                         healthy afterwards and rolls it back automatically if it is not.",
                        kind_label(update)
                    ),
                    from_version: Some(from),
                    to_version: Some(to),
                },
            ))
        }
        ActionKind::RollbackApp => {
            actor.require(APPS_INSTALL)?;
            if !actor.sees_app(app) {
                return Err(not_visible(app));
            }
            if in_catalog(&*state.catalog.read().await, app)? {
                return Err(AppError::Forbidden(format!(
                    "{app} is part of Kubarr itself; it is not rolled back from the assistant."
                )));
            }
            let current = installed_state(db, actor, app).await?;
            let last = app_operation::Entity::find()
                .filter(app_operation::Column::AppName.eq(app))
                .order_by_desc(app_operation::Column::CreatedAt)
                .order_by_desc(app_operation::Column::Id)
                .one(db)
                .await?;
            let Some(last) =
                last.filter(|op| op.operation == OP_UPDATE && op.status == "succeeded")
            else {
                return Err(AppError::Conflict(format!(
                    "{app}'s last change was not a successful update, so there is nothing to roll \
                     back."
                )));
            };
            let to = version_before_update(app, last.message.as_deref());
            let from = current.installed_chart_version.clone();
            Ok(plan(
                ActionParams::default(),
                ActionSummary {
                    title: format!("Roll back {app}"),
                    effect: format!(
                        "Rolls {app} back to the release it ran before its last update{}. Data \
                         is kept, but a one-way data migration the newer version ran is not \
                         undone. Kubarr checks that it is healthy afterwards.",
                        to.as_deref().map(|v| format!(" ({v})")).unwrap_or_default()
                    ),
                    from_version: from,
                    to_version: to,
                },
            ))
        }
        ActionKind::InstallApp => {
            actor.require(APPS_INSTALL)?;
            if !actor.sees_app(app) {
                return Err(not_visible(app));
            }
            let installed = app_state::Entity::find_by_id(app.to_string())
                .one(db)
                .await?
                .is_some_and(|s| s.desired_state == DESIRED_INSTALLED);
            if installed {
                return Err(AppError::Conflict(format!("{app} is already installed.")));
            }
            let version = app_install::installable_chart_version(&state.catalog, app).await?;
            if let Some(asked) = params.version.as_deref() {
                if asked != version {
                    return Err(AppError::Conflict(format!(
                        "{app} can only be installed at the catalog version {version}, not {asked}."
                    )));
                }
            }
            Ok(plan(
                ActionParams {
                    version: Some(version.clone()),
                    ..Default::default()
                },
                ActionSummary {
                    title: format!("Install {app}"),
                    effect: format!(
                        "Installs {app} {version} with the catalog's default settings (default \
                         storage, no GPU, no VPN changes). Use the Apps page to choose other \
                         settings."
                    ),
                    from_version: None,
                    to_version: Some(version),
                },
            ))
        }
        ActionKind::SetAppAutoUpdate => {
            actor.require(APPS_INSTALL)?;
            if !actor.sees_app(app) {
                return Err(not_visible(app));
            }
            let Some(enabled) = params.enabled else {
                return Err(AppError::BadRequest(
                    "params.enabled (true or false) is required.".into(),
                ));
            };
            let current = installed_state(db, actor, app).await?;
            if current.auto_update == enabled {
                return Err(AppError::Conflict(format!(
                    "Automatic updates are already {} for {app}.",
                    if enabled { "on" } else { "off" }
                )));
            }
            Ok(plan(
                ActionParams {
                    enabled: Some(enabled),
                    ..Default::default()
                },
                ActionSummary {
                    title: format!(
                        "Turn automatic updates {} for {app}",
                        if enabled { "on" } else { "off" }
                    ),
                    effect: if enabled {
                        format!(
                            "Kubarr will update {app} by itself during the update window. Major \
                             updates still wait for you."
                        )
                    } else {
                        format!("{app} will only be updated when someone clicks Update.")
                    },
                    from_version: None,
                    to_version: None,
                },
            ))
        }
        ActionKind::RetryOperation => {
            let Some(id) = params.operation_id.as_deref() else {
                return Err(AppError::BadRequest(
                    "params.operation_id is required.".into(),
                ));
            };
            let manager =
                AppManager::new(db.clone(), state.k8s_client.clone(), state.catalog.clone());
            let operation = match manager.get_operation(id).await {
                Ok(op) if op.app_name == app && actor.sees_app(app) => op,
                Ok(_) | Err(AppError::NotFound(_)) => {
                    return Err(AppError::NotFound(format!(
                        "No operation {id} of {app} is visible to you."
                    )))
                }
                Err(e) => return Err(e),
            };
            if operation.operation == OP_DELETE {
                return Err(AppError::Forbidden(
                    "The assistant does not uninstall apps; retry a removal from the Apps page."
                        .into(),
                ));
            }
            check_retry_allowed(&state.catalog, &operation, &actor.permissions).await?;
            if operation.status != "failed" {
                return Err(AppError::Conflict(format!(
                    "Only failed operations can be retried; this one is {}.",
                    operation.status
                )));
            }
            let mut summary = ActionSummary {
                title: format!("Retry the failed {} of {app}", operation.operation),
                effect: format!(
                    "Queues the {} of {app} again with the same settings.",
                    operation.operation
                ),
                from_version: None,
                to_version: None,
            };
            if operation.operation == OP_UPDATE {
                // A retried update deploys the version offered now.
                let current = installed_state(db, actor, app).await?;
                if let Some((from, to, update)) = offered_update(state, db, &current).await? {
                    check_update_kind(update)?;
                    summary.effect = format!(
                        "Updates {app} from {from} to {to} ({} update) again. Kubarr checks that \
                         it is healthy afterwards and rolls it back automatically if it is not.",
                        kind_label(update)
                    );
                    summary.from_version = Some(from);
                    summary.to_version = Some(to);
                }
            }
            Ok(plan(
                ActionParams {
                    operation_id: Some(id.to_string()),
                    ..Default::default()
                },
                summary,
            ))
        }
    }
}

/// The card the person sees.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct ActionCard {
    pub id: String,
    pub kind: String,
    pub app: String,
    /// The answer that proposed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<i64>,
    pub summary: ActionSummary,
    /// `pending`, `expired`, `running`, `done`, `failed` or `cancelled`.
    pub status: String,
    pub expires_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ActionCard {
    /// The card for a stored action, with `operation` (its queued operation,
    /// if any) deciding the outcome.
    pub fn from_model(
        model: assistant_pending_action::Model,
        operation: Option<&app_operation::Model>,
        now: DateTime<Utc>,
    ) -> ActionCard {
        let summary = serde_json::from_str(&model.summary).unwrap_or(ActionSummary {
            title: model.kind.clone(),
            effect: String::new(),
            from_version: None,
            to_version: None,
        });
        let mut error = model.error.clone();
        let status = match model.status.as_str() {
            STATUS_PENDING if model.expires_at <= now => STATUS_EXPIRED.to_string(),
            STATUS_RUNNING => match operation.map(|op| op.status.as_str()) {
                Some("succeeded") => STATUS_DONE.to_string(),
                Some("failed") => {
                    error = operation
                        .and_then(|op| op.error.clone().or_else(|| op.message.clone()))
                        .map(|e| super::tools::clip(&e, 300));
                    STATUS_FAILED.to_string()
                }
                Some("cancelled") => {
                    error = Some("The operation was cancelled.".into());
                    STATUS_FAILED.to_string()
                }
                _ => STATUS_RUNNING.to_string(),
            },
            other => other.to_string(),
        };
        ActionCard {
            id: model.id,
            kind: model.kind,
            app: model.target,
            message_id: model.message_id,
            summary,
            status,
            expires_at: model.expires_at,
            operation_id: model.operation_id,
            error,
            created_at: model.created_at,
        }
    }
}

/// The cards of a conversation, oldest first, with current outcomes.
pub async fn cards_for_conversation<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
) -> Result<Vec<ActionCard>> {
    let actions = assistant_pending_action::Entity::find()
        .filter(assistant_pending_action::Column::ConversationId.eq(conversation_id))
        .order_by_asc(assistant_pending_action::Column::CreatedAt)
        .all(db)
        .await?;
    let ids: Vec<String> = actions
        .iter()
        .filter_map(|a| a.operation_id.clone())
        .collect();
    let operations: HashMap<String, app_operation::Model> = if ids.is_empty() {
        HashMap::new()
    } else {
        app_operation::Entity::find()
            .filter(app_operation::Column::Id.is_in(ids))
            .all(db)
            .await?
            .into_iter()
            .map(|op| (op.id.clone(), op))
            .collect()
    };
    let now = Utc::now();
    Ok(actions
        .into_iter()
        .map(|a| {
            let op = a.operation_id.as_ref().and_then(|id| operations.get(id));
            ActionCard::from_model(a, op, now)
        })
        .collect())
}

async fn audit(
    state: &AppState,
    action: AuditAction,
    actor: (i64, &str),
    details: Value,
    success: bool,
    error: Option<String>,
) {
    if let Err(e) = state
        .audit
        .log(
            action,
            ResourceType::System,
            Some("assistant".into()),
            Some(actor.0),
            Some(actor.1.to_string()),
            Some(details),
            None,
            None,
            success,
            error,
        )
        .await
    {
        tracing::warn!("Failed to audit an assistant action: {e}");
    }
}

/// The sentence the person (and the model) may see for a refusal.
pub fn refusal_text(e: &AppError) -> String {
    match e {
        AppError::NotFound(m)
        | AppError::BadRequest(m)
        | AppError::Forbidden(m)
        | AppError::Conflict(m)
        | AppError::ServiceUnavailable(m) => m.clone(),
        other => {
            tracing::warn!(error = %other, "Assistant action failed");
            "Kubarr could not do this right now.".to_string()
        }
    }
}

/// The input of a `propose_action` call.
#[derive(Debug, Clone, Deserialize)]
pub struct Proposal {
    pub kind: String,
    pub app: String,
    #[serde(default)]
    pub params: ActionParams,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Validate and store a proposal; never acts. Returns the card, or the
/// reason it was refused (for the model).
pub async fn propose(
    ctx: &ToolContext,
    proposal: &Proposal,
) -> std::result::Result<ActionCard, String> {
    let actor = Actor {
        user_id: ctx.user_id,
        username: ctx.username.clone(),
        permissions: ctx.permissions.clone(),
    };
    let Some(kind) = ActionKind::parse(&proposal.kind) else {
        return Err(format!("Unknown action kind `{}`.", proposal.kind));
    };
    let mut details = json!({
        "conversation_id": ctx.conversation_id,
        "kind": kind.name(),
        "target": proposal.app,
    });
    let planned = plan(
        &ctx.state,
        &ctx.db,
        &actor,
        kind,
        &proposal.app,
        &proposal.params,
    )
    .await;
    let planned = match planned {
        Ok(p) => p,
        Err(e) => {
            let text = refusal_text(&e);
            audit(
                &ctx.state,
                AuditAction::AssistantActionProposed,
                (ctx.user_id, &ctx.username),
                details,
                false,
                Some(text.clone()),
            )
            .await;
            return Err(text);
        }
    };
    let now = Utc::now();
    let id = uuid::Uuid::new_v4().to_string();
    let model = assistant_pending_action::ActiveModel {
        id: Set(id.clone()),
        conversation_id: Set(ctx.conversation_id.clone()),
        message_id: Set(ctx.message_id),
        user_id: Set(ctx.user_id),
        kind: Set(kind.name().to_string()),
        target: Set(planned.app.clone()),
        params_json: Set(serde_json::to_string(&planned.params).unwrap_or_else(|_| "{}".into())),
        summary: Set(serde_json::to_string(&planned.summary).unwrap_or_else(|_| "{}".into())),
        status: Set(STATUS_PENDING.to_string()),
        operation_id: Set(None),
        error: Set(None),
        expires_at: Set(now + Duration::minutes(ACTION_TTL_MINUTES)),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&ctx.db)
    .await
    .map_err(|e| {
        tracing::warn!("Failed to store a proposed assistant action: {e}");
        "Kubarr could not store the proposal right now.".to_string()
    })?;
    details["action_id"] = json!(id);
    audit(
        &ctx.state,
        AuditAction::AssistantActionProposed,
        (ctx.user_id, &ctx.username),
        details,
        true,
        None,
    )
    .await;
    Ok(ActionCard::from_model(model, None, now))
}

/// The caller's own action; anyone else's reads as not found.
pub async fn owned(
    db: &DatabaseConnection,
    user_id: i64,
    id: &str,
) -> Result<assistant_pending_action::Model> {
    assistant_pending_action::Entity::find_by_id(id.to_string())
        .one(db)
        .await?
        .filter(|a| a.user_id == user_id)
        .ok_or_else(|| AppError::NotFound("Action not found".into()))
}

fn not_claimable(action: &assistant_pending_action::Model, now: DateTime<Utc>) -> AppError {
    if action.status == STATUS_PENDING && action.expires_at <= now {
        AppError::Conflict(
            "This action expired. Ask the assistant again if you still want it.".into(),
        )
    } else {
        AppError::Conflict(format!(
            "This action was already {}.",
            match action.status.as_str() {
                STATUS_CANCELLED => "cancelled",
                STATUS_PENDING => "changed",
                _ => "run",
            }
        ))
    }
}

/// The result of a Run click.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct RunResult {
    /// `running` (an operation is queued) or `done`.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}

/// Carry out a validated plan through Kubarr's own services. Returns the
/// queued operation's id, if the action queued one.
async fn execute(
    state: &AppState,
    db: &DatabaseConnection,
    actor: &Actor,
    plan: &Plan,
) -> Result<Option<String>> {
    let manager = AppManager::new(db.clone(), state.k8s_client.clone(), state.catalog.clone());
    let app = plan.app.as_str();
    let operation = match plan.kind {
        ActionKind::RestartApp => {
            manager
                .enqueue_operation(app, OP_RESTART, HashMap::new(), Some(actor.user_id))
                .await?
        }
        ActionKind::UpdateApp => {
            manager
                .enqueue_gpu_operation(app, OP_UPDATE, HashMap::new(), None, Some(actor.user_id))
                .await?
        }
        ActionKind::RollbackApp => {
            manager
                .enqueue_operation(app, OP_ROLLBACK, HashMap::new(), Some(actor.user_id))
                .await?
        }
        ActionKind::InstallApp => {
            app_install::install_with_choices(
                db,
                &state.catalog,
                &manager,
                app,
                InstallChoices {
                    chart_version: plan.params.version.clone(),
                    ..Default::default()
                },
                actor.user_id,
                &actor.permissions,
            )
            .await?
        }
        ActionKind::SetAppAutoUpdate => {
            manager
                .set_auto_update(app, plan.params.enabled.unwrap_or(false))
                .await?;
            return Ok(None);
        }
        ActionKind::RetryOperation => {
            let id = plan.params.operation_id.as_deref().unwrap_or_default();
            manager.retry_operation(id, actor.user_id).await?
        }
    };
    state.endpoint_cache.invalidate(app).await;
    Ok(Some(operation.id))
}

async fn append_message(db: &DatabaseConnection, conversation_id: &str, text: String) {
    let now = Utc::now();
    let inserted = assistant_message::ActiveModel {
        conversation_id: Set(conversation_id.to_string()),
        role: Set("assistant".into()),
        content_json: Set(MessageContent {
            text,
            ..Default::default()
        }
        .to_json()),
        status: Set(assistant_message::STATUS_COMPLETE.into()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await;
    if let Err(e) = inserted {
        tracing::warn!("Failed to add the action outcome to the conversation: {e}");
        return;
    }
    let touched = assistant_conversation::ActiveModel {
        id: Set(conversation_id.to_string()),
        updated_at: Set(now),
        ..Default::default()
    }
    .update(db)
    .await;
    if let Err(e) = touched {
        tracing::debug!("Failed to touch the conversation: {e}");
    }
}

async fn finish(
    db: &DatabaseConnection,
    id: &str,
    status: &str,
    operation_id: Option<String>,
    error: Option<String>,
) -> Result<()> {
    assistant_pending_action::ActiveModel {
        id: Set(id.to_string()),
        status: Set(status.to_string()),
        operation_id: Set(operation_id),
        error: Set(error),
        updated_at: Set(Utc::now()),
        ..Default::default()
    }
    .update(db)
    .await?;
    Ok(())
}

/// Run a pending action for its owner: claim it once, check everything
/// again with the user's current permissions, queue the work, audit it and
/// note the outcome in the conversation.
pub async fn run(
    state: &AppState,
    db: &DatabaseConnection,
    user_id: i64,
    username: &str,
    id: &str,
) -> Result<RunResult> {
    let action = owned(db, user_id, id).await?;
    let now = Utc::now();
    let claimed = assistant_pending_action::Entity::update_many()
        .col_expr(
            assistant_pending_action::Column::Status,
            STATUS_RUNNING.into(),
        )
        .col_expr(assistant_pending_action::Column::UpdatedAt, now.into())
        .filter(assistant_pending_action::Column::Id.eq(id))
        .filter(assistant_pending_action::Column::UserId.eq(user_id))
        .filter(assistant_pending_action::Column::Status.eq(STATUS_PENDING))
        .filter(assistant_pending_action::Column::ExpiresAt.gt(now))
        .exec(db)
        .await?;
    if claimed.rows_affected != 1 {
        let current = owned(db, user_id, id).await?;
        return Err(not_claimable(&current, now));
    }

    let actor = Actor {
        user_id,
        username: username.to_string(),
        permissions: crate::endpoints::extractors::get_user_permissions(db, user_id).await,
    };
    let stored: ActionParams = serde_json::from_str(&action.params_json).unwrap_or_default();
    let proposed: Option<ActionSummary> = serde_json::from_str(&action.summary).ok();
    let mut details = json!({
        "conversation_id": action.conversation_id,
        "action_id": action.id,
        "kind": action.kind,
        "target": action.target,
    });
    let outcome = async {
        let kind = ActionKind::parse(&action.kind)
            .ok_or_else(|| AppError::BadRequest("Unknown action kind".into()))?;
        let fresh = plan(state, db, &actor, kind, &action.target, &stored).await?;
        if fresh.params != stored
            || proposed
                .as_ref()
                .is_some_and(|p| p.to_version != fresh.summary.to_version)
        {
            return Err(AppError::Conflict(
                "Things changed since this was proposed (for example a newer version is \
                 available). Ask the assistant again."
                    .into(),
            ));
        }
        execute(state, db, &actor, &fresh)
            .await
            .map(|op| (fresh, op))
    }
    .await;

    match outcome {
        Ok((fresh, operation_id)) => {
            let status = if operation_id.is_some() {
                STATUS_RUNNING
            } else {
                STATUS_DONE
            };
            finish(db, id, status, operation_id.clone(), None).await?;
            if let Some(op) = &operation_id {
                details["operation_id"] = json!(op);
            }
            audit(
                state,
                AuditAction::AssistantActionConfirmed,
                (user_id, username),
                details,
                true,
                None,
            )
            .await;
            let text = match &operation_id {
                Some(op) => format!(
                    "_{username} clicked Run: {}. Kubarr queued it as operation `{op}`; the card \
                     shows the outcome when it finishes._",
                    fresh.summary.title
                ),
                None => format!("_{username} clicked Run: {}. Done._", fresh.summary.title),
            };
            append_message(db, &action.conversation_id, text).await;
            Ok(RunResult {
                status: status.to_string(),
                operation_id,
            })
        }
        Err(e) => {
            let text = refusal_text(&e);
            finish(db, id, STATUS_FAILED, None, Some(text.clone())).await?;
            audit(
                state,
                AuditAction::AssistantActionConfirmed,
                (user_id, username),
                details,
                false,
                Some(text.clone()),
            )
            .await;
            let title = proposed.map_or_else(|| action.kind.clone(), |p| p.title);
            append_message(
                db,
                &action.conversation_id,
                format!("_{username} clicked Run: {title}. It did not run: {text}_"),
            )
            .await;
            Err(e)
        }
    }
}

/// Cancel a pending action for its owner.
pub async fn cancel(
    state: &AppState,
    db: &DatabaseConnection,
    user_id: i64,
    username: &str,
    id: &str,
) -> Result<()> {
    let action = owned(db, user_id, id).await?;
    let now = Utc::now();
    let changed = assistant_pending_action::Entity::update_many()
        .col_expr(
            assistant_pending_action::Column::Status,
            STATUS_CANCELLED.into(),
        )
        .col_expr(assistant_pending_action::Column::UpdatedAt, now.into())
        .filter(assistant_pending_action::Column::Id.eq(id))
        .filter(assistant_pending_action::Column::UserId.eq(user_id))
        .filter(assistant_pending_action::Column::Status.eq(STATUS_PENDING))
        .exec(db)
        .await?;
    if changed.rows_affected != 1 {
        let current = owned(db, user_id, id).await?;
        return Err(not_claimable(&current, now));
    }
    audit(
        state,
        AuditAction::AssistantActionCancelled,
        (user_id, username),
        json!({
            "conversation_id": action.conversation_id,
            "action_id": action.id,
            "kind": action.kind,
            "target": action.target,
        }),
        true,
        None,
    )
    .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip() {
        for kind in ActionKind::ALL {
            assert_eq!(ActionKind::parse(kind.name()), Some(kind));
        }
        assert_eq!(ActionKind::parse("delete_app"), None);
    }

    #[test]
    fn majors_and_unclassified_updates_are_refused() {
        assert!(check_update_kind(UpdateKind::Patch).is_ok());
        assert!(check_update_kind(UpdateKind::Minor).is_ok());
        assert!(check_update_kind(UpdateKind::Revision).is_ok());
        assert!(matches!(
            check_update_kind(UpdateKind::Major),
            Err(AppError::Forbidden(m)) if m.starts_with("Major updates go through the app page")
        ));
        assert!(check_update_kind(UpdateKind::Downgrade).is_err());
        assert!(check_update_kind(UpdateKind::Unknown).is_err());
    }

    #[test]
    fn version_before_update_reads_the_outcome_message() {
        assert_eq!(
            version_before_update("jellyfin", Some("Updated jellyfin 1.2.3 → 1.3.0")),
            Some("1.2.3".into())
        );
        assert_eq!(
            version_before_update("jellyfin", Some("Restarted jellyfin")),
            None
        );
        assert_eq!(version_before_update("jellyfin", None), None);
    }

    fn model(status: &str, expires_in: i64) -> assistant_pending_action::Model {
        let now = Utc::now();
        assistant_pending_action::Model {
            id: "a".into(),
            conversation_id: "c".into(),
            message_id: Some(3),
            user_id: 1,
            kind: "restart_app".into(),
            target: "jellyfin".into(),
            params_json: "{}".into(),
            summary: r#"{"title":"Restart jellyfin","effect":"x"}"#.into(),
            status: status.into(),
            operation_id: Some("op".into()),
            error: None,
            expires_at: now + Duration::seconds(expires_in),
            created_at: now,
            updated_at: now,
        }
    }

    fn op(status: &str) -> app_operation::Model {
        let now = Utc::now();
        app_operation::Model {
            id: "op".into(),
            app_name: "jellyfin".into(),
            operation: "restart".into(),
            status: status.into(),
            stop_requested: false,
            message: Some("m".into()),
            error: Some("boom".into()),
            custom_config: None,
            attempts: 1,
            created_by: Some(1),
            trigger: Some("manual".into()),
            diagnostics: None,
            created_at: now,
            started_at: None,
            finished_at: None,
            updated_at: now,
        }
    }

    #[test]
    fn card_status_reflects_expiry_and_the_operation() {
        let now = Utc::now();
        assert_eq!(
            ActionCard::from_model(model("pending", 60), None, now).status,
            "pending"
        );
        assert_eq!(
            ActionCard::from_model(model("pending", -1), None, now).status,
            "expired"
        );
        assert_eq!(
            ActionCard::from_model(model("running", 60), Some(&op("running")), now).status,
            "running"
        );
        assert_eq!(
            ActionCard::from_model(model("running", 60), Some(&op("succeeded")), now).status,
            "done"
        );
        let failed = ActionCard::from_model(model("running", 60), Some(&op("failed")), now);
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.error.as_deref(), Some("boom"));
        assert_eq!(
            ActionCard::from_model(model("cancelled", 60), None, now).status,
            "cancelled"
        );
        assert_eq!(
            ActionCard::from_model(model("pending", 60), None, now)
                .summary
                .title,
            "Restart jellyfin"
        );
    }
}
