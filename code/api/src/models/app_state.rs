use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "app_states")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub app_name: String,
    pub namespace: String,
    pub desired_state: String,
    pub observed_state: String,
    pub healthy: bool,
    pub message: Option<String>,
    pub installed_chart_version: Option<String>,
    pub available_chart_version: Option<String>,
    pub update_available: bool,
    pub last_operation_id: Option<String>,
    pub last_checked_at: Option<DateTimeUtc>,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
