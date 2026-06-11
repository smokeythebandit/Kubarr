use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::{role, user};

#[derive(Debug, Clone, Deserialize)]
pub struct CreateUser {
    pub username: String,
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub role_ids: Vec<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateUser {
    pub email: Option<String>,
    pub is_active: Option<bool>,
    pub is_approved: Option<bool>,
    pub role_ids: Option<Vec<i64>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserResponse {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub is_active: bool,
    pub is_approved: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub roles: Vec<RoleInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoleInfo {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
}

impl From<role::Model> for RoleInfo {
    fn from(role: role::Model) -> Self {
        Self {
            id: role.id,
            name: role.name,
            description: role.description,
        }
    }
}

impl UserResponse {
    pub fn from_user_with_roles(user: user::Model, roles: Vec<role::Model>) -> Self {
        Self {
            id: user.id,
            username: user.username,
            email: user.email,
            is_active: user.is_active,
            is_approved: user.is_approved,
            created_at: user.created_at,
            updated_at: user.updated_at,
            roles: roles.into_iter().map(RoleInfo::from).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_role_model(id: i64, name: &str) -> crate::models::role::Model {
        crate::models::role::Model {
            id,
            name: name.to_string(),
            description: None,
            is_system: false,
            requires_2fa: false,
            created_at: Utc::now(),
        }
    }

    fn make_user_model() -> crate::models::user::Model {
        crate::models::user::Model {
            id: 1,
            username: "alice".to_string(),
            email: "alice@example.com".to_string(),
            hashed_password: "hashed".to_string(),
            is_active: true,
            is_approved: true,
            totp_secret: None,
            totp_enabled: false,
            totp_verified_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn create_user_deser() {
        let r: CreateUser = serde_json::from_str(
            r#"{"username":"bob","email":"bob@example.com","password":"pass"}"#,
        )
        .expect("deser");
        assert_eq!(r.username, "bob");
        assert_eq!(r.role_ids, Vec::<i64>::new());
    }

    #[test]
    fn create_user_deser_with_roles() {
        let r: CreateUser = serde_json::from_str(
            r#"{"username":"bob","email":"b@b.com","password":"p","role_ids":[1,2]}"#,
        )
        .expect("deser");
        assert_eq!(r.role_ids, vec![1, 2]);
    }

    #[test]
    fn update_user_deser_empty() {
        let r: UpdateUser = serde_json::from_str("{}").expect("deser");
        assert!(r.email.is_none());
        assert!(r.is_active.is_none());
    }

    #[test]
    fn update_user_deser_full() {
        let r: UpdateUser = serde_json::from_str(
            r#"{"email":"new@e.com","is_active":true,"is_approved":false,"role_ids":[3]}"#,
        )
        .expect("deser");
        assert_eq!(r.email.as_deref(), Some("new@e.com"));
        assert_eq!(r.is_active, Some(true));
    }

    #[test]
    fn role_info_from_model() {
        let model = make_role_model(5, "moderator");
        let info = RoleInfo::from(model);
        assert_eq!(info.id, 5);
        assert_eq!(info.name, "moderator");
    }

    #[test]
    fn role_info_ser() {
        let info = RoleInfo {
            id: 1,
            name: "admin".to_string(),
            description: Some("Admin".to_string()),
        };
        let json = serde_json::to_string(&info).expect("ser");
        assert!(json.contains("\"name\":\"admin\""));
    }

    #[test]
    fn user_response_from_user_with_roles() {
        let user = make_user_model();
        let roles = vec![make_role_model(1, "admin")];
        let resp = UserResponse::from_user_with_roles(user, roles);
        assert_eq!(resp.id, 1);
        assert_eq!(resp.username, "alice");
        assert_eq!(resp.roles.len(), 1);
        assert_eq!(resp.roles[0].name, "admin");
    }

    #[test]
    fn user_response_ser() {
        let user = make_user_model();
        let resp = UserResponse::from_user_with_roles(user, vec![]);
        let json = serde_json::to_string(&resp).expect("ser");
        assert!(json.contains("\"username\":\"alice\""));
        // hashed_password should NOT appear in serialized output (skip_serializing)
        assert!(!json.contains("hashed_password"));
    }
}
