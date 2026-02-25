//! Direct serde coverage tests for request/response structs in endpoint modules.
//!
//! These tests ensure that all Serialize/Deserialize implementations are exercised
//! for structs defined in endpoint files. This is important because tarpaulin
//! counts derive-generated code as production code lines.

use kubarr::endpoints::users::{
    AdminResetPasswordRequest, ChangeOwnPasswordRequest, CreateInviteRequest, CreateUserRequest,
    DeleteOwnAccountRequest, Disable2FARequest, Enable2FARequest, ListParams, RoleInfo,
    TwoFactorEnableResponse, TwoFactorRecoveryCodesResponse, TwoFactorSetupResponse,
    TwoFactorStatusResponse, UpdateOwnProfileRequest, UpdatePreferences, UpdateUserRequest,
};

use kubarr::endpoints::roles::{
    CreateRoleRequest, PermissionInfo, RoleWithAppsResponse, SetRoleApps, SetRolePermissions,
    UpdateRoleRequest,
};

use kubarr::endpoints::setup::{
    BootstrapStartResponse, GeneratedCredentialsResponse, ServerConfigRequest,
    ServerConfigResponse, SetupRequest, SetupRequiredResponse, SetupStatusResponse,
};

use kubarr::endpoints::apps::NamespaceQuery;

use kubarr::endpoints::audit::{ClearLogsRequest, ClearLogsResponse};

use kubarr::endpoints::settings::{SettingResponse, SettingUpdate, SettingsResponse};

// ============================================================================
// users.rs structs
// ============================================================================

#[test]
fn list_params_deser_full() {
    let json = r#"{"skip":10,"limit":25}"#;
    let p: ListParams = serde_json::from_str(json).expect("deserialize");
    assert_eq!(p.skip, Some(10));
    assert_eq!(p.limit, Some(25));
}

#[test]
fn list_params_deser_empty() {
    let p: ListParams = serde_json::from_str("{}").expect("deserialize");
    assert_eq!(p.skip, None);
    assert_eq!(p.limit, None);
}

#[test]
fn create_user_request_deser() {
    let json =
        r#"{"username":"alice","email":"alice@example.com","password":"secret","role_ids":[1,2]}"#;
    let r: CreateUserRequest = serde_json::from_str(json).expect("deserialize");
    assert_eq!(r.username, "alice");
    assert_eq!(r.role_ids, vec![1, 2]);
}

#[test]
fn create_user_request_default_role_ids() {
    let json = r#"{"username":"alice","email":"alice@example.com","password":"s"}"#;
    let r: CreateUserRequest = serde_json::from_str(json).expect("deserialize");
    assert_eq!(r.role_ids, Vec::<i64>::new());
}

#[test]
fn update_user_request_deser_full() {
    let json = r#"{"email":"new@example.com","is_active":true,"is_approved":false,"role_ids":[3]}"#;
    let r: UpdateUserRequest = serde_json::from_str(json).expect("deserialize");
    assert_eq!(r.email.as_deref(), Some("new@example.com"));
    assert_eq!(r.is_active, Some(true));
}

#[test]
fn update_user_request_deser_empty() {
    let r: UpdateUserRequest = serde_json::from_str("{}").expect("deserialize");
    assert!(r.email.is_none());
    assert!(r.is_active.is_none());
}

#[test]
fn role_info_ser() {
    let r = RoleInfo {
        id: 1,
        name: "admin".to_string(),
        description: Some("Admin role".to_string()),
    };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"name\":\"admin\""));
}

#[test]
fn update_preferences_deser() {
    let json = r#"{"theme":"dark"}"#;
    let p: UpdatePreferences = serde_json::from_str(json).expect("deser");
    assert_eq!(p.theme.as_deref(), Some("dark"));
}

#[test]
fn change_own_password_request_deser() {
    let json = r#"{"current_password":"old","new_password":"new"}"#;
    let r: ChangeOwnPasswordRequest = serde_json::from_str(json).expect("deser");
    assert_eq!(r.current_password, "old");
}

#[test]
fn update_own_profile_request_deser() {
    let r: UpdateOwnProfileRequest =
        serde_json::from_str(r#"{"username":"bob","email":"bob@example.com"}"#).expect("deser");
    assert_eq!(r.username.as_deref(), Some("bob"));
}

#[test]
fn admin_reset_password_deser() {
    let r: AdminResetPasswordRequest =
        serde_json::from_str(r#"{"new_password":"abc123"}"#).expect("deser");
    assert_eq!(r.new_password, "abc123");
}

#[test]
fn two_factor_setup_response_ser() {
    let r = TwoFactorSetupResponse {
        secret: "ABCDEF".to_string(),
        provisioning_uri: "otpauth://totp/test".to_string(),
    };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"secret\":\"ABCDEF\""));
}

#[test]
fn enable_2fa_request_deser() {
    let r: Enable2FARequest = serde_json::from_str(r#"{"code":"123456"}"#).expect("deser");
    assert_eq!(r.code, "123456");
}

#[test]
fn disable_2fa_request_deser() {
    let r: Disable2FARequest = serde_json::from_str(r#"{"password":"mypassword"}"#).expect("deser");
    assert_eq!(r.password, "mypassword");
}

#[test]
fn delete_own_account_request_deser() {
    let r: DeleteOwnAccountRequest =
        serde_json::from_str(r#"{"password":"mypassword"}"#).expect("deser");
    assert_eq!(r.password, "mypassword");
}

#[test]
fn two_factor_status_response_ser() {
    let r = TwoFactorStatusResponse {
        enabled: true,
        verified_at: None,
        required_by_role: false,
    };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"enabled\":true"));
}

#[test]
fn two_factor_enable_response_ser() {
    let r = TwoFactorEnableResponse {
        message: "2FA enabled".to_string(),
        recovery_codes: vec!["CODE1".to_string(), "CODE2".to_string()],
    };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("CODE1"));
    assert!(json.contains("\"message\":\"2FA enabled\""));
}

#[test]
fn two_factor_recovery_codes_response_ser() {
    let r = TwoFactorRecoveryCodesResponse { remaining: 6 };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"remaining\":6"));
}

#[test]
fn create_invite_request_default_days() {
    let r: CreateInviteRequest = serde_json::from_str("{}").expect("deser");
    assert_eq!(r.expires_in_days, 7);
}

#[test]
fn create_invite_request_custom_days() {
    let r: CreateInviteRequest = serde_json::from_str(r#"{"expires_in_days":30}"#).expect("deser");
    assert_eq!(r.expires_in_days, 30);
}

// ============================================================================
// roles.rs structs
// ============================================================================

#[test]
fn create_role_request_deser() {
    let json = r#"{"name":"moderator","description":"Moderator role","app_names":["sonarr"],"requires_2fa":true}"#;
    let r: CreateRoleRequest = serde_json::from_str(json).expect("deser");
    assert_eq!(r.name, "moderator");
    assert!(r.requires_2fa);
}

#[test]
fn create_role_request_defaults() {
    let r: CreateRoleRequest = serde_json::from_str(r#"{"name":"viewer"}"#).expect("deser");
    assert_eq!(r.app_names, Vec::<String>::new());
    assert!(!r.requires_2fa);
}

#[test]
fn update_role_request_deser() {
    let r: UpdateRoleRequest = serde_json::from_str("{}").expect("deser");
    assert!(r.name.is_none());
    assert!(r.requires_2fa.is_none());
}

#[test]
fn update_role_request_deser_full() {
    let r: UpdateRoleRequest =
        serde_json::from_str(r#"{"name":"newname","requires_2fa":true}"#).expect("deser");
    assert_eq!(r.name.as_deref(), Some("newname"));
    assert_eq!(r.requires_2fa, Some(true));
}

#[test]
fn set_role_apps_deser() {
    let r: SetRoleApps =
        serde_json::from_str(r#"{"app_names":["sonarr","radarr"]}"#).expect("deser");
    assert_eq!(r.app_names, vec!["sonarr", "radarr"]);
}

#[test]
fn set_role_permissions_deser() {
    let r: SetRolePermissions =
        serde_json::from_str(r#"{"permissions":["users.view","apps.view"]}"#).expect("deser");
    assert_eq!(r.permissions, vec!["users.view", "apps.view"]);
}

#[test]
fn permission_info_ser() {
    let r = PermissionInfo {
        key: "users.view".to_string(),
        category: "users".to_string(),
        description: "View users".to_string(),
    };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"key\":\"users.view\""));
}

#[test]
fn role_with_apps_response_ser() {
    use chrono::Utc;
    let r = RoleWithAppsResponse {
        id: 1,
        name: "admin".to_string(),
        description: Some("Admin".to_string()),
        is_system: true,
        requires_2fa: false,
        created_at: Utc::now(),
        app_names: vec!["sonarr".to_string()],
        permissions: vec!["users.view".to_string()],
    };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"name\":\"admin\""));
    assert!(json.contains("sonarr"));
}

// ============================================================================
// setup.rs structs
// ============================================================================

#[test]
fn setup_required_response_ser() {
    let r = SetupRequiredResponse {
        setup_required: true,
        database_pending: false,
    };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"setup_required\":true"));
}

#[test]
fn setup_status_response_ser() {
    let r = SetupStatusResponse {
        setup_required: false,
        bootstrap_complete: true,
        server_configured: true,
        admin_user_exists: true,
        storage_configured: false,
    };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"bootstrap_complete\":true"));
}

#[test]
fn setup_request_deser() {
    let json =
        r#"{"admin_username":"admin","admin_email":"admin@example.com","admin_password":"pass"}"#;
    let r: SetupRequest = serde_json::from_str(json).expect("deser");
    assert_eq!(r.admin_username, "admin");
}

#[test]
fn generated_credentials_response_ser() {
    let r = GeneratedCredentialsResponse {
        admin_username: "admin".to_string(),
        admin_email: "admin@example.com".to_string(),
        admin_password: "random_pass".to_string(),
    };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"admin_username\":\"admin\""));
}

#[test]
fn bootstrap_start_response_ser() {
    let r = BootstrapStartResponse {
        message: "Bootstrap started".to_string(),
        started: true,
    };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"started\":true"));
}

#[test]
fn server_config_request_deser() {
    let r: ServerConfigRequest =
        serde_json::from_str(r#"{"name":"myserver","storage_path":"/data"}"#).expect("deser");
    assert_eq!(r.name, "myserver");
    assert_eq!(r.storage_path, "/data");
}

#[test]
fn server_config_response_ser() {
    let r = ServerConfigResponse {
        name: "myserver".to_string(),
        storage_path: "/data".to_string(),
        nfs_server: None,
    };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"name\":\"myserver\""));
}

// ============================================================================
// apps.rs structs
// ============================================================================

#[test]
fn namespace_query_deser_some() {
    let q: NamespaceQuery = serde_json::from_str(r#"{"namespace":"media"}"#).expect("deser");
    assert_eq!(q.namespace.as_deref(), Some("media"));
}

#[test]
fn namespace_query_deser_none() {
    let q: NamespaceQuery = serde_json::from_str("{}").expect("deser");
    assert!(q.namespace.is_none());
}

// ============================================================================
// audit.rs structs
// ============================================================================

#[test]
fn clear_logs_request_deser_with_days() {
    let r: ClearLogsRequest = serde_json::from_str(r#"{"days":30}"#).expect("deser");
    assert_eq!(r.days, Some(30));
}

#[test]
fn clear_logs_request_deser_empty() {
    let r: ClearLogsRequest = serde_json::from_str("{}").expect("deser");
    assert_eq!(r.days, None);
}

#[test]
fn clear_logs_response_ser() {
    let r = ClearLogsResponse {
        deleted: 42,
        message: "42 logs deleted".to_string(),
    };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"deleted\":42"));
}

// ============================================================================
// settings.rs structs
// ============================================================================

#[test]
fn setting_response_ser() {
    let r = SettingResponse {
        key: "test_key".to_string(),
        value: "test_value".to_string(),
        description: Some("A test setting".to_string()),
    };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"key\":\"test_key\""));
    assert!(json.contains("\"value\":\"test_value\""));
}

#[test]
fn setting_update_deser() {
    let u: SettingUpdate = serde_json::from_str(r#"{"value":"new_value"}"#).expect("deser");
    assert_eq!(u.value, "new_value");
}

#[test]
fn settings_response_ser() {
    use std::collections::HashMap;
    let mut map = HashMap::new();
    map.insert(
        "my_key".to_string(),
        SettingResponse {
            key: "my_key".to_string(),
            value: "my_value".to_string(),
            description: None,
        },
    );
    let r = SettingsResponse { settings: map };
    let json = serde_json::to_string(&r).expect("ser");
    assert!(json.contains("\"settings\""));
    assert!(json.contains("my_key"));
}
