//! One AI assistant conversation, owned by the user who started it.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "assistant_conversations")]
pub struct Model {
    /// UUID.
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub user_id: i64,
    pub title: String,
    /// What the conversation is about (`{"app": ..., "page": ...}`), if anything.
    pub context_json: Option<String>,
    /// Running summary of the turns no longer sent to the model in full.
    pub summary: Option<String>,
    /// Id of the last message folded into `summary`.
    pub summary_through: Option<i64>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
