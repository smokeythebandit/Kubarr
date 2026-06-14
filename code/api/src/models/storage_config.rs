use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "storage_config")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub mode: String,
    pub mount_path: String,
    pub uid: i32,
    pub gid: i32,
    pub fs_group: i32,
    pub config_json: String,
    pub validation_json: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
