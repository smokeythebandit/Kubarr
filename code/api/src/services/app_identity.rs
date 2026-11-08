//! Signed identity tokens Kubarr hands to applications behind its gateway.
//!
//! An app whose chart declares `kubarr.io/identity-header: "true"` receives an
//! `X-Kubarr-Identity` header on each proxied request from a signed-in user.
//! The header is a short-lived RS256 JWT, signed with Kubarr's session key and
//! verifiable against `GET /auth/jwks`, so the app can sign the user in
//! without a second password and without trusting the network path: the
//! gateway strips any client-supplied copy, and a forged one fails signature
//! verification.

use chrono::{Duration, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::middleware::AuthenticatedUser;
use crate::services::security::get_private_key;

/// Lifetime of an identity token. The gateway caches an authorization,
/// header included, for a few seconds, so this must comfortably exceed that
/// cache while staying short enough that a leaked token is soon useless.
pub const IDENTITY_TOKEN_TTL_SECONDS: i64 = 300;

/// Issuer claim on identity tokens; apps pin it.
pub const IDENTITY_ISSUER: &str = "kubarr";

/// `kid` of the signing key, matching `security::get_jwks()`.
pub const IDENTITY_KEY_ID: &str = "kubarr-jwt-key";

/// Holding this permission makes a user an administrator inside apps that
/// map Kubarr roles: whoever may change Kubarr's own settings may change the
/// app's.
pub const APP_ADMIN_PERMISSION: &str = "settings.manage";

/// Claims an app can rely on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdentityClaims {
    pub iss: String,
    /// Kubarr user id. Stable across renames, so apps link accounts on it.
    pub sub: String,
    /// The app this token is for; an app must reject any other audience.
    pub aud: String,
    pub iat: i64,
    pub exp: i64,
    pub preferred_username: String,
    pub email: String,
    pub is_admin: bool,
}

impl IdentityClaims {
    pub fn for_user(user: &AuthenticatedUser, app_name: &str) -> Self {
        let now = Utc::now();
        Self {
            iss: IDENTITY_ISSUER.to_string(),
            sub: user.user.id.to_string(),
            aud: app_name.to_string(),
            iat: now.timestamp(),
            exp: (now + Duration::seconds(IDENTITY_TOKEN_TTL_SECONDS)).timestamp(),
            preferred_username: user.user.username.clone(),
            email: user.user.email.clone(),
            is_admin: is_app_admin(&user.permissions),
        }
    }
}

/// Whether a permission set maps to administrator inside an app.
pub fn is_app_admin(permissions: &[String]) -> bool {
    permissions.iter().any(|p| p == APP_ADMIN_PERMISSION)
}

/// Mint the identity token for `user` addressed to `app_name`.
pub fn mint(user: &AuthenticatedUser, app_name: &str) -> Result<String> {
    let private_key = get_private_key()?;
    let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes())
        .map_err(|e| AppError::Internal(format!("Invalid private key: {}", e)))?;
    let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(IDENTITY_KEY_ID.to_string());
    encode(
        &header,
        &IdentityClaims::for_user(user, app_name),
        &encoding_key,
    )
    .map_err(|e| e.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user;
    use jsonwebtoken::{decode, DecodingKey, Validation};

    fn user(permissions: &[&str]) -> AuthenticatedUser {
        AuthenticatedUser {
            user: user::Model {
                id: 42,
                username: "alice".into(),
                email: "alice@example.com".into(),
                hashed_password: String::new(),
                is_active: true,
                is_approved: true,
                totp_secret: None,
                totp_enabled: false,
                totp_verified_at: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            permissions: permissions.iter().map(|p| p.to_string()).collect(),
        }
    }

    #[test]
    fn admin_follows_settings_manage() {
        assert!(is_app_admin(&["settings.manage".into()]));
        assert!(!is_app_admin(&["settings.view".into(), "app.*".into()]));
    }

    #[tokio::test]
    async fn token_verifies_against_the_published_key_and_names_the_app() {
        use sea_orm_migration::MigratorTrait;
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        crate::migrations::Migrator::up(&db, None).await.unwrap();
        crate::services::init_jwt_keys(&db).await.unwrap();
        let token = mint(&user(&["app.jellyfin", "settings.manage"]), "jellyfin").unwrap();

        let public = crate::services::security::get_public_key().unwrap();
        let mut validation = Validation::new(jsonwebtoken::Algorithm::RS256);
        validation.set_audience(&["jellyfin"]);
        validation.set_issuer(&[IDENTITY_ISSUER]);
        let data = decode::<IdentityClaims>(
            &token,
            &DecodingKey::from_rsa_pem(public.as_bytes()).unwrap(),
            &validation,
        )
        .unwrap();
        assert_eq!(data.header.kid.as_deref(), Some(IDENTITY_KEY_ID));
        assert_eq!(data.claims.sub, "42");
        assert_eq!(data.claims.preferred_username, "alice");
        assert!(data.claims.is_admin);
        assert_eq!(
            data.claims.exp - data.claims.iat,
            IDENTITY_TOKEN_TTL_SECONDS
        );

        let mut other_app = Validation::new(jsonwebtoken::Algorithm::RS256);
        other_app.set_audience(&["seerr"]);
        assert!(decode::<IdentityClaims>(
            &token,
            &DecodingKey::from_rsa_pem(public.as_bytes()).unwrap(),
            &other_app,
        )
        .is_err());
    }
}
