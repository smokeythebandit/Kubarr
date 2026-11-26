use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Pending destination-verification code and send-rate window per (user, channel).
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "notification_verifications")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub user_id: i64,
    pub channel_type: String,
    /// `salt_hex$sha256_hex`; `None` when no code is pending.
    #[serde(skip_serializing)]
    pub code_hash: Option<String>,
    pub expires_at: Option<DateTimeUtc>,
    pub attempts: i32,
    pub last_sent_at: Option<DateTimeUtc>,
    pub window_started_at: Option<DateTimeUtc>,
    pub sends_in_window: i32,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id"
    )]
    User,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
