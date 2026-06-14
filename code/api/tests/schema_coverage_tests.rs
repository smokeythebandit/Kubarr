//! Coverage tests for `src/schemas/` — serialization, deserialization,
//! required-field validation, optional-field handling, and From impls.
//!
//! The existing `schema_tests.rs` covers `RoleResponse`, `RoleInfo::from`,
//! and `UserResponse::from_user_with_roles`.  This file covers the remaining
//! schema types:
//!
//! - `CreateUser` / `UpdateUser`
//! - `CreateRole` / `UpdateRole` / `SetRoleApps`
//! - `UpdateSetting`
//! - `CreateInvite` / `InviteResponse`
//! - `UserPreferencesResponse` / `UpdateUserPreferences`

use chrono::Utc;
use kubarr::schemas::{
    invite::{CreateInvite, InviteResponse},
    preferences::{UpdateUserPreferences, UserPreferencesResponse},
    role::{CreateRole, SetRoleApps, UpdateRole},
    settings::UpdateSetting,
    user::{CreateUser, UpdateUser},
};

// ============================================================================
// schemas/user.rs — CreateUser
// ============================================================================

#[test]
fn create_user_deserializes_all_fields() {
    let json = r#"{
        "username": "alice",
        "email": "alice@example.com",
        "password": "s3cret",
        "role_ids": [1, 2, 3]
    }"#;

    let cu: CreateUser = serde_json::from_str(json).expect("CreateUser must deserialize");
    assert_eq!(cu.username, "alice");
    assert_eq!(cu.email, "alice@example.com");
    assert_eq!(cu.password, "s3cret");
    assert_eq!(cu.role_ids, vec![1_i64, 2, 3]);
}

#[test]
fn create_user_role_ids_defaults_to_empty_vec() {
    // role_ids has #[serde(default)] so it is optional in JSON
    let json = r#"{"username": "bob", "email": "bob@test.com", "password": "pw"}"#;
    let cu: CreateUser =
        serde_json::from_str(json).expect("CreateUser without role_ids must deserialize");
    assert_eq!(cu.username, "bob");
    assert!(cu.role_ids.is_empty(), "role_ids must default to empty vec");
}

#[test]
fn create_user_missing_required_field_fails() {
    // Missing "password" — must fail to deserialize
    let json = r#"{"username": "carol", "email": "carol@test.com"}"#;
    let result: Result<CreateUser, _> = serde_json::from_str(json);
    assert!(
        result.is_err(),
        "CreateUser without password must fail to deserialize"
    );
}

#[test]
fn create_user_missing_username_fails() {
    let json = r#"{"email": "x@x.com", "password": "pw"}"#;
    let result: Result<CreateUser, _> = serde_json::from_str(json);
    assert!(result.is_err(), "CreateUser without username must fail");
}

#[test]
fn create_user_missing_email_fails() {
    let json = r#"{"username": "x", "password": "pw"}"#;
    let result: Result<CreateUser, _> = serde_json::from_str(json);
    assert!(result.is_err(), "CreateUser without email must fail");
}

#[test]
fn create_user_clone() {
    let json =
        r#"{"username": "dave", "email": "dave@test.com", "password": "pw", "role_ids": [5]}"#;
    let cu: CreateUser = serde_json::from_str(json).unwrap();
    let cloned = cu.clone();
    assert_eq!(cloned.username, "dave");
    assert_eq!(cloned.role_ids, vec![5_i64]);
}

// ============================================================================
// schemas/user.rs — UpdateUser
// ============================================================================

#[test]
fn update_user_all_fields_some() {
    let json = r#"{
        "email": "new@example.com",
        "is_active": false,
        "is_approved": true,
        "role_ids": [2, 3]
    }"#;

    let uu: UpdateUser = serde_json::from_str(json).expect("UpdateUser must deserialize");
    assert_eq!(uu.email, Some("new@example.com".to_string()));
    assert_eq!(uu.is_active, Some(false));
    assert_eq!(uu.is_approved, Some(true));
    assert_eq!(uu.role_ids, Some(vec![2_i64, 3]));
}

#[test]
fn update_user_all_fields_none() {
    let json = r#"{}"#;
    let uu: UpdateUser =
        serde_json::from_str(json).expect("UpdateUser with empty body must deserialize");
    assert!(uu.email.is_none());
    assert!(uu.is_active.is_none());
    assert!(uu.is_approved.is_none());
    assert!(uu.role_ids.is_none());
}

#[test]
fn update_user_partial_fields() {
    let json = r#"{"email": "partial@test.com"}"#;
    let uu: UpdateUser = serde_json::from_str(json).unwrap();
    assert_eq!(uu.email, Some("partial@test.com".to_string()));
    assert!(uu.is_active.is_none());
    assert!(uu.is_approved.is_none());
    assert!(uu.role_ids.is_none());
}

#[test]
fn update_user_role_ids_empty_vec() {
    let json = r#"{"role_ids": []}"#;
    let uu: UpdateUser = serde_json::from_str(json).unwrap();
    assert_eq!(uu.role_ids, Some(vec![]));
}

#[test]
fn update_user_clone() {
    let json = r#"{"email": "clone@test.com", "is_active": true}"#;
    let uu: UpdateUser = serde_json::from_str(json).unwrap();
    let cloned = uu.clone();
    assert_eq!(cloned.email, Some("clone@test.com".to_string()));
    assert_eq!(cloned.is_active, Some(true));
}

// ============================================================================
// schemas/role.rs — CreateRole
// ============================================================================

#[test]
fn create_role_with_all_fields() {
    let json = r#"{"name": "editor", "description": "Can edit content", "app_names": ["sonarr", "radarr"]}"#;
    let cr: CreateRole = serde_json::from_str(json).expect("CreateRole must deserialize");
    assert_eq!(cr.name, "editor");
    assert_eq!(cr.description, Some("Can edit content".to_string()));
    assert_eq!(cr.app_names, vec!["sonarr", "radarr"]);
}

#[test]
fn create_role_missing_description_is_none() {
    let json = r#"{"name": "viewer"}"#;
    let cr: CreateRole =
        serde_json::from_str(json).expect("CreateRole without description must deserialize");
    assert_eq!(cr.name, "viewer");
    assert!(cr.description.is_none());
    assert!(cr.app_names.is_empty(), "app_names defaults to empty vec");
}

#[test]
fn create_role_app_names_defaults_to_empty() {
    // app_names has #[serde(default)]
    let json = r#"{"name": "tester", "description": null}"#;
    let cr: CreateRole = serde_json::from_str(json).unwrap();
    assert!(cr.app_names.is_empty());
}

#[test]
fn create_role_missing_name_fails() {
    let json = r#"{"description": "no name"}"#;
    let result: Result<CreateRole, _> = serde_json::from_str(json);
    assert!(result.is_err(), "CreateRole without name must fail");
}

#[test]
fn create_role_clone() {
    let json = r#"{"name": "cloner", "app_names": ["jellyfin"]}"#;
    let cr: CreateRole = serde_json::from_str(json).unwrap();
    let cloned = cr.clone();
    assert_eq!(cloned.name, "cloner");
    assert_eq!(cloned.app_names, vec!["jellyfin"]);
}

// ============================================================================
// schemas/role.rs — UpdateRole
// ============================================================================

#[test]
fn update_role_both_fields_some() {
    let json = r#"{"name": "new_name", "description": "updated desc"}"#;
    let ur: UpdateRole = serde_json::from_str(json).unwrap();
    assert_eq!(ur.name, Some("new_name".to_string()));
    assert_eq!(ur.description, Some("updated desc".to_string()));
}

#[test]
fn update_role_both_fields_none() {
    let json = r#"{}"#;
    let ur: UpdateRole = serde_json::from_str(json).unwrap();
    assert!(ur.name.is_none());
    assert!(ur.description.is_none());
}

#[test]
fn update_role_only_name() {
    let json = r#"{"name": "renamed"}"#;
    let ur: UpdateRole = serde_json::from_str(json).unwrap();
    assert_eq!(ur.name, Some("renamed".to_string()));
    assert!(ur.description.is_none());
}

#[test]
fn update_role_only_description() {
    let json = r#"{"description": "just a desc"}"#;
    let ur: UpdateRole = serde_json::from_str(json).unwrap();
    assert!(ur.name.is_none());
    assert_eq!(ur.description, Some("just a desc".to_string()));
}

#[test]
fn update_role_description_explicit_null() {
    // Explicit null → None
    let json = r#"{"description": null}"#;
    let ur: UpdateRole = serde_json::from_str(json).unwrap();
    assert!(ur.description.is_none());
}

#[test]
fn update_role_clone() {
    let json = r#"{"name": "cloned_role"}"#;
    let ur: UpdateRole = serde_json::from_str(json).unwrap();
    let cloned = ur.clone();
    assert_eq!(cloned.name, Some("cloned_role".to_string()));
}

// ============================================================================
// schemas/role.rs — SetRoleApps
// ============================================================================

#[test]
fn set_role_apps_with_apps() {
    let json = r#"{"app_names": ["sonarr", "radarr", "jellyfin"]}"#;
    let sra: SetRoleApps = serde_json::from_str(json).unwrap();
    assert_eq!(sra.app_names.len(), 3);
    assert!(sra.app_names.contains(&"sonarr".to_string()));
    assert!(sra.app_names.contains(&"jellyfin".to_string()));
}

#[test]
fn set_role_apps_empty_list() {
    let json = r#"{"app_names": []}"#;
    let sra: SetRoleApps = serde_json::from_str(json).unwrap();
    assert!(sra.app_names.is_empty());
}

#[test]
fn set_role_apps_missing_field_fails() {
    let json = r#"{}"#;
    let result: Result<SetRoleApps, _> = serde_json::from_str(json);
    assert!(result.is_err(), "SetRoleApps requires app_names field");
}

#[test]
fn set_role_apps_clone() {
    let json = r#"{"app_names": ["app_a", "app_b"]}"#;
    let sra: SetRoleApps = serde_json::from_str(json).unwrap();
    let cloned = sra.clone();
    assert_eq!(cloned.app_names, vec!["app_a", "app_b"]);
}

// ============================================================================
// schemas/settings.rs — UpdateSetting
// ============================================================================

#[test]
fn update_setting_deserializes() {
    let json = r#"{"value": "true"}"#;
    let us: UpdateSetting = serde_json::from_str(json).expect("UpdateSetting must deserialize");
    assert_eq!(us.value, "true");
}

#[test]
fn update_setting_empty_string_value() {
    let json = r#"{"value": ""}"#;
    let us: UpdateSetting = serde_json::from_str(json).unwrap();
    assert_eq!(us.value, "");
}

#[test]
fn update_setting_missing_value_fails() {
    let json = r#"{}"#;
    let result: Result<UpdateSetting, _> = serde_json::from_str(json);
    assert!(result.is_err(), "UpdateSetting without value must fail");
}

#[test]
fn update_setting_clone() {
    let json = r#"{"value": "42"}"#;
    let us: UpdateSetting = serde_json::from_str(json).unwrap();
    let cloned = us.clone();
    assert_eq!(cloned.value, "42");
}

#[test]
fn update_setting_numeric_string_value() {
    let json = r#"{"value": "100"}"#;
    let us: UpdateSetting = serde_json::from_str(json).unwrap();
    assert_eq!(us.value, "100");
}

// ============================================================================
// schemas/invite.rs — CreateInvite
// ============================================================================

#[test]
fn create_invite_with_expires_in_days() {
    let json = r#"{"expires_in_days": 14}"#;
    let ci: CreateInvite = serde_json::from_str(json).expect("CreateInvite must deserialize");
    assert_eq!(ci.expires_in_days, 14);
}

#[test]
fn create_invite_defaults_to_7_days() {
    // expires_in_days has #[serde(default = "default_invite_days")] → 7
    let json = r#"{}"#;
    let ci: CreateInvite =
        serde_json::from_str(json).expect("CreateInvite without days must deserialize");
    assert_eq!(ci.expires_in_days, 7, "default expires_in_days must be 7");
}

#[test]
fn create_invite_zero_days() {
    let json = r#"{"expires_in_days": 0}"#;
    let ci: CreateInvite = serde_json::from_str(json).unwrap();
    assert_eq!(ci.expires_in_days, 0);
}

#[test]
fn create_invite_large_days() {
    let json = r#"{"expires_in_days": 365}"#;
    let ci: CreateInvite = serde_json::from_str(json).unwrap();
    assert_eq!(ci.expires_in_days, 365);
}

#[test]
fn create_invite_clone() {
    let json = r#"{"expires_in_days": 30}"#;
    let ci: CreateInvite = serde_json::from_str(json).unwrap();
    let cloned = ci.clone();
    assert_eq!(cloned.expires_in_days, 30);
}

// ============================================================================
// schemas/invite.rs — InviteResponse
// ============================================================================

#[test]
fn invite_response_serializes_with_all_fields_some() {
    let now = Utc::now();
    let resp = InviteResponse {
        id: 42,
        code: "ABCD1234".to_string(),
        created_by_id: 1,
        created_by_username: "admin".to_string(),
        used_by_id: Some(5),
        used_by_username: Some("alice".to_string()),
        is_used: true,
        expires_at: Some(now),
        created_at: now,
        used_at: Some(now),
    };

    let json = serde_json::to_value(&resp).expect("InviteResponse must serialize");
    assert_eq!(json["id"], 42);
    assert_eq!(json["code"], "ABCD1234");
    assert_eq!(json["created_by_id"], 1);
    assert_eq!(json["created_by_username"], "admin");
    assert_eq!(json["used_by_id"], 5);
    assert_eq!(json["used_by_username"], "alice");
    assert_eq!(json["is_used"], true);
    assert!(json["expires_at"].as_str().is_some());
    assert!(json["created_at"].as_str().is_some());
    assert!(json["used_at"].as_str().is_some());
}

#[test]
fn invite_response_serializes_with_optional_fields_none() {
    let now = Utc::now();
    let resp = InviteResponse {
        id: 1,
        code: "XYZ".to_string(),
        created_by_id: 2,
        created_by_username: "creator".to_string(),
        used_by_id: None,
        used_by_username: None,
        is_used: false,
        expires_at: None,
        created_at: now,
        used_at: None,
    };

    let json = serde_json::to_value(&resp).expect("InviteResponse must serialize");
    assert_eq!(json["id"], 1);
    assert!(!json["is_used"].as_bool().unwrap());
    // Serde serializes Option::None as null
    assert!(json["used_by_id"].is_null());
    assert!(json["used_by_username"].is_null());
    assert!(json["expires_at"].is_null());
    assert!(json["used_at"].is_null());
}

#[test]
fn invite_response_clone() {
    let now = Utc::now();
    let resp = InviteResponse {
        id: 7,
        code: "CLONE".to_string(),
        created_by_id: 1,
        created_by_username: "admin".to_string(),
        used_by_id: None,
        used_by_username: None,
        is_used: false,
        expires_at: Some(now),
        created_at: now,
        used_at: None,
    };

    let cloned = resp.clone();
    assert_eq!(cloned.id, 7);
    assert_eq!(cloned.code, "CLONE");
    assert!(cloned.used_by_id.is_none());
}

// ============================================================================
// schemas/preferences.rs — UserPreferencesResponse
// ============================================================================

#[test]
fn user_preferences_response_serializes() {
    let resp = UserPreferencesResponse {
        theme: "dark".to_string(),
    };

    let json = serde_json::to_value(&resp).expect("UserPreferencesResponse must serialize");
    assert_eq!(json["theme"], "dark");
}

#[test]
fn user_preferences_response_light_theme() {
    let resp = UserPreferencesResponse {
        theme: "light".to_string(),
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("\"theme\":\"light\""));
}

#[test]
fn user_preferences_response_clone() {
    let resp = UserPreferencesResponse {
        theme: "system".to_string(),
    };
    let cloned = resp.clone();
    assert_eq!(cloned.theme, "system");
}

// ============================================================================
// schemas/preferences.rs — UpdateUserPreferences
// ============================================================================

#[test]
fn update_user_preferences_with_theme() {
    let json = r#"{"theme": "dark"}"#;
    let uup: UpdateUserPreferences =
        serde_json::from_str(json).expect("UpdateUserPreferences must deserialize");
    assert_eq!(uup.theme, Some("dark".to_string()));
}

#[test]
fn update_user_preferences_without_theme_is_none() {
    let json = r#"{}"#;
    let uup: UpdateUserPreferences =
        serde_json::from_str(json).expect("UpdateUserPreferences with empty body must deserialize");
    assert!(uup.theme.is_none());
}

#[test]
fn update_user_preferences_explicit_null_theme() {
    let json = r#"{"theme": null}"#;
    let uup: UpdateUserPreferences = serde_json::from_str(json).unwrap();
    assert!(uup.theme.is_none());
}

#[test]
fn update_user_preferences_clone() {
    let json = r#"{"theme": "auto"}"#;
    let uup: UpdateUserPreferences = serde_json::from_str(json).unwrap();
    let cloned = uup.clone();
    assert_eq!(cloned.theme, Some("auto".to_string()));
}

// ============================================================================
// Cross-type — serialization round-trips / debug formatting
// ============================================================================

#[test]
fn create_user_debug_format() {
    let json = r#"{"username": "debug_user", "email": "d@d.com", "password": "pw"}"#;
    let cu: CreateUser = serde_json::from_str(json).unwrap();
    let debug = format!("{:?}", cu);
    assert!(debug.contains("CreateUser"));
    assert!(debug.contains("debug_user"));
}

#[test]
fn update_setting_debug_format() {
    let json = r#"{"value": "on"}"#;
    let us: UpdateSetting = serde_json::from_str(json).unwrap();
    let debug = format!("{:?}", us);
    assert!(debug.contains("UpdateSetting"));
    assert!(debug.contains("on"));
}

#[test]
fn create_role_debug_format() {
    let json = r#"{"name": "testrole"}"#;
    let cr: CreateRole = serde_json::from_str(json).unwrap();
    let debug = format!("{:?}", cr);
    assert!(debug.contains("CreateRole"));
    assert!(debug.contains("testrole"));
}

#[test]
fn invite_response_debug_format() {
    let now = Utc::now();
    let resp = InviteResponse {
        id: 99,
        code: "DEBUGCODE".to_string(),
        created_by_id: 1,
        created_by_username: "admin".to_string(),
        used_by_id: None,
        used_by_username: None,
        is_used: false,
        expires_at: None,
        created_at: now,
        used_at: None,
    };
    let debug = format!("{:?}", resp);
    assert!(debug.contains("InviteResponse"));
    assert!(debug.contains("DEBUGCODE"));
}
