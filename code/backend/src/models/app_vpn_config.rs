use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "app_vpn_configs")]
pub struct Model {
    /// App identifier (e.g., "qbittorrent", "transmission")
    #[sea_orm(primary_key, auto_increment = false)]
    pub app_name: String,
    /// Reference to vpn_providers.id
    pub vpn_provider_id: i64,
    /// Override the provider's kill_switch setting (None = use provider default)
    pub kill_switch_override: Option<bool>,
    /// Enable VPN port forwarding (NAT-PMP) for incoming connections
    pub port_forwarding: bool,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::vpn_provider::Entity",
        from = "Column::VpnProviderId",
        to = "super::vpn_provider::Column::Id"
    )]
    VpnProvider,
}

impl Related<super::vpn_provider::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::VpnProvider.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
