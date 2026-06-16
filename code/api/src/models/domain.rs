use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "domains")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub domain: String,
    pub kind: String,
    pub scope: String,
    pub primary: bool,
    pub enabled: bool,
    pub dns_mode: String,
    pub ddns_profile_id: Option<i64>,
    pub dns_status: String,
    pub tls_mode: String,
    pub letsencrypt_profile_id: Option<i64>,
    pub tls_secret_name: Option<String>,
    pub certificate_status: String,
    pub certificate_expires_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
