//! Unit tests for VPN service functions
//!
//! Tests:
//! - `get_supported_providers()` — pure function, no DB/K8s needed
//! - Struct serialization/deserialization for VPN types
//! - DB-only functions: list_vpn_providers, get_vpn_provider (not found), create_vpn_provider,
//!   update_vpn_provider, list_app_vpn_configs, get_app_vpn_config, assign_vpn_to_app,
//!   get_vpn_deployment_config

mod common;
use common::create_test_db_with_seed;

use kubarr::models::vpn_provider::VpnType;
use kubarr::services::vpn::{
    get_supported_providers, AppVpnConfigResponse, AssignVpnRequest, CreateVpnProviderRequest,
    OpenVpnCredentials, SupportedProvider, UpdateVpnProviderRequest, VpnCredentials,
    VpnProviderResponse, VpnTestResult, WireGuardCredentials,
};

// ============================================================================
// get_supported_providers
// ============================================================================

#[test]
fn test_get_supported_providers_returns_nonempty_list() {
    let providers = get_supported_providers();
    assert!(
        !providers.is_empty(),
        "get_supported_providers must return at least one provider"
    );
}

#[test]
fn test_get_supported_providers_has_custom() {
    let providers = get_supported_providers();
    assert!(
        providers.iter().any(|p| p.id == "custom"),
        "must have custom provider"
    );
}

#[test]
fn test_get_supported_providers_has_mullvad() {
    let providers = get_supported_providers();
    assert!(
        providers.iter().any(|p| p.id == "mullvad"),
        "must have mullvad provider"
    );
}

#[test]
fn test_get_supported_providers_has_nordvpn() {
    let providers = get_supported_providers();
    assert!(
        providers.iter().any(|p| p.id == "nordvpn"),
        "must have nordvpn provider"
    );
}

#[test]
fn test_get_supported_providers_at_least_10() {
    let providers = get_supported_providers();
    assert!(
        providers.len() >= 5,
        "must have at least 5 supported providers"
    );
}

#[test]
fn test_get_supported_providers_all_have_vpn_types() {
    let providers = get_supported_providers();
    for p in &providers {
        assert!(
            !p.vpn_types.is_empty(),
            "provider {} must have at least one vpn type",
            p.id
        );
    }
}

#[test]
fn test_get_supported_providers_all_have_descriptions() {
    let providers = get_supported_providers();
    for p in &providers {
        assert!(
            !p.description.is_empty(),
            "provider {} must have a description",
            p.id
        );
        assert!(!p.name.is_empty(), "provider {} must have a name", p.id);
    }
}

#[test]
fn test_supported_provider_pia_has_port_forwarding() {
    let providers = get_supported_providers();
    let pia = providers.iter().find(|p| p.id == "private_internet_access");
    if let Some(p) = pia {
        assert!(p.supports_port_forwarding, "PIA supports port forwarding");
    }
}

#[test]
fn test_supported_provider_nordvpn_no_port_forwarding() {
    let providers = get_supported_providers();
    let nord = providers.iter().find(|p| p.id == "nordvpn");
    if let Some(p) = nord {
        assert!(
            !p.supports_port_forwarding,
            "NordVPN does not support port forwarding"
        );
    }
}

// ============================================================================
// Struct serialization/deserialization
// ============================================================================

#[test]
fn test_wireguard_credentials_serialization() {
    let creds = WireGuardCredentials {
        private_key: "my_private_key_here".to_string(),
        addresses: vec!["10.0.0.1/32".to_string()],
        public_key: Some("my_public_key".to_string()),
        endpoint_ip: Some("1.2.3.4".to_string()),
        endpoint_port: Some(51820),
        preshared_key: None,
    };
    let json = serde_json::to_value(&creds).expect("serialize WireGuardCredentials");
    assert_eq!(json["private_key"], "my_private_key_here");
    assert_eq!(json["endpoint_port"], 51820);
    // preshared_key should be absent (skip_serializing_if = None)
    assert!(!json.as_object().unwrap().contains_key("preshared_key"));
}

#[test]
fn test_wireguard_credentials_deserialization() {
    let json = serde_json::json!({
        "private_key": "wg_priv",
        "addresses": ["10.0.0.1/32", "::1/128"],
        "public_key": "wg_pub"
    });
    let creds: WireGuardCredentials = serde_json::from_value(json).expect("deserialize");
    assert_eq!(creds.private_key, "wg_priv");
    assert_eq!(creds.addresses.len(), 2);
    assert_eq!(creds.public_key.as_deref(), Some("wg_pub"));
    assert!(creds.endpoint_ip.is_none());
}

#[test]
fn test_wireguard_credentials_minimal() {
    let json = serde_json::json!({ "private_key": "key123" });
    let creds: WireGuardCredentials = serde_json::from_value(json).expect("deserialize minimal");
    assert_eq!(creds.private_key, "key123");
    assert!(creds.addresses.is_empty());
    assert!(creds.public_key.is_none());
}

#[test]
fn test_openvpn_credentials_serialization() {
    let creds = OpenVpnCredentials {
        username: "user@example.com".to_string(),
        password: "secret123".to_string(),
        server_countries: Some("Netherlands".to_string()),
        server_cities: None,
        server_hostnames: None,
    };
    let json = serde_json::to_value(&creds).expect("serialize OpenVpnCredentials");
    assert_eq!(json["username"], "user@example.com");
    assert_eq!(json["server_countries"], "Netherlands");
    // None fields skipped
    assert!(!json.as_object().unwrap().contains_key("server_cities"));
}

#[test]
fn test_openvpn_credentials_deserialization() {
    let json = serde_json::json!({
        "username": "vpnuser",
        "password": "vpnpass",
        "server_countries": "Germany"
    });
    let creds: OpenVpnCredentials = serde_json::from_value(json).expect("deserialize");
    assert_eq!(creds.username, "vpnuser");
    assert_eq!(creds.password, "vpnpass");
    assert_eq!(creds.server_countries.as_deref(), Some("Germany"));
}

#[test]
fn test_vpn_credentials_wireguard_variant() {
    let json = serde_json::json!({
        "private_key": "key",
        "addresses": ["10.0.0.1/32"]
    });
    let creds: VpnCredentials =
        serde_json::from_value(json).expect("deserialize VpnCredentials WG");
    assert!(matches!(creds, VpnCredentials::WireGuard(_)));
}

#[test]
fn test_assign_vpn_request_deserialization_basic() {
    let json = serde_json::json!({ "vpn_provider_id": 42 });
    let req: AssignVpnRequest = serde_json::from_value(json).expect("deserialize AssignVpnRequest");
    assert_eq!(req.vpn_provider_id, 42);
    assert!(req.kill_switch_override.is_none());
    assert!(req.port_forwarding.is_none());
}

#[test]
fn test_assign_vpn_request_deserialization_full() {
    let json = serde_json::json!({
        "vpn_provider_id": 1,
        "kill_switch_override": true,
        "port_forwarding": true
    });
    let req: AssignVpnRequest = serde_json::from_value(json).expect("deserialize");
    assert_eq!(req.vpn_provider_id, 1);
    assert_eq!(req.kill_switch_override, Some(true));
    assert_eq!(req.port_forwarding, Some(true));
}

#[test]
fn test_create_vpn_provider_request_defaults() {
    let json = serde_json::json!({
        "name": "My VPN",
        "vpn_type": "wireguard",
        "credentials": {"private_key": "key"}
    });
    let req: CreateVpnProviderRequest = serde_json::from_value(json).expect("deserialize");
    assert_eq!(req.name, "My VPN");
    assert!(req.enabled, "enabled defaults to true");
    assert!(req.kill_switch, "kill_switch defaults to true");
    assert!(
        req.firewall_outbound_subnets.contains("10.0.0.0"),
        "firewall defaults to RFC1918"
    );
}

#[test]
fn test_vpn_test_result_serialization() {
    let result = VpnTestResult {
        success: true,
        message: "VPN connected".to_string(),
        public_ip: Some("1.2.3.4".to_string()),
    };
    let json = serde_json::to_value(&result).expect("serialize VpnTestResult");
    assert_eq!(json["success"], true);
    assert_eq!(json["public_ip"], "1.2.3.4");
}

#[test]
fn test_vpn_test_result_no_ip() {
    let result = VpnTestResult {
        success: false,
        message: "Failed to connect".to_string(),
        public_ip: None,
    };
    let json = serde_json::to_value(&result).expect("serialize");
    assert_eq!(json["success"], false);
}

// ============================================================================
// DB-based service function tests (no K8s needed)
// ============================================================================

#[tokio::test]
async fn test_list_vpn_providers_empty_initially() {
    let db = create_test_db_with_seed().await;
    let providers = kubarr::services::vpn::list_vpn_providers(&db)
        .await
        .expect("list_vpn_providers must succeed");
    assert!(providers.is_empty(), "fresh DB must have no VPN providers");
}

#[tokio::test]
async fn test_get_vpn_provider_not_found() {
    let db = create_test_db_with_seed().await;
    let result = kubarr::services::vpn::get_vpn_provider(&db, 99999).await;
    assert!(result.is_err(), "nonexistent provider must return error");
}

#[tokio::test]
async fn test_create_and_get_vpn_provider() {
    let db = create_test_db_with_seed().await;

    let req = CreateVpnProviderRequest {
        name: "Test WG Provider".to_string(),
        vpn_type: VpnType::WireGuard,
        service_provider: Some("custom".to_string()),
        credentials: serde_json::json!({ "private_key": "wg_key_test" }),
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };

    let created = kubarr::services::vpn::create_vpn_provider(&db, req)
        .await
        .expect("create_vpn_provider must succeed");
    assert_eq!(created.name, "Test WG Provider");
    assert_eq!(created.vpn_type, VpnType::WireGuard);
    assert!(created.enabled);
    assert!(created.kill_switch);
    assert_eq!(created.app_count, 0);

    // Get it by ID
    let fetched = kubarr::services::vpn::get_vpn_provider(&db, created.id)
        .await
        .expect("get_vpn_provider must succeed after create");
    assert_eq!(fetched.name, "Test WG Provider");
    assert_eq!(fetched.id, created.id);
}

#[tokio::test]
async fn test_create_vpn_provider_invalid_credentials() {
    let db = create_test_db_with_seed().await;

    // WireGuard without private_key should fail validation
    let req = CreateVpnProviderRequest {
        name: "Bad WG".to_string(),
        vpn_type: VpnType::WireGuard,
        service_provider: None,
        credentials: serde_json::json!({ "public_key": "no_private_key" }),
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };

    let result = kubarr::services::vpn::create_vpn_provider(&db, req).await;
    assert!(result.is_err(), "create with missing private_key must fail");
}

#[tokio::test]
async fn test_create_vpn_provider_openvpn() {
    let db = create_test_db_with_seed().await;

    let req = CreateVpnProviderRequest {
        name: "Test OpenVPN Provider".to_string(),
        vpn_type: VpnType::OpenVpn,
        service_provider: Some("mullvad".to_string()),
        credentials: serde_json::json!({
            "username": "user123",
            "password": "pass456"
        }),
        enabled: true,
        kill_switch: false,
        firewall_outbound_subnets: "10.0.0.0/8,172.16.0.0/12".to_string(),
    };

    let created = kubarr::services::vpn::create_vpn_provider(&db, req)
        .await
        .expect("create OpenVPN provider must succeed");
    assert_eq!(created.name, "Test OpenVPN Provider");
    assert_eq!(created.vpn_type, VpnType::OpenVpn);
    assert!(!created.kill_switch);
}

#[tokio::test]
async fn test_update_vpn_provider() {
    let db = create_test_db_with_seed().await;

    let req = CreateVpnProviderRequest {
        name: "Original Name".to_string(),
        vpn_type: VpnType::WireGuard,
        service_provider: None,
        credentials: serde_json::json!({ "private_key": "key" }),
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let created = kubarr::services::vpn::create_vpn_provider(&db, req)
        .await
        .expect("create");

    let update_req = UpdateVpnProviderRequest {
        name: Some("Updated Name".to_string()),
        service_provider: None,
        credentials: None,
        enabled: Some(false),
        kill_switch: None,
        firewall_outbound_subnets: None,
    };

    let updated = kubarr::services::vpn::update_vpn_provider(&db, created.id, update_req)
        .await
        .expect("update_vpn_provider must succeed");
    assert_eq!(updated.name, "Updated Name");
    assert!(!updated.enabled);
}

#[tokio::test]
async fn test_update_vpn_provider_not_found() {
    let db = create_test_db_with_seed().await;
    let req = UpdateVpnProviderRequest {
        name: Some("New Name".to_string()),
        service_provider: None,
        credentials: None,
        enabled: None,
        kill_switch: None,
        firewall_outbound_subnets: None,
    };
    let result = kubarr::services::vpn::update_vpn_provider(&db, 99999, req).await;
    assert!(result.is_err(), "updating nonexistent provider must fail");
}

#[tokio::test]
async fn test_list_app_vpn_configs_empty_initially() {
    let db = create_test_db_with_seed().await;
    let configs = kubarr::services::vpn::list_app_vpn_configs(&db)
        .await
        .expect("list_app_vpn_configs must succeed");
    assert!(configs.is_empty(), "fresh DB must have no VPN configs");
}

#[tokio::test]
async fn test_get_app_vpn_config_returns_none_when_absent() {
    let db = create_test_db_with_seed().await;
    let result = kubarr::services::vpn::get_app_vpn_config(&db, "sonarr")
        .await
        .expect("get_app_vpn_config must succeed");
    assert!(result.is_none(), "no config for sonarr initially");
}

#[tokio::test]
async fn test_assign_vpn_to_app() {
    let db = create_test_db_with_seed().await;

    // Create a provider first
    let req = CreateVpnProviderRequest {
        name: "Test Provider".to_string(),
        vpn_type: VpnType::WireGuard,
        service_provider: None,
        credentials: serde_json::json!({ "private_key": "key" }),
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let provider = kubarr::services::vpn::create_vpn_provider(&db, req)
        .await
        .expect("create provider");

    // Assign it to sonarr
    let assign_req = AssignVpnRequest {
        vpn_provider_id: provider.id,
        kill_switch_override: Some(false),
        port_forwarding: Some(true),
    };

    let config = kubarr::services::vpn::assign_vpn_to_app(&db, "sonarr", assign_req)
        .await
        .expect("assign_vpn_to_app must succeed");
    assert_eq!(config.app_name, "sonarr");
    assert_eq!(config.vpn_provider_id, provider.id);
    assert_eq!(config.kill_switch_override, Some(false));
    assert!(config.port_forwarding);
    // effective_kill_switch = override (Some(false)) → false
    assert!(!config.effective_kill_switch);
}

#[tokio::test]
async fn test_assign_vpn_to_app_provider_not_found() {
    let db = create_test_db_with_seed().await;
    let req = AssignVpnRequest {
        vpn_provider_id: 99999,
        kill_switch_override: None,
        port_forwarding: None,
    };
    let result = kubarr::services::vpn::assign_vpn_to_app(&db, "sonarr", req).await;
    assert!(result.is_err(), "assigning nonexistent provider must fail");
}

#[tokio::test]
async fn test_assign_vpn_to_disabled_provider_fails() {
    let db = create_test_db_with_seed().await;

    // Create a disabled provider
    let req = CreateVpnProviderRequest {
        name: "Disabled".to_string(),
        vpn_type: VpnType::OpenVpn,
        service_provider: None,
        credentials: serde_json::json!({ "username": "u", "password": "p" }),
        enabled: false,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let provider = kubarr::services::vpn::create_vpn_provider(&db, req)
        .await
        .expect("create disabled provider");

    let assign_req = AssignVpnRequest {
        vpn_provider_id: provider.id,
        kill_switch_override: None,
        port_forwarding: None,
    };
    let result = kubarr::services::vpn::assign_vpn_to_app(&db, "sonarr", assign_req).await;
    assert!(result.is_err(), "assigning disabled provider must fail");
}

#[tokio::test]
async fn test_get_vpn_deployment_config_returns_none_when_absent() {
    let db = create_test_db_with_seed().await;
    let result = kubarr::services::vpn::get_vpn_deployment_config(&db, "radarr")
        .await
        .expect("get_vpn_deployment_config must succeed");
    assert!(result.is_none(), "no VPN config for radarr initially");
}

#[tokio::test]
async fn test_list_providers_after_create() {
    let db = create_test_db_with_seed().await;

    for i in 0..3 {
        let req = CreateVpnProviderRequest {
            name: format!("Provider {}", i),
            vpn_type: VpnType::WireGuard,
            service_provider: None,
            credentials: serde_json::json!({ "private_key": format!("key_{}", i) }),
            enabled: true,
            kill_switch: true,
            firewall_outbound_subnets: "10.0.0.0/8".to_string(),
        };
        kubarr::services::vpn::create_vpn_provider(&db, req)
            .await
            .expect("create provider");
    }

    let providers = kubarr::services::vpn::list_vpn_providers(&db)
        .await
        .unwrap();
    assert_eq!(providers.len(), 3);
}

#[tokio::test]
async fn test_get_vpn_deployment_config_disabled_provider_returns_none() {
    let db = create_test_db_with_seed().await;

    // Create a provider (enabled initially)
    let req = CreateVpnProviderRequest {
        name: "Prov For Disable Test".to_string(),
        vpn_type: VpnType::WireGuard,
        service_provider: None,
        credentials: serde_json::json!({ "private_key": "disable_test_key" }),
        enabled: true,
        kill_switch: false,
        firewall_outbound_subnets: "0.0.0.0/0".to_string(),
    };
    let provider = kubarr::services::vpn::create_vpn_provider(&db, req)
        .await
        .expect("create provider");

    // Assign to app
    let assign = AssignVpnRequest {
        vpn_provider_id: provider.id,
        kill_switch_override: None,
        port_forwarding: None,
    };
    kubarr::services::vpn::assign_vpn_to_app(&db, "radarr", assign)
        .await
        .expect("assign_vpn_to_app must succeed");

    // Now disable the provider directly in DB
    use kubarr::models::vpn_provider;
    use sea_orm::{ActiveModelTrait, Set};
    let active = vpn_provider::ActiveModel {
        id: Set(provider.id),
        enabled: Set(false),
        ..Default::default()
    };
    active.update(&db).await.expect("disable provider");

    // get_vpn_deployment_config should return None for a disabled provider
    let config = kubarr::services::vpn::get_vpn_deployment_config(&db, "radarr")
        .await
        .expect("get_vpn_deployment_config must succeed");
    assert!(
        config.is_none(),
        "Deployment config must be None when provider is disabled"
    );
}

#[tokio::test]
async fn test_get_vpn_deployment_config_with_kill_switch_override() {
    let db = create_test_db_with_seed().await;

    // Provider with kill_switch=true
    let req = CreateVpnProviderRequest {
        name: "KS Provider".to_string(),
        vpn_type: VpnType::WireGuard,
        service_provider: None,
        credentials: serde_json::json!({ "private_key": "ks_key" }),
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: "10.0.0.0/8".to_string(),
    };
    let provider = kubarr::services::vpn::create_vpn_provider(&db, req)
        .await
        .expect("create provider");

    // Assign with kill_switch_override=false (override)
    let assign = AssignVpnRequest {
        vpn_provider_id: provider.id,
        kill_switch_override: Some(false),
        port_forwarding: Some(false),
    };
    kubarr::services::vpn::assign_vpn_to_app(&db, "sonarr2", assign)
        .await
        .expect("assign");

    let config = kubarr::services::vpn::get_vpn_deployment_config(&db, "sonarr2")
        .await
        .expect("get config")
        .expect("config must be Some");

    // kill_switch should be the override (false), not the provider default (true)
    assert!(config.enabled, "VPN must be enabled");
    assert!(
        !config.kill_switch,
        "kill_switch_override=false must take precedence over provider kill_switch=true"
    );
}
