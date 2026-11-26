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

/// Audit action types.
///
/// Every variant is also a candidate notification event type (see
/// [`AuditAction::notifiable`]); the derived `EnumIter` keeps that list exhaustive.
#[derive(Debug, Clone, Serialize, Deserialize, EnumIter)]
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
    AppUpdated,
    AppUpdateFailed,
    AppUpdateRollbackFailed,
    /// A requested rollback finished and the app is healthy.
    AppRolledBack,
    /// A requested rollback failed or left the app unhealthy.
    AppRollbackFailed,

    // VPN management
    VpnProviderCreated,
    VpnProviderUpdated,
    VpnProviderDeleted,
    VpnAssigned,
    VpnRemoved,

    // System
    SystemSettingChanged,
    InviteCreated,
    InviteUsed,
    InviteDeleted,
    /// The assistant's spend this month passed 80 % of its budget.
    AssistantBudgetWarning,
    /// The assistant ran a read tool for a user (not notifiable).
    AssistantToolCall,
    /// The assistant proposed an action (not notifiable).
    AssistantActionProposed,
    /// The person clicked Run on a proposed action (not notifiable).
    AssistantActionConfirmed,
    /// The person cancelled a proposed action (not notifiable).
    AssistantActionCancelled,

    // Notification destinations
    NotificationVerificationSent,
    NotificationDestinationVerified,

    // API access
    ApiAccess,
}

impl AuditAction {
    /// Every variant, in declaration order.
    pub fn all() -> impl Iterator<Item = AuditAction> {
        <AuditAction as sea_orm::Iterable>::iter()
    }

    /// Whether this action makes sense as a user-facing notification event.
    ///
    /// Excluded (pure noise or circular):
    /// - `api_access`, `token_refresh`, `app_accessed`: emitted on routine traffic,
    ///   would flood every channel.
    /// - `notification_verification_sent`, `notification_destination_verified`: the
    ///   user is in the middle of configuring that very destination.
    /// - `assistant_tool_call`: one per tool the assistant runs; audit only.
    ///
    /// The match is exhaustive on purpose: a new variant does not compile until it
    /// is classified here.
    pub fn is_notifiable(&self) -> bool {
        match self {
            AuditAction::ApiAccess
            | AuditAction::TokenRefresh
            | AuditAction::AppAccessed
            | AuditAction::NotificationVerificationSent
            | AuditAction::NotificationDestinationVerified
            | AuditAction::AssistantToolCall
            | AuditAction::AssistantActionProposed
            | AuditAction::AssistantActionConfirmed
            | AuditAction::AssistantActionCancelled
            // The person asked for it and is watching the card.
            | AuditAction::AppRolledBack => false,
            AuditAction::Login
            | AuditAction::LoginFailed
            | AuditAction::Logout
            | AuditAction::TwoFactorEnabled
            | AuditAction::TwoFactorDisabled
            | AuditAction::TwoFactorVerified
            | AuditAction::TwoFactorFailed
            | AuditAction::PasswordChanged
            | AuditAction::UserCreated
            | AuditAction::UserUpdated
            | AuditAction::UserDeleted
            | AuditAction::UserApproved
            | AuditAction::UserDeactivated
            | AuditAction::UserActivated
            | AuditAction::RoleCreated
            | AuditAction::RoleUpdated
            | AuditAction::RoleDeleted
            | AuditAction::RoleAssigned
            | AuditAction::RoleUnassigned
            | AuditAction::AppInstalled
            | AuditAction::AppUninstalled
            | AuditAction::AppStarted
            | AuditAction::AppStopped
            | AuditAction::AppRestarted
            | AuditAction::AppConfigured
            | AuditAction::AppUpdated
            | AuditAction::AppUpdateFailed
            | AuditAction::AppUpdateRollbackFailed
            | AuditAction::AppRollbackFailed
            | AuditAction::VpnProviderCreated
            | AuditAction::VpnProviderUpdated
            | AuditAction::VpnProviderDeleted
            | AuditAction::VpnAssigned
            | AuditAction::VpnRemoved
            | AuditAction::SystemSettingChanged
            | AuditAction::InviteCreated
            | AuditAction::InviteUsed
            | AuditAction::InviteDeleted
            | AuditAction::AssistantBudgetWarning => true,
        }
    }

    /// Every action that can be configured as a notification event.
    pub fn notifiable() -> impl Iterator<Item = AuditAction> {
        Self::all().filter(AuditAction::is_notifiable)
    }
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
            AuditAction::AppUpdated => write!(f, "app_updated"),
            AuditAction::AppUpdateFailed => write!(f, "app_update_failed"),
            AuditAction::AppUpdateRollbackFailed => write!(f, "app_update_rollback_failed"),
            AuditAction::AppRolledBack => write!(f, "app_rolled_back"),
            AuditAction::AppRollbackFailed => write!(f, "app_rollback_failed"),
            AuditAction::VpnProviderCreated => write!(f, "vpn_provider_created"),
            AuditAction::VpnProviderUpdated => write!(f, "vpn_provider_updated"),
            AuditAction::VpnProviderDeleted => write!(f, "vpn_provider_deleted"),
            AuditAction::VpnAssigned => write!(f, "vpn_assigned"),
            AuditAction::VpnRemoved => write!(f, "vpn_removed"),
            AuditAction::SystemSettingChanged => write!(f, "system_setting_changed"),
            AuditAction::InviteCreated => write!(f, "invite_created"),
            AuditAction::InviteUsed => write!(f, "invite_used"),
            AuditAction::InviteDeleted => write!(f, "invite_deleted"),
            AuditAction::AssistantBudgetWarning => write!(f, "assistant_budget_warning"),
            AuditAction::AssistantToolCall => write!(f, "assistant_tool_call"),
            AuditAction::AssistantActionProposed => write!(f, "assistant_action_proposed"),
            AuditAction::AssistantActionConfirmed => write!(f, "assistant_action_confirmed"),
            AuditAction::AssistantActionCancelled => write!(f, "assistant_action_cancelled"),
            AuditAction::NotificationVerificationSent => {
                write!(f, "notification_verification_sent")
            }
            AuditAction::NotificationDestinationVerified => {
                write!(f, "notification_destination_verified")
            }
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
    Vpn,
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
            ResourceType::Vpn => write!(f, "vpn"),
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
        assert_eq!(AuditAction::AppUpdated.to_string(), "app_updated");
        assert_eq!(
            AuditAction::AppUpdateFailed.to_string(),
            "app_update_failed"
        );
        assert_eq!(
            AuditAction::AppUpdateRollbackFailed.to_string(),
            "app_update_rollback_failed"
        );
        assert_eq!(AuditAction::AppAccessed.to_string(), "app_accessed");
    }

    #[test]
    fn vpn_audit_values() {
        for (action, expected) in [
            (AuditAction::VpnProviderCreated, "vpn_provider_created"),
            (AuditAction::VpnProviderUpdated, "vpn_provider_updated"),
            (AuditAction::VpnProviderDeleted, "vpn_provider_deleted"),
            (AuditAction::VpnAssigned, "vpn_assigned"),
            (AuditAction::VpnRemoved, "vpn_removed"),
        ] {
            assert_eq!(action.to_string(), expected);
        }
        assert_eq!(ResourceType::Vpn.to_string(), "vpn");
    }

    #[test]
    fn audit_action_display_system() {
        assert_eq!(
            AuditAction::SystemSettingChanged.to_string(),
            "system_setting_changed"
        );
        assert_eq!(AuditAction::InviteCreated.to_string(), "invite_created");
        assert_eq!(AuditAction::InviteUsed.to_string(), "invite_used");
        assert_eq!(
            AuditAction::AssistantBudgetWarning.to_string(),
            "assistant_budget_warning"
        );
        assert_eq!(
            AuditAction::AssistantToolCall.to_string(),
            "assistant_tool_call"
        );
        assert_eq!(
            AuditAction::AssistantActionProposed.to_string(),
            "assistant_action_proposed"
        );
        assert_eq!(
            AuditAction::AssistantActionConfirmed.to_string(),
            "assistant_action_confirmed"
        );
        assert_eq!(
            AuditAction::AssistantActionCancelled.to_string(),
            "assistant_action_cancelled"
        );
        assert_eq!(AuditAction::InviteDeleted.to_string(), "invite_deleted");
        assert_eq!(AuditAction::ApiAccess.to_string(), "api_access");
    }

    #[test]
    fn audit_action_all_is_exhaustive_and_unique() {
        let names: Vec<String> = AuditAction::all().map(|a| a.to_string()).collect();
        assert_eq!(names.first().map(String::as_str), Some("login"));
        assert_eq!(names.last().map(String::as_str), Some("api_access"));
        let mut deduped = names.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(deduped.len(), names.len(), "duplicate Display values");
    }

    #[test]
    fn audit_action_notifiable_excludes_noise() {
        let names: Vec<String> = AuditAction::notifiable().map(|a| a.to_string()).collect();
        for noise in [
            "api_access",
            "token_refresh",
            "app_accessed",
            "notification_verification_sent",
            "notification_destination_verified",
            "assistant_tool_call",
            "assistant_action_proposed",
            "assistant_action_confirmed",
            "assistant_action_cancelled",
            "app_rolled_back",
        ] {
            assert!(
                !names.iter().any(|n| n == noise),
                "{noise} must be excluded"
            );
        }
        assert_eq!(names.len(), AuditAction::all().count() - 10);
        assert!(names.iter().any(|n| n == "assistant_budget_warning"));
        assert!(names.iter().any(|n| n == "app_rollback_failed"));
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
