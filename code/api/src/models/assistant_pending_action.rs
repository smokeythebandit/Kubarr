//! An action the AI assistant proposed, waiting for the person's Run or
//! Cancel click. Only the owner of the conversation can run or cancel it,
//! once, before it expires.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Proposed; waiting for Run or Cancel (expired once past `expires_at`).
pub const STATUS_PENDING: &str = "pending";
/// A Run click claimed it; an operation may be queued (`operation_id`).
pub const STATUS_RUNNING: &str = "running";
/// It ran to completion (for actions without an operation).
pub const STATUS_DONE: &str = "done";
/// It was refused at run time or could not be started; `error` says why.
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_CANCELLED: &str = "cancelled";
/// Shown, never stored: a pending action past `expires_at`.
pub const STATUS_EXPIRED: &str = "expired";

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "assistant_pending_actions")]
pub struct Model {
    /// UUID.
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub conversation_id: String,
    /// The answer that proposed it.
    pub message_id: Option<i64>,
    pub user_id: i64,
    /// `restart_app`, `update_app`, ...
    pub kind: String,
    /// The app (or, for `retry_operation`, the operation's app).
    pub target: String,
    /// Validated parameters (`{"version": ...}`, `{"enabled": ...}`, ...).
    pub params_json: String,
    /// The card summary computed by Kubarr, as JSON.
    pub summary: String,
    pub status: String,
    pub operation_id: Option<String>,
    pub error: Option<String>,
    pub expires_at: DateTimeUtc,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
