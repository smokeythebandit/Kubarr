//! A question or an answer in an AI assistant conversation.
//!
//! Only what the person saw is stored: the text, and for answers the sources
//! (tool names and targets). Tool results are never persisted.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// The answer finished normally.
pub const STATUS_COMPLETE: &str = "complete";
/// The answer is still being written by the replica serving the stream.
pub const STATUS_STREAMING: &str = "streaming";
/// The stream broke before the answer finished; `content_json` holds the
/// partial text.
pub const STATUS_INTERRUPTED: &str = "interrupted";
/// The answer failed (API error, budget, limits); `content_json.error` says why.
pub const STATUS_ERROR: &str = "error";

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "assistant_messages")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub conversation_id: String,
    /// `user` or `assistant`.
    pub role: String,
    /// `{"text": ..., "sources": [...], "error": ...}`.
    pub content_json: String,
    pub status: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
