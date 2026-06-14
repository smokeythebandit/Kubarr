//! Tests for schema conversion functions in `schemas/role.rs` and `schemas/user.rs`
//!
//! These schemas are utility types used internally. Their conversion functions
//! are exercised here to ensure line coverage.

use chrono::Utc;

use kubarr::models::{role, user};
use kubarr::schemas::role::RoleResponse;
use kubarr::schemas::user::{RoleInfo, UserResponse};

// ============================================================================
// schemas/role.rs — RoleResponse::from_role_with_permissions
// ============================================================================

fn make_role_model(id: i64, name: &str, is_system: bool) -> role::Model {
    let now = Utc::now();
    role::Model {
        id,
        name: name.to_string(),
        description: Some("Test role description".to_string()),
        is_system,
        requires_2fa: false,
        created_at: now,
    }
}

#[test]
fn test_role_response_from_role_with_permissions_basic() {
    let role = make_role_model(1, "admin", true);
    let perms = vec!["apps.view".to_string(), "users.*".to_string()];

    let resp = RoleResponse::from_role_with_permissions(role.clone(), perms.clone());

    assert_eq!(resp.id, 1);
    assert_eq!(resp.name, "admin");
    assert_eq!(resp.description, Some("Test role description".to_string()));
    assert!(resp.is_system);
    assert_eq!(resp.app_permissions, perms);
}

#[test]
fn test_role_response_from_role_with_no_permissions() {
    let role = make_role_model(2, "viewer", false);
    let resp = RoleResponse::from_role_with_permissions(role, vec![]);

    assert_eq!(resp.id, 2);
    assert_eq!(resp.name, "viewer");
    assert!(!resp.is_system);
    assert!(resp.app_permissions.is_empty());
}

#[test]
fn test_role_response_preserves_timestamp() {
    let role = make_role_model(3, "custom", false);
    let expected_ts = role.created_at;
    let resp = RoleResponse::from_role_with_permissions(role, vec![]);

    assert_eq!(resp.created_at, expected_ts);
}

#[test]
fn test_role_response_preserves_description() {
    let now = Utc::now();
    let role = role::Model {
        id: 10,
        name: "norole".to_string(),
        description: None,
        is_system: false,
        requires_2fa: false,
        created_at: now,
    };

    let resp = RoleResponse::from_role_with_permissions(role, vec![]);
    assert_eq!(resp.description, None);
}

// ============================================================================
// schemas/user.rs — RoleInfo::from + UserResponse::from_user_with_roles
// ============================================================================

fn make_user_model(id: i64, username: &str, email: &str) -> user::Model {
    let now = Utc::now();
    user::Model {
        id,
        username: username.to_string(),
        email: email.to_string(),
        hashed_password: "hashed".to_string(),
        is_active: true,
        is_approved: true,
        totp_secret: None,
        totp_enabled: false,
        totp_verified_at: None,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn test_role_info_from_role_model() {
    let role = make_role_model(5, "editor", false);
    let info: RoleInfo = role.into();

    assert_eq!(info.id, 5);
    assert_eq!(info.name, "editor");
    assert_eq!(info.description, Some("Test role description".to_string()));
}

#[test]
fn test_role_info_from_system_role() {
    let now = Utc::now();
    let role = role::Model {
        id: 1,
        name: "admin".to_string(),
        description: None,
        is_system: true,
        requires_2fa: false,
        created_at: now,
    };
    let info: RoleInfo = role.into();

    assert_eq!(info.id, 1);
    assert_eq!(info.name, "admin");
    assert_eq!(info.description, None);
}

#[test]
fn test_user_response_from_user_with_roles_empty() {
    let user = make_user_model(1, "alice", "alice@example.com");
    let resp = UserResponse::from_user_with_roles(user.clone(), vec![]);

    assert_eq!(resp.id, 1);
    assert_eq!(resp.username, "alice");
    assert_eq!(resp.email, "alice@example.com");
    assert!(resp.is_active);
    assert!(resp.is_approved);
    assert!(resp.roles.is_empty());
}

#[test]
fn test_user_response_from_user_with_roles() {
    let user = make_user_model(2, "bob", "bob@example.com");
    let role1 = make_role_model(1, "admin", true);
    let role2 = make_role_model(2, "viewer", false);
    let resp = UserResponse::from_user_with_roles(user, vec![role1, role2]);

    assert_eq!(resp.id, 2);
    assert_eq!(resp.roles.len(), 2);
    assert_eq!(resp.roles[0].name, "admin");
    assert_eq!(resp.roles[1].name, "viewer");
}

#[test]
fn test_user_response_preserves_timestamps() {
    let user = make_user_model(3, "carol", "carol@example.com");
    let expected_created = user.created_at;
    let expected_updated = user.updated_at;
    let resp = UserResponse::from_user_with_roles(user, vec![]);

    assert_eq!(resp.created_at, expected_created);
    assert_eq!(resp.updated_at, expected_updated);
}

#[test]
fn test_user_response_inactive_not_approved() {
    let now = Utc::now();
    let user = user::Model {
        id: 99,
        username: "inactive".to_string(),
        email: "inactive@example.com".to_string(),
        hashed_password: "pw".to_string(),
        is_active: false,
        is_approved: false,
        totp_secret: None,
        totp_enabled: false,
        totp_verified_at: None,
        created_at: now,
        updated_at: now,
    };
    let resp = UserResponse::from_user_with_roles(user, vec![]);
    assert!(!resp.is_active);
    assert!(!resp.is_approved);
}
