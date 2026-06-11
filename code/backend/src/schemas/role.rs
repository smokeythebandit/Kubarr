use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::role;

#[derive(Debug, Clone, Deserialize)]
pub struct CreateRole {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub app_names: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateRole {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetRoleApps {
    pub app_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoleResponse {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub is_system: bool,
    pub created_at: DateTime<Utc>,
    pub app_permissions: Vec<String>,
}

impl RoleResponse {
    pub fn from_role_with_permissions(role: role::Model, app_permissions: Vec<String>) -> Self {
        Self {
            id: role.id,
            name: role.name,
            description: role.description,
            is_system: role.is_system,
            created_at: role.created_at,
            app_permissions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_role_model() -> crate::models::role::Model {
        crate::models::role::Model {
            id: 1,
            name: "admin".to_string(),
            description: Some("Administrator".to_string()),
            is_system: true,
            requires_2fa: false,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn create_role_deser_full() {
        let r: CreateRole = serde_json::from_str(
            r#"{"name":"editor","description":"Edit role","app_names":["sonarr"]}"#,
        )
        .expect("deser");
        assert_eq!(r.name, "editor");
        assert_eq!(r.description.as_deref(), Some("Edit role"));
        assert_eq!(r.app_names, vec!["sonarr"]);
    }

    #[test]
    fn create_role_deser_defaults() {
        let r: CreateRole = serde_json::from_str(r#"{"name":"viewer"}"#).expect("deser");
        assert_eq!(r.app_names, Vec::<String>::new());
        assert!(r.description.is_none());
    }

    #[test]
    fn update_role_deser_empty() {
        let r: UpdateRole = serde_json::from_str("{}").expect("deser");
        assert!(r.name.is_none());
        assert!(r.description.is_none());
    }

    #[test]
    fn update_role_deser_full() {
        let r: UpdateRole =
            serde_json::from_str(r#"{"name":"newname","description":"new desc"}"#).expect("deser");
        assert_eq!(r.name.as_deref(), Some("newname"));
    }

    #[test]
    fn set_role_apps_deser() {
        let r: SetRoleApps =
            serde_json::from_str(r#"{"app_names":["sonarr","radarr"]}"#).expect("deser");
        assert_eq!(r.app_names, vec!["sonarr", "radarr"]);
    }

    #[test]
    fn role_response_from_role_with_permissions() {
        let model = make_role_model();
        let resp = RoleResponse::from_role_with_permissions(model, vec!["apps.view".to_string()]);
        assert_eq!(resp.id, 1);
        assert_eq!(resp.name, "admin");
        assert!(resp.is_system);
        assert_eq!(resp.app_permissions, vec!["apps.view"]);
    }

    #[test]
    fn role_response_ser() {
        let model = make_role_model();
        let resp = RoleResponse::from_role_with_permissions(model, vec![]);
        let json = serde_json::to_string(&resp).expect("ser");
        assert!(json.contains("\"name\":\"admin\""));
    }
}
