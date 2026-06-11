use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, utoipa::ToSchema)]
#[sea_orm(table_name = "audit_logs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[schema(value_type = String)]
    pub timestamp: DateTimeUtc,
    pub user_id: Option<i64>,
    pub username: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub details: Option<String>, // JSON string for flexible data
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub success: bool,
    pub error_message: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

// Audit action types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditAction {
    // Authentication
    Login,
    LoginFailed,
    Logout,
    TokenRefresh,
    TwoFactorEnabled,
    TwoFactorDisabled,
    TwoFactorVerified,
    TwoFactorFailed,
    PasswordChanged,

    // User management
    UserCreated,
    UserUpdated,
    UserDeleted,
    UserApproved,
    UserDeactivated,
    UserActivated,

    // Role management
    RoleCreated,
    RoleUpdated,
    RoleDeleted,
    RoleAssigned,
    RoleUnassigned,

    // App management
    AppInstalled,
    AppUninstalled,
    AppStarted,
    AppStopped,
    AppRestarted,
    AppConfigured,
    AppAccessed,

    // System
    SystemSettingChanged,
    InviteCreated,
    InviteUsed,
    InviteDeleted,

    // API access
    ApiAccess,
}

impl std::fmt::Display for AuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditAction::Login => write!(f, "login"),
            AuditAction::LoginFailed => write!(f, "login_failed"),
            AuditAction::Logout => write!(f, "logout"),
            AuditAction::TokenRefresh => write!(f, "token_refresh"),
            AuditAction::TwoFactorEnabled => write!(f, "2fa_enabled"),
            AuditAction::TwoFactorDisabled => write!(f, "2fa_disabled"),
            AuditAction::TwoFactorVerified => write!(f, "2fa_verified"),
            AuditAction::TwoFactorFailed => write!(f, "2fa_failed"),
            AuditAction::PasswordChanged => write!(f, "password_changed"),
            AuditAction::UserCreated => write!(f, "user_created"),
            AuditAction::UserUpdated => write!(f, "user_updated"),
            AuditAction::UserDeleted => write!(f, "user_deleted"),
            AuditAction::UserApproved => write!(f, "user_approved"),
            AuditAction::UserDeactivated => write!(f, "user_deactivated"),
            AuditAction::UserActivated => write!(f, "user_activated"),
            AuditAction::RoleCreated => write!(f, "role_created"),
            AuditAction::RoleUpdated => write!(f, "role_updated"),
            AuditAction::RoleDeleted => write!(f, "role_deleted"),
            AuditAction::RoleAssigned => write!(f, "role_assigned"),
            AuditAction::RoleUnassigned => write!(f, "role_unassigned"),
            AuditAction::AppInstalled => write!(f, "app_installed"),
            AuditAction::AppUninstalled => write!(f, "app_uninstalled"),
            AuditAction::AppStarted => write!(f, "app_started"),
            AuditAction::AppStopped => write!(f, "app_stopped"),
            AuditAction::AppRestarted => write!(f, "app_restarted"),
            AuditAction::AppConfigured => write!(f, "app_configured"),
            AuditAction::AppAccessed => write!(f, "app_accessed"),
            AuditAction::SystemSettingChanged => write!(f, "system_setting_changed"),
            AuditAction::InviteCreated => write!(f, "invite_created"),
            AuditAction::InviteUsed => write!(f, "invite_used"),
            AuditAction::InviteDeleted => write!(f, "invite_deleted"),
            AuditAction::ApiAccess => write!(f, "api_access"),
        }
    }
}

// Resource types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResourceType {
    User,
    Role,
    App,
    System,
    Invite,
    Session,
}

impl std::fmt::Display for ResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceType::User => write!(f, "user"),
            ResourceType::Role => write!(f, "role"),
            ResourceType::App => write!(f, "app"),
            ResourceType::System => write!(f, "system"),
            ResourceType::Invite => write!(f, "invite"),
            ResourceType::Session => write!(f, "session"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // AuditAction Display
    // -------------------------------------------------------------------------

    #[test]
    fn audit_action_display_authentication() {
        assert_eq!(AuditAction::Login.to_string(), "login");
        assert_eq!(AuditAction::LoginFailed.to_string(), "login_failed");
        assert_eq!(AuditAction::Logout.to_string(), "logout");
        assert_eq!(AuditAction::TokenRefresh.to_string(), "token_refresh");
        assert_eq!(AuditAction::TwoFactorEnabled.to_string(), "2fa_enabled");
        assert_eq!(AuditAction::TwoFactorDisabled.to_string(), "2fa_disabled");
        assert_eq!(AuditAction::TwoFactorVerified.to_string(), "2fa_verified");
        assert_eq!(AuditAction::TwoFactorFailed.to_string(), "2fa_failed");
        assert_eq!(AuditAction::PasswordChanged.to_string(), "password_changed");
    }

    #[test]
    fn audit_action_display_user_management() {
        assert_eq!(AuditAction::UserCreated.to_string(), "user_created");
        assert_eq!(AuditAction::UserUpdated.to_string(), "user_updated");
        assert_eq!(AuditAction::UserDeleted.to_string(), "user_deleted");
        assert_eq!(AuditAction::UserApproved.to_string(), "user_approved");
        assert_eq!(AuditAction::UserDeactivated.to_string(), "user_deactivated");
        assert_eq!(AuditAction::UserActivated.to_string(), "user_activated");
    }

    #[test]
    fn audit_action_display_role_management() {
        assert_eq!(AuditAction::RoleCreated.to_string(), "role_created");
        assert_eq!(AuditAction::RoleUpdated.to_string(), "role_updated");
        assert_eq!(AuditAction::RoleDeleted.to_string(), "role_deleted");
        assert_eq!(AuditAction::RoleAssigned.to_string(), "role_assigned");
        assert_eq!(AuditAction::RoleUnassigned.to_string(), "role_unassigned");
    }

    #[test]
    fn audit_action_display_app_management() {
        assert_eq!(AuditAction::AppInstalled.to_string(), "app_installed");
        assert_eq!(AuditAction::AppUninstalled.to_string(), "app_uninstalled");
        assert_eq!(AuditAction::AppStarted.to_string(), "app_started");
        assert_eq!(AuditAction::AppStopped.to_string(), "app_stopped");
        assert_eq!(AuditAction::AppRestarted.to_string(), "app_restarted");
        assert_eq!(AuditAction::AppConfigured.to_string(), "app_configured");
        assert_eq!(AuditAction::AppAccessed.to_string(), "app_accessed");
    }

    #[test]
    fn audit_action_display_system() {
        assert_eq!(
            AuditAction::SystemSettingChanged.to_string(),
            "system_setting_changed"
        );
        assert_eq!(AuditAction::InviteCreated.to_string(), "invite_created");
        assert_eq!(AuditAction::InviteUsed.to_string(), "invite_used");
        assert_eq!(AuditAction::InviteDeleted.to_string(), "invite_deleted");
        assert_eq!(AuditAction::ApiAccess.to_string(), "api_access");
    }

    // -------------------------------------------------------------------------
    // ResourceType Display
    // -------------------------------------------------------------------------

    #[test]
    fn resource_type_display() {
        assert_eq!(ResourceType::User.to_string(), "user");
        assert_eq!(ResourceType::Role.to_string(), "role");
        assert_eq!(ResourceType::App.to_string(), "app");
        assert_eq!(ResourceType::System.to_string(), "system");
        assert_eq!(ResourceType::Invite.to_string(), "invite");
        assert_eq!(ResourceType::Session.to_string(), "session");
    }

    // -------------------------------------------------------------------------
    // AuditAction serde
    // -------------------------------------------------------------------------

    #[test]
    fn audit_action_serde_roundtrip() {
        let actions = vec![
            AuditAction::Login,
            AuditAction::AppInstalled,
            AuditAction::ApiAccess,
        ];
        for action in actions {
            let json = serde_json::to_string(&action).expect("serialize");
            let back: AuditAction = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(action.to_string(), back.to_string());
        }
    }
}
