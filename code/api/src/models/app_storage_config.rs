use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "app_storage_configs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub app_name: String,
    pub schema_version: i32,
    pub chart_version: String,
    pub config_json: String,
    pub fresh_private_config: bool,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
