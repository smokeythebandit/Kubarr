//! One row per Messages API call made for the AI assistant: tokens used and
//! what they cost, the basis of the monthly budget.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "assistant_usage")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// The user the call was made for; `None` for system calls (AI recovery).
    pub user_id: Option<i64>,
    pub conversation_id: Option<String>,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    /// Cost in millionths of a US dollar, rounded up.
    pub cost_usd_micros: i64,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
