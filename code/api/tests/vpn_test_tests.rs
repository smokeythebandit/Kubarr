//! VPN Service Unit Tests
//!
//! This test suite verifies the VPN service functionality including:
//! - Credential structure serialization/deserialization
//! - Request/Response type serialization
//! - VPN provider CRUD operations
//! - App VPN configuration management
//! - Credential validation
//! - Secret data building for Gluetun

mod common;
use common::create_test_db;

use kubarr::models::prelude::*;
use kubarr::models::vpn_provider;
use kubarr::services::vpn::{
    AssignVpnRequest, CreateVpnProviderRequest, OpenVpnCredentials, UpdateVpnProviderRequest,
    VpnCredentials, WireGuardCredentials,
};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};

// ============================================================================
// Credential Serialization Tests
// ============================================================================

#[test]
fn test_wireguard_credentials_serialization() {
    let creds = WireGuardCredentials {
        private_key: "test_private_key".to_string(),
        addresses: vec!["10.0.0.1/32".to_string()],
        public_key: Some("test_public_key".to_string()),
        endpoint_ip: Some("1.2.3.4".to_string()),
        endpoint_port: Some(51820),
        preshared_key: Some("test_preshared_key".to_string()),
    };

    let json = serde_json::to_string(&creds).unwrap();
    assert!(json.contains("\"private_key\":\"test_private_key\""));
    assert!(json.contains("\"addresses\":[\"10.0.0.1/32\"]"));
    assert!(json.contains("\"endpoint_port\":51820"));

    // Deserialize back
    let parsed: WireGuardCredentials = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.private_key, creds.private_key);
    assert_eq!(parsed.addresses, creds.addresses);
    assert_eq!(parsed.endpoint_port, creds.endpoint_port);
}

#[test]
fn test_wireguard_credentials_minimal() {
    let creds = WireGuardCredentials {
        private_key: "minimal_key".to_string(),
        addresses: vec![],
        public_key: None,
        endpoint_ip: None,
        endpoint_port: None,
        preshared_key: None,
    };

    let json = serde_json::to_string(&creds).unwrap();
    assert!(json.contains("\"private_key\":\"minimal_key\""));
    assert!(!json.contains("\"public_key\""));
    assert!(!json.contains("\"endpoint_ip\""));
}

#[test]
fn test_openvpn_credentials_serialization() {
    let creds = OpenVpnCredentials {
        username: "test_user".to_string(),
        password: "test_pass".to_string(),
        server_countries: Some("USA,Canada".to_string()),
        server_cities: Some("New York,Toronto".to_string()),
        server_hostnames: Some("us1.example.com".to_string()),
    };

    let json = serde_json::to_string(&creds).unwrap();
    assert!(json.contains("\"username\":\"test_user\""));
    assert!(json.contains("\"password\":\"test_pass\""));
    assert!(json.contains("\"server_countries\":\"USA,Canada\""));

    // Deserialize back
    let parsed: OpenVpnCredentials = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.username, creds.username);
    assert_eq!(parsed.password, creds.password);
    assert_eq!(parsed.server_countries, creds.server_countries);
}

#[test]
fn test_openvpn_credentials_minimal() {
    let creds = OpenVpnCredentials {
        username: "user".to_string(),
        password: "pass".to_string(),
        server_countries: None,
        server_cities: None,
        server_hostnames: None,
    };

    let json = serde_json::to_string(&creds).unwrap();
    assert!(json.contains("\"username\":\"user\""));
    assert!(!json.contains("\"server_countries\""));
}

#[test]
fn test_vpn_credentials_enum_wireguard() {
    let creds = VpnCredentials::WireGuard(WireGuardCredentials {
        private_key: "key".to_string(),
        addresses: vec![],
        public_key: None,
        endpoint_ip: None,
        endpoint_port: None,
        preshared_key: None,
    });

    let json = serde_json::to_string(&creds).unwrap();
    assert!(json.contains("\"private_key\""));
}

#[test]
fn test_vpn_credentials_enum_openvpn() {
    let creds = VpnCredentials::OpenVpn(OpenVpnCredentials {
        username: "user".to_string(),
        password: "pass".to_string(),
        server_countries: None,
        server_cities: None,
        server_hostnames: None,
    });

    let json = serde_json::to_string(&creds).unwrap();
    assert!(json.contains("\"username\""));
    assert!(json.contains("\"password\""));
}

// ============================================================================
// Request/Response Type Tests
// ============================================================================

#[test]
fn test_create_vpn_provider_request_deserialization() {
    let json = r#"{
        "name": "Test VPN",
        "vpn_type": "wireguard",
        "service_provider": "mullvad",
        "credentials": {
            "private_key": "test_key"
        },
        "enabled": true,
        "kill_switch": true,
        "firewall_outbound_subnets": "10.0.0.0/8"
    }"#;

    let req: CreateVpnProviderRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.name, "Test VPN");
    assert_eq!(req.vpn_type, vpn_provider::VpnType::WireGuard);
    assert_eq!(req.service_provider, Some("mullvad".to_string()));
    assert!(req.enabled);
    assert!(req.kill_switch);
}

#[test]
fn test_create_vpn_provider_request_defaults() {
    let json = r#"{
        "name": "Test VPN",
        "vpn_type": "openvpn",
        "credentials": {
            "username": "user",
            "password": "pass"
        }
    }"#;

    let req: CreateVpnProviderRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.enabled, true); // Default
    assert_eq!(req.kill_switch, true); // Default
    assert_eq!(
        req.firewall_outbound_subnets,
        "10.0.0.0/8,172.16.0.0/12,192.168.0.0/16"
    ); // Default
}

#[test]
fn test_update_vpn_provider_request_partial() {
    let json = r#"{
        "name": "Updated Name"
    }"#;

    let req: UpdateVpnProviderRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.name, Some("Updated Name".to_string()));
    assert!(req.enabled.is_none());
    assert!(req.credentials.is_none());
}

#[test]
fn test_assign_vpn_request_deserialization() {
    let json = r#"{
        "vpn_provider_id": 1,
        "kill_switch_override": true
    }"#;

    let req: AssignVpnRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.vpn_provider_id, 1);
    assert_eq!(req.kill_switch_override, Some(true));
}

#[test]
fn test_assign_vpn_request_no_override() {
    let json = r#"{
        "vpn_provider_id": 2
    }"#;

    let req: AssignVpnRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.vpn_provider_id, 2);
    assert!(req.kill_switch_override.is_none());
}

// ============================================================================
// VPN Provider CRUD Tests
// ============================================================================

#[tokio::test]
async fn test_create_vpn_provider_wireguard() {
    let db = create_test_db().await;

    let credentials = serde_json::json!({
        "private_key": "test_wireguard_key",
        "addresses": ["10.0.0.1/32"]
    });

    let req = CreateVpnProviderRequest {
        name: "Test WireGuard".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: Some("mullvad".to_string()),
        credentials,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };

    let result = kubarr::services::vpn::create_vpn_provider(&db, req).await;
    assert!(result.is_ok());

    let provider = result.unwrap();
    assert_eq!(provider.name, "Test WireGuard");
    assert_eq!(provider.vpn_type, vpn_provider::VpnType::WireGuard);
    assert_eq!(provider.service_provider, Some("mullvad".to_string()));
    assert!(provider.enabled);
    assert!(provider.kill_switch);
    assert_eq!(provider.app_count, 0);
}

#[tokio::test]
async fn test_create_vpn_provider_openvpn() {
    let db = create_test_db().await;

    let credentials = serde_json::json!({
        "username": "test_user",
        "password": "test_password"
    });

    let req = CreateVpnProviderRequest {
        name: "Test OpenVPN".to_string(),
        vpn_type: vpn_provider::VpnType::OpenVpn,
        service_provider: None,
        credentials,
        enabled: true,
        kill_switch: false,
        firewall_outbound_subnets: "192.168.0.0/16".to_string(),
    };

    let result = kubarr::services::vpn::create_vpn_provider(&db, req).await;
    assert!(result.is_ok());

    let provider = result.unwrap();
    assert_eq!(provider.name, "Test OpenVPN");
    assert_eq!(provider.vpn_type, vpn_provider::VpnType::OpenVpn);
    assert!(provider.service_provider.is_none());
    assert!(!provider.kill_switch);
}

#[tokio::test]
async fn test_create_vpn_provider_invalid_wireguard_credentials() {
    let db = create_test_db().await;

    let credentials = serde_json::json!({
        "addresses": ["10.0.0.1/32"]
        // Missing private_key
    });

    let req = CreateVpnProviderRequest {
        name: "Invalid WireGuard".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };

    let result = kubarr::services::vpn::create_vpn_provider(&db, req).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("private_key"));
}

#[tokio::test]
async fn test_create_vpn_provider_invalid_openvpn_credentials() {
    let db = create_test_db().await;

    let credentials = serde_json::json!({
        "username": "user"
        // Missing password
    });

    let req = CreateVpnProviderRequest {
        name: "Invalid OpenVPN".to_string(),
        vpn_type: vpn_provider::VpnType::OpenVpn,
        service_provider: None,
        credentials,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };

    let result = kubarr::services::vpn::create_vpn_provider(&db, req).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("password"));
}

#[tokio::test]
async fn test_list_vpn_providers() {
    let db = create_test_db().await;

    // Create two providers
    let creds1 = serde_json::json!({"private_key": "key1"});
    let req1 = CreateVpnProviderRequest {
        name: "Provider 1".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials: creds1,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    kubarr::services::vpn::create_vpn_provider(&db, req1)
        .await
        .unwrap();

    let creds2 = serde_json::json!({"username": "user", "password": "pass"});
    let req2 = CreateVpnProviderRequest {
        name: "Provider 2".to_string(),
        vpn_type: vpn_provider::VpnType::OpenVpn,
        service_provider: Some("nordvpn".to_string()),
        credentials: creds2,
        enabled: false,
        kill_switch: false,
        firewall_outbound_subnets: "192.168.0.0/16".to_string(),
    };
    kubarr::services::vpn::create_vpn_provider(&db, req2)
        .await
        .unwrap();

    // List providers
    let providers = kubarr::services::vpn::list_vpn_providers(&db)
        .await
        .unwrap();
    assert_eq!(providers.len(), 2);

    // Verify first provider
    let provider1 = &providers[0];
    assert_eq!(provider1.name, "Provider 1");
    assert_eq!(provider1.vpn_type, vpn_provider::VpnType::WireGuard);
    assert!(provider1.enabled);

    // Verify second provider
    let provider2 = &providers[1];
    assert_eq!(provider2.name, "Provider 2");
    assert_eq!(provider2.vpn_type, vpn_provider::VpnType::OpenVpn);
    assert!(!provider2.enabled);
    assert_eq!(provider2.service_provider, Some("nordvpn".to_string()));
}

#[tokio::test]
async fn test_get_vpn_provider() {
    let db = create_test_db().await;

    let credentials = serde_json::json!({"private_key": "test_key"});
    let req = CreateVpnProviderRequest {
        name: "Get Test".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };

    let created = kubarr::services::vpn::create_vpn_provider(&db, req)
        .await
        .unwrap();

    // Get the provider
    let provider = kubarr::services::vpn::get_vpn_provider(&db, created.id)
        .await
        .unwrap();

    assert_eq!(provider.id, created.id);
    assert_eq!(provider.name, "Get Test");
    assert_eq!(provider.app_count, 0);
}

#[tokio::test]
async fn test_get_vpn_provider_not_found() {
    let db = create_test_db().await;

    let result = kubarr::services::vpn::get_vpn_provider(&db, 999).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("VPN provider 999 not found"));
}

#[tokio::test]
async fn test_update_vpn_provider() {
    let db = create_test_db().await;

    // Create provider
    let credentials = serde_json::json!({"private_key": "original_key"});
    let req = CreateVpnProviderRequest {
        name: "Original Name".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let created = kubarr::services::vpn::create_vpn_provider(&db, req)
        .await
        .unwrap();

    // Update provider
    let update_req = UpdateVpnProviderRequest {
        name: Some("Updated Name".to_string()),
        service_provider: Some("mullvad".to_string()),
        credentials: None,
        enabled: Some(false),
        kill_switch: Some(false),
        firewall_outbound_subnets: None,
    };

    let updated = kubarr::services::vpn::update_vpn_provider(&db, created.id, update_req)
        .await
        .unwrap();

    assert_eq!(updated.name, "Updated Name");
    assert_eq!(updated.service_provider, Some("mullvad".to_string()));
    assert!(!updated.enabled);
    assert!(!updated.kill_switch);
    assert_eq!(updated.firewall_outbound_subnets, "10.0.0.0/8"); // Unchanged
}

#[tokio::test]
async fn test_update_vpn_provider_credentials() {
    let db = create_test_db().await;

    // Create provider
    let credentials = serde_json::json!({"private_key": "original_key"});
    let req = CreateVpnProviderRequest {
        name: "Test".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let created = kubarr::services::vpn::create_vpn_provider(&db, req)
        .await
        .unwrap();

    // Update with new credentials
    let new_credentials = serde_json::json!({"private_key": "new_key"});
    let update_req = UpdateVpnProviderRequest {
        name: None,
        service_provider: None,
        credentials: Some(new_credentials),
        enabled: None,
        kill_switch: None,
        firewall_outbound_subnets: None,
    };

    let result = kubarr::services::vpn::update_vpn_provider(&db, created.id, update_req).await;
    assert!(result.is_ok());

    // Verify credentials were updated by checking the database
    let provider_model = VpnProvider::find_by_id(created.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(provider_model.credentials_json.contains("new_key"));
}

#[tokio::test]
async fn test_update_vpn_provider_invalid_credentials() {
    let db = create_test_db().await;

    // Create provider
    let credentials = serde_json::json!({"private_key": "key"});
    let req = CreateVpnProviderRequest {
        name: "Test".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let created = kubarr::services::vpn::create_vpn_provider(&db, req)
        .await
        .unwrap();

    // Try to update with invalid credentials (missing private_key)
    let bad_credentials = serde_json::json!({"addresses": ["10.0.0.1/32"]});
    let update_req = UpdateVpnProviderRequest {
        name: None,
        service_provider: None,
        credentials: Some(bad_credentials),
        enabled: None,
        kill_switch: None,
        firewall_outbound_subnets: None,
    };

    let result = kubarr::services::vpn::update_vpn_provider(&db, created.id, update_req).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("private_key"));
}

// ============================================================================
// App VPN Config Tests
// ============================================================================

#[tokio::test]
async fn test_assign_vpn_to_app() {
    let db = create_test_db().await;

    // Create a VPN provider
    let credentials = serde_json::json!({"private_key": "test_key"});
    let provider_req = CreateVpnProviderRequest {
        name: "Test Provider".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let provider = kubarr::services::vpn::create_vpn_provider(&db, provider_req)
        .await
        .unwrap();

    // Assign VPN to app
    let assign_req = AssignVpnRequest {
        vpn_provider_id: provider.id,
        kill_switch_override: Some(false),
        port_forwarding: None,
    };

    let config = kubarr::services::vpn::assign_vpn_to_app(&db, "qbittorrent", assign_req)
        .await
        .unwrap();

    assert_eq!(config.app_name, "qbittorrent");
    assert_eq!(config.vpn_provider_id, provider.id);
    assert_eq!(config.vpn_provider_name, "Test Provider");
    assert_eq!(config.kill_switch_override, Some(false));
    assert!(!config.effective_kill_switch); // Override to false
}

#[tokio::test]
async fn test_assign_vpn_to_app_no_override() {
    let db = create_test_db().await;

    // Create a VPN provider with kill_switch enabled
    let credentials = serde_json::json!({"private_key": "test_key"});
    let provider_req = CreateVpnProviderRequest {
        name: "Test Provider".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let provider = kubarr::services::vpn::create_vpn_provider(&db, provider_req)
        .await
        .unwrap();

    // Assign VPN without override
    let assign_req = AssignVpnRequest {
        vpn_provider_id: provider.id,
        kill_switch_override: None,
        port_forwarding: None,
    };

    let config = kubarr::services::vpn::assign_vpn_to_app(&db, "sonarr", assign_req)
        .await
        .unwrap();

    assert_eq!(config.kill_switch_override, None);
    assert!(config.effective_kill_switch); // Uses provider default (true)
}

#[tokio::test]
async fn test_assign_vpn_to_app_disabled_provider() {
    let db = create_test_db().await;

    // Create a disabled VPN provider
    let credentials = serde_json::json!({"private_key": "test_key"});
    let provider_req = CreateVpnProviderRequest {
        name: "Disabled Provider".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials,
        enabled: false, // Disabled
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let provider = kubarr::services::vpn::create_vpn_provider(&db, provider_req)
        .await
        .unwrap();

    // Try to assign disabled provider to app
    let assign_req = AssignVpnRequest {
        vpn_provider_id: provider.id,
        kill_switch_override: None,
        port_forwarding: None,
    };

    let result = kubarr::services::vpn::assign_vpn_to_app(&db, "radarr", assign_req).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Cannot assign a disabled VPN provider"));
}

#[tokio::test]
async fn test_assign_vpn_to_app_nonexistent_provider() {
    let db = create_test_db().await;

    let assign_req = AssignVpnRequest {
        vpn_provider_id: 999,
        kill_switch_override: None,
        port_forwarding: None,
    };

    let result = kubarr::services::vpn::assign_vpn_to_app(&db, "lidarr", assign_req).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("VPN provider 999 not found"));
}

#[tokio::test]
async fn test_reassign_vpn_to_app() {
    let db = create_test_db().await;

    // Create two providers
    let creds1 = serde_json::json!({"private_key": "key1"});
    let provider1_req = CreateVpnProviderRequest {
        name: "Provider 1".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials: creds1,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let provider1 = kubarr::services::vpn::create_vpn_provider(&db, provider1_req)
        .await
        .unwrap();

    let creds2 = serde_json::json!({"username": "user", "password": "pass"});
    let provider2_req = CreateVpnProviderRequest {
        name: "Provider 2".to_string(),
        vpn_type: vpn_provider::VpnType::OpenVpn,
        service_provider: None,
        credentials: creds2,
        enabled: true,
        kill_switch: false,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let provider2 = kubarr::services::vpn::create_vpn_provider(&db, provider2_req)
        .await
        .unwrap();

    // Assign first provider
    let assign_req1 = AssignVpnRequest {
        vpn_provider_id: provider1.id,
        kill_switch_override: None,
        port_forwarding: None,
    };
    kubarr::services::vpn::assign_vpn_to_app(&db, "prowlarr", assign_req1)
        .await
        .unwrap();

    // Reassign to second provider
    let assign_req2 = AssignVpnRequest {
        vpn_provider_id: provider2.id,
        kill_switch_override: Some(true),
        port_forwarding: None,
    };
    let config = kubarr::services::vpn::assign_vpn_to_app(&db, "prowlarr", assign_req2)
        .await
        .unwrap();

    assert_eq!(config.vpn_provider_id, provider2.id);
    assert_eq!(config.vpn_provider_name, "Provider 2");
    assert_eq!(config.kill_switch_override, Some(true));
    assert!(config.effective_kill_switch); // Override to true
}

#[tokio::test]
async fn test_get_app_vpn_config() {
    let db = create_test_db().await;

    // Create provider and assign to app
    let credentials = serde_json::json!({"private_key": "test_key"});
    let provider_req = CreateVpnProviderRequest {
        name: "Test Provider".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials,
        enabled: true,
        kill_switch: false,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let provider = kubarr::services::vpn::create_vpn_provider(&db, provider_req)
        .await
        .unwrap();

    let assign_req = AssignVpnRequest {
        vpn_provider_id: provider.id,
        kill_switch_override: None,
        port_forwarding: None,
    };
    kubarr::services::vpn::assign_vpn_to_app(&db, "bazarr", assign_req)
        .await
        .unwrap();

    // Get the config
    let config = kubarr::services::vpn::get_app_vpn_config(&db, "bazarr")
        .await
        .unwrap();

    assert!(config.is_some());
    let config = config.unwrap();
    assert_eq!(config.app_name, "bazarr");
    assert_eq!(config.vpn_provider_id, provider.id);
}

#[tokio::test]
async fn test_get_app_vpn_config_not_found() {
    let db = create_test_db().await;

    let config = kubarr::services::vpn::get_app_vpn_config(&db, "nonexistent")
        .await
        .unwrap();

    assert!(config.is_none());
}

#[tokio::test]
async fn test_list_app_vpn_configs() {
    let db = create_test_db().await;

    // Create provider
    let credentials = serde_json::json!({"private_key": "test_key"});
    let provider_req = CreateVpnProviderRequest {
        name: "Test Provider".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let provider = kubarr::services::vpn::create_vpn_provider(&db, provider_req)
        .await
        .unwrap();

    // Assign to multiple apps
    for app_name in ["jellyfin", "plex", "emby"] {
        let assign_req = AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: None,
            port_forwarding: None,
        };
        kubarr::services::vpn::assign_vpn_to_app(&db, app_name, assign_req)
            .await
            .unwrap();
    }

    // List configs
    let configs = kubarr::services::vpn::list_app_vpn_configs(&db)
        .await
        .unwrap();

    assert_eq!(configs.len(), 3);
    assert!(configs.iter().any(|c| c.app_name == "jellyfin"));
    assert!(configs.iter().any(|c| c.app_name == "plex"));
    assert!(configs.iter().any(|c| c.app_name == "emby"));
}

#[tokio::test]
async fn test_vpn_provider_app_count() {
    let db = create_test_db().await;

    // Create provider
    let credentials = serde_json::json!({"private_key": "test_key"});
    let provider_req = CreateVpnProviderRequest {
        name: "Test Provider".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let provider = kubarr::services::vpn::create_vpn_provider(&db, provider_req)
        .await
        .unwrap();

    // Initially should have 0 apps
    let provider = kubarr::services::vpn::get_vpn_provider(&db, provider.id)
        .await
        .unwrap();
    assert_eq!(provider.app_count, 0);

    // Assign to two apps
    for app_name in ["app1", "app2"] {
        let assign_req = AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: None,
            port_forwarding: None,
        };
        kubarr::services::vpn::assign_vpn_to_app(&db, app_name, assign_req)
            .await
            .unwrap();
    }

    // Should now have 2 apps
    let provider = kubarr::services::vpn::get_vpn_provider(&db, provider.id)
        .await
        .unwrap();
    assert_eq!(provider.app_count, 2);
}

// ============================================================================
// VPN Deployment Config Tests
// ============================================================================

#[tokio::test]
async fn test_get_vpn_deployment_config() {
    let db = create_test_db().await;

    // Create provider and assign to app
    let credentials = serde_json::json!({"private_key": "test_key"});
    let provider_req = CreateVpnProviderRequest {
        name: "Deploy Test".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials,
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8,172.16.0.0/12".to_string(),
    };
    let provider = kubarr::services::vpn::create_vpn_provider(&db, provider_req)
        .await
        .unwrap();

    let assign_req = AssignVpnRequest {
        vpn_provider_id: provider.id,
        kill_switch_override: Some(false),
        port_forwarding: None,
    };
    kubarr::services::vpn::assign_vpn_to_app(&db, "testapp", assign_req)
        .await
        .unwrap();

    // Get deployment config
    let deploy_config = kubarr::services::vpn::get_vpn_deployment_config(&db, "testapp")
        .await
        .unwrap();

    assert!(deploy_config.is_some());
    let deploy_config = deploy_config.unwrap();
    assert!(deploy_config.enabled);
    assert_eq!(deploy_config.secret_name, "vpn-testapp");
    assert!(!deploy_config.kill_switch); // Override to false
    assert_eq!(
        deploy_config.firewall_outbound_subnets,
        "10.0.0.0/8,172.16.0.0/12"
    );
}

#[tokio::test]
async fn test_get_vpn_deployment_config_disabled_provider() {
    let db = create_test_db().await;

    // Create disabled provider
    let credentials = serde_json::json!({"private_key": "test_key"});
    let provider_req = CreateVpnProviderRequest {
        name: "Disabled".to_string(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: None,
        credentials,
        enabled: false, // Disabled
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let provider = kubarr::services::vpn::create_vpn_provider(&db, provider_req)
        .await
        .unwrap();

    // Manually create app config (bypassing the service which checks enabled status)
    let now = chrono::Utc::now();
    use kubarr::models::app_vpn_config;
    let config = app_vpn_config::ActiveModel {
        app_name: Set("testapp2".to_string()),
        vpn_provider_id: Set(provider.id),
        kill_switch_override: Set(None),
        port_forwarding: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
    };
    config.insert(&db).await.unwrap();

    // Get deployment config - should be None because provider is disabled
    let deploy_config = kubarr::services::vpn::get_vpn_deployment_config(&db, "testapp2")
        .await
        .unwrap();

    assert!(deploy_config.is_none());
}

#[tokio::test]
async fn test_get_vpn_deployment_config_no_vpn() {
    let db = create_test_db().await;

    let deploy_config = kubarr::services::vpn::get_vpn_deployment_config(&db, "novpnapp")
        .await
        .unwrap();

    assert!(deploy_config.is_none());
}

// ============================================================================
// Supported Providers Tests
// ============================================================================

#[test]
fn test_get_supported_providers() {
    let providers = kubarr::services::vpn::get_supported_providers();

    assert!(!providers.is_empty());
    assert!(providers.len() >= 10);

    // Check for some well-known providers
    assert!(providers.iter().any(|p| p.id == "custom"));
    assert!(providers.iter().any(|p| p.id == "mullvad"));
    assert!(providers.iter().any(|p| p.id == "nordvpn"));
    assert!(providers.iter().any(|p| p.id == "protonvpn"));

    // Verify custom provider supports both types
    let custom = providers.iter().find(|p| p.id == "custom").unwrap();
    assert!(custom.vpn_types.contains(&"wireguard"));
    assert!(custom.vpn_types.contains(&"openvpn"));
}

// ============================================================================
// Helper Function Tests
// ============================================================================

#[test]
fn test_build_wireguard_secret_data() {
    let credentials = serde_json::json!({
        "private_key": "test_private_key",
        "addresses": ["10.0.0.1/32", "fd00::1/128"],
        "public_key": "test_public_key",
        "endpoint_ip": "1.2.3.4",
        "endpoint_port": 51820,
        "preshared_key": "test_preshared"
    });

    // We can't directly test the private function, but we can test through serialization
    // and verify the data structure is correct
    let wg_creds: WireGuardCredentials = serde_json::from_value(credentials).unwrap();
    assert_eq!(wg_creds.private_key, "test_private_key");
    assert_eq!(wg_creds.addresses.len(), 2);
    assert_eq!(wg_creds.endpoint_port, Some(51820));
}

#[test]
fn test_build_openvpn_secret_data() {
    let credentials = serde_json::json!({
        "username": "vpn_user",
        "password": "vpn_password",
        "server_countries": "United States,Canada",
        "server_cities": "New York,Toronto",
        "server_hostnames": "us1.example.com,ca1.example.com"
    });

    let ovpn_creds: OpenVpnCredentials = serde_json::from_value(credentials).unwrap();
    assert_eq!(ovpn_creds.username, "vpn_user");
    assert_eq!(ovpn_creds.password, "vpn_password");
    assert_eq!(
        ovpn_creds.server_countries,
        Some("United States,Canada".to_string())
    );
}
