use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique)]
    pub username: String,
    #[sea_orm(unique)]
    pub email: String,
    #[serde(skip_serializing)]
    pub hashed_password: String,
    pub is_active: bool,
    pub is_approved: bool,
    #[serde(skip_serializing)]
    pub totp_secret: Option<String>,
    pub totp_enabled: bool,
    pub totp_verified_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::user_role::Entity")]
    UserRoles,
    #[sea_orm(has_one = "super::user_preferences::Entity")]
    Preferences,
}

impl Related<super::role::Entity> for Entity {
    fn to() -> RelationDef {
        super::user_role::Relation::Role.def()
    }
    fn via() -> Option<RelationDef> {
        Some(super::user_role::Relation::User.def().rev())
    }
}

impl Related<super::user_role::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UserRoles.def()
    }
}

impl Related<super::user_preferences::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Preferences.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
