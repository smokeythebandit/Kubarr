use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "app_audit_outbox")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub operation_id: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub created_by: Option<i64>,
    pub username: Option<String>,
    pub details: String,
    pub success: bool,
    pub outcome: String,
    pub error_label: Option<String>,
    pub created_at: DateTimeUtc,
    pub processed_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
