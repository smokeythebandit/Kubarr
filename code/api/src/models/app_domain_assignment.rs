use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "app_domain_assignments")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub app_name: String,
    pub domain_id: i64,
    pub route_mode: String,
    pub hostname: Option<String>,
    pub path_prefix: Option<String>,
    pub primary: bool,
    pub enabled: bool,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
