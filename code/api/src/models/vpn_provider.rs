use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// VPN provider type (WireGuard or OpenVPN)
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    EnumIter,
    DeriveActiveEnum,
    Serialize,
    Deserialize,
    utoipa::ToSchema,
)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::N(16))")]
pub enum VpnType {
    #[sea_orm(string_value = "wireguard")]
    #[serde(rename = "wireguard")]
    WireGuard,
    #[sea_orm(string_value = "openvpn")]
    #[serde(rename = "openvpn")]
    OpenVpn,
}

impl std::fmt::Display for VpnType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VpnType::WireGuard => write!(f, "wireguard"),
            VpnType::OpenVpn => write!(f, "openvpn"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "vpn_providers")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub name: String,
    pub vpn_type: VpnType,
    /// Service provider (e.g., "nordvpn", "mullvad", "custom")
    pub service_provider: Option<String>,
    /// JSON blob containing VPN credentials (encrypted later)
    #[serde(skip_serializing)]
    pub credentials_json: String,
    pub enabled: bool,
    pub kill_switch: bool,
    /// Comma-separated list of subnets to allow through firewall
    pub firewall_outbound_subnets: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::app_vpn_config::Entity")]
    AppVpnConfigs,
}

impl Related<super::app_vpn_config::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::AppVpnConfigs.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vpn_type_display() {
        assert_eq!(VpnType::WireGuard.to_string(), "wireguard");
        assert_eq!(VpnType::OpenVpn.to_string(), "openvpn");
    }

    #[test]
    fn vpn_type_serde() {
        let wg: VpnType = serde_json::from_str("\"wireguard\"").expect("deser");
        let ov: VpnType = serde_json::from_str("\"openvpn\"").expect("deser");
        assert_eq!(wg, VpnType::WireGuard);
        assert_eq!(ov, VpnType::OpenVpn);
        assert_eq!(
            serde_json::to_string(&VpnType::WireGuard).unwrap(),
            "\"wireguard\""
        );
        assert_eq!(
            serde_json::to_string(&VpnType::OpenVpn).unwrap(),
            "\"openvpn\""
        );
    }
}
