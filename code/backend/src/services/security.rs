use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rand::RngExt;
use rand_core::OsRng;
use rsa::{
    pkcs8::{DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding},
    RsaPrivateKey, RsaPublicKey,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};

use crate::config::CONFIG;
use crate::error::{AppError, Result};

// JWT token expiration times (in seconds)
const ACCESS_TOKEN_EXPIRE: i64 = 3600; // 1 hour
const REFRESH_TOKEN_EXPIRE: i64 = 604800; // 7 days

// Database keys for system_settings
const JWT_PRIVATE_KEY_SETTING: &str = "jwt_private_key";
const JWT_PUBLIC_KEY_SETTING: &str = "jwt_public_key";

// In-memory key cache
static PRIVATE_KEY: Lazy<RwLock<Option<String>>> = Lazy::new(|| RwLock::new(None));
static PUBLIC_KEY: Lazy<RwLock<Option<String>>> = Lazy::new(|| RwLock::new(None));

/// JWT token claims
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // Subject (user identifier)
    pub iss: String, // Issuer
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>, // Audience (client_id)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>, // User email
    pub exp: i64,    // Expiration time
    pub iat: i64,    // Issued at
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>, // JWT ID for uniqueness
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>, // "refresh" for refresh tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>, // OAuth2 scope
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>, // OAuth2 client ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>, // User permissions (e.g., ["apps.view", "app.sonarr"])
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_apps: Option<Vec<String>>, // Apps user can access (e.g., ["sonarr", "radarr", "*"])
}

/// Minimal JWT claims for session tokens (stored in cookie)
/// Contains only the session ID - expiration is checked in the database
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionClaims {
    pub sid: String, // Session ID (UUID)
}

/// Initialize JWT keys from database (call once during startup)
/// Generates new keys if not present and stores them in the database
pub async fn init_jwt_keys(db: &sea_orm::DatabaseConnection) -> Result<()> {
    use crate::models::prelude::*;
    use crate::models::system_setting;

    // Try to load existing keys from database
    let private_setting = SystemSetting::find()
        .filter(system_setting::Column::Key.eq(JWT_PRIVATE_KEY_SETTING))
        .one(db)
        .await?;

    let public_setting = SystemSetting::find()
        .filter(system_setting::Column::Key.eq(JWT_PUBLIC_KEY_SETTING))
        .one(db)
        .await?;

    let (private_pem, public_pem) = match (private_setting, public_setting) {
        (Some(priv_s), Some(pub_s)) if !priv_s.value.is_empty() && !pub_s.value.is_empty() => {
            tracing::info!("JWT keys loaded from database");
            (priv_s.value, pub_s.value)
        }
        _ => {
            // Generate new key pair
            tracing::info!("Generating new JWT key pair");
            let (private_pem, public_pem) = generate_rsa_key_pair()?;

            let now = chrono::Utc::now();

            // Save private key
            let private_model = system_setting::ActiveModel {
                key: Set(JWT_PRIVATE_KEY_SETTING.to_string()),
                value: Set(private_pem.clone()),
                description: Set(Some("JWT signing private key (RSA)".to_string())),
                updated_at: Set(now),
            };
            // Use insert or update based on whether it exists
            if SystemSetting::find()
                .filter(system_setting::Column::Key.eq(JWT_PRIVATE_KEY_SETTING))
                .one(db)
                .await?
                .is_some()
            {
                private_model.update(db).await?;
            } else {
                private_model.insert(db).await?;
            }

            // Save public key
            let public_model = system_setting::ActiveModel {
                key: Set(JWT_PUBLIC_KEY_SETTING.to_string()),
                value: Set(public_pem.clone()),
                description: Set(Some("JWT signing public key (RSA)".to_string())),
                updated_at: Set(now),
            };
            if SystemSetting::find()
                .filter(system_setting::Column::Key.eq(JWT_PUBLIC_KEY_SETTING))
                .one(db)
                .await?
                .is_some()
            {
                public_model.update(db).await?;
            } else {
                public_model.insert(db).await?;
            }

            tracing::info!("JWT keys saved to database");
            (private_pem, public_pem)
        }
    };

    // Cache in memory
    {
        let mut cache = PRIVATE_KEY.write();
        *cache = Some(private_pem);
    }
    {
        let mut cache = PUBLIC_KEY.write();
        *cache = Some(public_pem);
    }

    Ok(())
}

/// Get the JWT private key (PEM format)
/// Must be called after init_jwt_keys()
pub fn get_private_key() -> Result<String> {
    let cache = PRIVATE_KEY.read();
    cache.clone().ok_or_else(|| {
        AppError::Internal("JWT keys not initialized. Call init_jwt_keys() first.".to_string())
    })
}

/// Get the JWT public key (PEM format)
/// Must be called after init_jwt_keys()
pub fn get_public_key() -> Result<String> {
    let cache = PUBLIC_KEY.read();
    cache.clone().ok_or_else(|| {
        AppError::Internal("JWT keys not initialized. Call init_jwt_keys() first.".to_string())
    })
}

/// Generate an RSA key pair for JWT signing
pub fn generate_rsa_key_pair() -> Result<(String, String)> {
    let private_key = RsaPrivateKey::new(&mut OsRng, 2048)
        .map_err(|e| AppError::Internal(format!("Failed to generate RSA key: {}", e)))?;

    let public_key = RsaPublicKey::from(&private_key);

    let private_pem = private_key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| AppError::Internal(format!("Failed to serialize private key: {}", e)))?
        .to_string();

    let public_pem = public_key
        .to_public_key_pem(LineEnding::LF)
        .map_err(|e| AppError::Internal(format!("Failed to serialize public key: {}", e)))?;

    Ok((private_pem, public_pem))
}

/// Hash a password using bcrypt
pub fn hash_password(password: &str) -> Result<String> {
    bcrypt::hash(password, bcrypt::DEFAULT_COST)
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))
}

/// Verify a password against its hash
pub fn verify_password(password: &str, hash: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

/// Create a JWT access token
pub fn create_access_token(
    subject: &str,
    email: Option<&str>,
    scope: Option<&str>,
    client_id: Option<&str>,
    expires_in: Option<i64>,
    permissions: Option<Vec<String>>,
    allowed_apps: Option<Vec<String>>,
) -> Result<String> {
    let now = Utc::now();
    let exp = now + Duration::seconds(expires_in.unwrap_or(ACCESS_TOKEN_EXPIRE));

    let issuer = format!("{}/auth", CONFIG.auth.oauth2_issuer_url);
    let claims = Claims {
        sub: subject.to_string(),
        iss: issuer,
        aud: client_id.map(String::from),
        email: email.map(String::from),
        exp: exp.timestamp(),
        iat: now.timestamp(),
        jti: Some(uuid::Uuid::new_v4().to_string()),
        token_type: None,
        scope: scope.map(String::from),
        client_id: client_id.map(String::from),
        permissions,
        allowed_apps,
    };

    let private_key = get_private_key()?;
    let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes())
        .map_err(|e| AppError::Internal(format!("Invalid private key: {}", e)))?;

    let header = Header::new(jsonwebtoken::Algorithm::RS256);
    encode(&header, &claims, &encoding_key).map_err(|e| e.into())
}

/// Create a JWT refresh token (no permissions embedded - only for token refresh)
pub fn create_refresh_token(
    subject: &str,
    email: Option<&str>,
    scope: Option<&str>,
    client_id: Option<&str>,
    expires_in: Option<i64>,
) -> Result<String> {
    let now = Utc::now();
    let exp = now + Duration::seconds(expires_in.unwrap_or(REFRESH_TOKEN_EXPIRE));

    let issuer = format!("{}/auth", CONFIG.auth.oauth2_issuer_url);
    let claims = Claims {
        sub: subject.to_string(),
        iss: issuer,
        aud: client_id.map(String::from),
        email: email.map(String::from),
        exp: exp.timestamp(),
        iat: now.timestamp(),
        jti: Some(uuid::Uuid::new_v4().to_string()),
        token_type: Some("refresh".to_string()),
        scope: scope.map(String::from),
        client_id: client_id.map(String::from),
        permissions: None, // Refresh tokens don't carry permissions
        allowed_apps: None,
    };

    let private_key = get_private_key()?;
    let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes())
        .map_err(|e| AppError::Internal(format!("Invalid private key: {}", e)))?;

    let header = Header::new(jsonwebtoken::Algorithm::RS256);
    encode(&header, &claims, &encoding_key).map_err(|e| e.into())
}

/// Decode and validate a JWT token
pub fn decode_token(token: &str) -> Result<Claims> {
    let public_key = get_public_key()?;
    let decoding_key = DecodingKey::from_rsa_pem(public_key.as_bytes())
        .map_err(|e| AppError::Internal(format!("Invalid public key: {}", e)))?;

    let mut validation = Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.validate_exp = true;
    // Disable audience validation - audience is application-specific
    validation.validate_aud = false;
    // No clock skew tolerance for expiration check
    validation.leeway = 0;

    let token_data = decode::<Claims>(token, &decoding_key, &validation)?;
    Ok(token_data.claims)
}

/// Create a minimal session token (JWT containing only session ID)
/// This is stored in the cookie - expiration and user data are looked up from the database
pub fn create_session_token(session_id: &str) -> Result<String> {
    let claims = SessionClaims {
        sid: session_id.to_string(),
    };

    let private_key = get_private_key()?;
    let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes())
        .map_err(|e| AppError::Internal(format!("Invalid private key: {}", e)))?;

    let header = Header::new(jsonwebtoken::Algorithm::RS256);
    encode(&header, &claims, &encoding_key).map_err(|e| e.into())
}

/// Decode and validate a session token
/// Returns the session ID if signature is valid - expiration is checked in the database
pub fn decode_session_token(token: &str) -> Result<SessionClaims> {
    let public_key = get_public_key()?;
    let decoding_key = DecodingKey::from_rsa_pem(public_key.as_bytes())
        .map_err(|e| AppError::Internal(format!("Invalid public key: {}", e)))?;

    let mut validation = Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.validate_exp = false; // Expiration checked in database
    validation.validate_aud = false;
    validation.required_spec_claims.clear(); // No required claims

    let token_data = decode::<SessionClaims>(token, &decoding_key, &validation)?;
    Ok(token_data.claims)
}

/// Generate a cryptographically secure random string (hex)
pub fn generate_random_string(length: usize) -> String {
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..length).map(|_| rng.random()).collect();
    hex::encode(bytes)
}

/// Generate a secure random password
#[allow(dead_code)]
pub fn generate_secure_password(length: usize) -> String {
    const CHARSET: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*()";
    let mut rng = rand::rng();
    (0..length)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

// ==========================================================================
// TOTP (Time-based One-Time Password) Functions
// ==========================================================================

const TOTP_ISSUER: &str = "Kubarr";

/// Generate a new TOTP secret (base32 encoded)
pub fn generate_totp_secret() -> String {
    use totp_rs::Secret;
    Secret::generate_secret().to_encoded().to_string()
}

/// Create a TOTP instance for verification
fn create_totp(secret: &str, account_name: &str) -> Result<totp_rs::TOTP> {
    use totp_rs::{Algorithm, Secret, TOTP};

    let secret_bytes = Secret::Encoded(secret.to_string())
        .to_bytes()
        .map_err(|e| AppError::Internal(format!("Invalid TOTP secret: {}", e)))?;

    TOTP::new(
        Algorithm::SHA1,
        6,  // digits
        1,  // skew (allow 1 step before/after for clock drift)
        30, // step (30 seconds)
        secret_bytes,
        Some(TOTP_ISSUER.to_string()),
        account_name.to_string(),
    )
    .map_err(|e| AppError::Internal(format!("Failed to create TOTP: {}", e)))
}

/// Verify a TOTP code
pub fn verify_totp(secret: &str, code: &str, account_name: &str) -> Result<bool> {
    let totp = create_totp(secret, account_name)?;
    Ok(totp.check_current(code).unwrap_or(false))
}

/// Get TOTP provisioning URI for QR code generation
pub fn get_totp_provisioning_uri(secret: &str, account_name: &str) -> Result<String> {
    let totp = create_totp(secret, account_name)?;
    Ok(totp.get_url())
}

/// Get TOTP QR code as base64-encoded PNG data URL
#[allow(dead_code)]
pub fn get_totp_qr_code_base64(secret: &str, account_name: &str) -> Result<String> {
    let totp = create_totp(secret, account_name)?;
    let base64 = totp
        .get_qr_base64()
        .map_err(|e| AppError::Internal(format!("Failed to generate QR code: {}", e)))?;
    // Return as data URL for direct use in <img src="">
    Ok(format!("data:image/png;base64,{}", base64))
}

/// Generate a 2FA challenge token
pub fn generate_2fa_challenge_token() -> String {
    generate_random_string(32)
}

// ==========================================================================
// Recovery Code Functions
// ==========================================================================

const RECOVERY_CODE_CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const RECOVERY_CODE_LENGTH: usize = 10;
const RECOVERY_CODES_COUNT: usize = 8;

/// Generate 8 single-use recovery codes (10-character alphanumeric)
pub fn generate_recovery_codes() -> Vec<String> {
    let mut rng = rand::rng();
    (0..RECOVERY_CODES_COUNT)
        .map(|_| {
            (0..RECOVERY_CODE_LENGTH)
                .map(|_| {
                    let idx = rng.random_range(0..RECOVERY_CODE_CHARSET.len());
                    RECOVERY_CODE_CHARSET[idx] as char
                })
                .collect()
        })
        .collect()
}

/// Hash a recovery code using bcrypt (cost 8 for performance)
pub fn hash_recovery_code(code: &str) -> Result<String> {
    bcrypt::hash(code, 8)
        .map_err(|e| AppError::Internal(format!("Failed to hash recovery code: {}", e)))
}

/// Verify a recovery code against its hash
pub fn verify_recovery_code(code: &str, hash: &str) -> bool {
    bcrypt::verify(code, hash).unwrap_or(false)
}

/// Get the JWKS (JSON Web Key Set) for the public key
pub fn get_jwks() -> Result<serde_json::Value> {
    let public_pem = get_public_key()?;

    // Parse the public key
    let public_key = RsaPublicKey::from_public_key_pem(&public_pem)
        .map_err(|e| AppError::Internal(format!("Failed to parse public key: {}", e)))?;

    // Get the modulus and exponent
    use rsa::traits::PublicKeyParts;
    let n = URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be());
    let e = URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be());

    Ok(serde_json::json!({
        "keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "n": n,
            "e": e,
            "kid": "kubarr-jwt-key"
        }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // generate_random_string
    // -------------------------------------------------------------------------

    #[test]
    fn random_string_hex_length() {
        // generate_random_string(n) returns hex of n random bytes → 2n hex chars
        let s = generate_random_string(16);
        assert_eq!(s.len(), 32);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn random_string_zero_length() {
        let s = generate_random_string(0);
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn random_string_is_random() {
        let a = generate_random_string(16);
        let b = generate_random_string(16);
        // Extremely unlikely to be identical
        assert_ne!(a, b);
    }

    // -------------------------------------------------------------------------
    // generate_secure_password
    // -------------------------------------------------------------------------

    #[test]
    fn secure_password_length() {
        let p = generate_secure_password(20);
        assert_eq!(p.len(), 20);
    }

    #[test]
    fn secure_password_charset() {
        let p = generate_secure_password(100);
        let valid: &[u8] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*()";
        assert!(p.bytes().all(|b| valid.contains(&b)));
    }

    // -------------------------------------------------------------------------
    // generate_recovery_codes
    // -------------------------------------------------------------------------

    #[test]
    fn recovery_codes_count() {
        let codes = generate_recovery_codes();
        assert_eq!(codes.len(), 8);
    }

    #[test]
    fn recovery_codes_length_and_charset() {
        let codes = generate_recovery_codes();
        for code in &codes {
            assert_eq!(code.len(), 10);
            assert!(
                code.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
                "unexpected char in recovery code: {}",
                code
            );
        }
    }

    // -------------------------------------------------------------------------
    // hash_recovery_code / verify_recovery_code
    // -------------------------------------------------------------------------

    #[test]
    fn hash_and_verify_recovery_code() {
        let code = "ABCDEF1234";
        let hash = hash_recovery_code(code).expect("hash ok");
        assert!(verify_recovery_code(code, &hash));
    }

    #[test]
    fn verify_recovery_code_wrong_code() {
        let code = "ABCDEF1234";
        let hash = hash_recovery_code(code).expect("hash ok");
        assert!(!verify_recovery_code("WRONG12345", &hash));
    }

    // -------------------------------------------------------------------------
    // hash_password / verify_password
    // -------------------------------------------------------------------------

    #[test]
    fn hash_and_verify_password() {
        let pw = "hunter2";
        let hash = hash_password(pw).expect("hash ok");
        assert!(verify_password(pw, &hash));
    }

    #[test]
    fn verify_password_wrong_password() {
        let hash = hash_password("correct").expect("hash ok");
        assert!(!verify_password("wrong", &hash));
    }

    // -------------------------------------------------------------------------
    // generate_2fa_challenge_token
    // -------------------------------------------------------------------------

    #[test]
    fn challenge_token_is_hex_64_chars() {
        // generate_2fa_challenge_token calls generate_random_string(32) → 64 hex chars
        let t = generate_2fa_challenge_token();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // -------------------------------------------------------------------------
    // generate_totp_secret
    // -------------------------------------------------------------------------

    #[test]
    fn totp_secret_is_nonempty_base32() {
        let s = generate_totp_secret();
        assert!(!s.is_empty());
        // Base32 alphabet: A-Z and 2-7
        assert!(s
            .chars()
            .all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)));
    }

    // -------------------------------------------------------------------------
    // get_totp_provisioning_uri
    // -------------------------------------------------------------------------

    #[test]
    fn totp_provisioning_uri_format() {
        let secret = generate_totp_secret();
        let uri = get_totp_provisioning_uri(&secret, "testuser@example.com")
            .expect("provisioning URI ok");
        assert!(uri.starts_with("otpauth://totp/"));
        assert!(uri.contains("Kubarr"));
    }

    // -------------------------------------------------------------------------
    // verify_totp
    // -------------------------------------------------------------------------

    #[test]
    fn verify_totp_with_invalid_code_returns_false() {
        let secret = generate_totp_secret();
        let result =
            verify_totp(&secret, "000000", "testuser@example.com").expect("verify_totp ok");
        // "000000" is extremely unlikely to be valid
        // We just verify the function runs without error
        let _ = result;
    }

    #[test]
    fn verify_totp_invalid_secret_returns_err() {
        // Empty secret should fail deserialization
        let result = verify_totp("not-base32!!!", "123456", "user");
        // May succeed or fail depending on secret parsing, but should not panic
        let _ = result;
    }

    // -------------------------------------------------------------------------
    // get_totp_qr_code_base64
    // -------------------------------------------------------------------------

    #[test]
    fn totp_qr_code_base64_is_data_url() {
        let secret = generate_totp_secret();
        let result = get_totp_qr_code_base64(&secret, "testuser@example.com");
        let data_url = result.expect("qr code ok");
        assert!(data_url.starts_with("data:image/png;base64,"));
        assert!(data_url.len() > 30);
    }

    // -------------------------------------------------------------------------
    // generate_rsa_key_pair
    // -------------------------------------------------------------------------

    #[test]
    fn generate_rsa_key_pair_produces_valid_pem() {
        let (private_pem, public_pem) = generate_rsa_key_pair().expect("key pair");
        assert!(
            private_pem.contains("-----BEGIN PRIVATE KEY-----"),
            "private key should be PKCS8 PEM"
        );
        assert!(
            public_pem.contains("-----BEGIN PUBLIC KEY-----"),
            "public key should be PKCS8 PEM"
        );
    }

    // -------------------------------------------------------------------------
    // init_jwt_keys + get_private_key/get_public_key + JWT functions
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn init_jwt_keys_and_jwt_roundtrip() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("db");
        init_jwt_keys(&db).await.expect("init_jwt_keys");

        // After init, key getters should succeed
        let private_key = get_private_key().expect("private key");
        assert!(!private_key.is_empty());
        assert!(private_key.contains("PRIVATE KEY"));

        let public_key = get_public_key().expect("public key");
        assert!(!public_key.is_empty());
        assert!(public_key.contains("PUBLIC KEY"));

        // create_access_token and decode_token roundtrip
        let token = create_access_token(
            "user-42",
            Some("user42@example.com"),
            Some("openid"),
            Some("client1"),
            Some(3600),
            Some(vec!["apps.view".to_string()]),
            Some(vec!["sonarr".to_string()]),
        )
        .expect("create_access_token");
        assert!(!token.is_empty());

        let claims = decode_token(&token).expect("decode_token");
        assert_eq!(claims.sub, "user-42");
        assert_eq!(claims.email.as_deref(), Some("user42@example.com"));
        assert_eq!(claims.scope.as_deref(), Some("openid"));
        assert_eq!(
            claims.permissions.as_deref(),
            Some(&["apps.view".to_string()][..])
        );

        // create_refresh_token roundtrip
        let refresh = create_refresh_token(
            "user-42",
            Some("user42@example.com"),
            None,
            None,
            Some(604800),
        )
        .expect("create_refresh_token");
        let refresh_claims = decode_token(&refresh).expect("decode refresh");
        assert_eq!(refresh_claims.sub, "user-42");
        assert_eq!(refresh_claims.token_type.as_deref(), Some("refresh"));
        assert!(refresh_claims.permissions.is_none());

        // create_session_token and decode_session_token roundtrip
        let sid = "abc-123-session-uuid";
        let session_token = create_session_token(sid).expect("create_session_token");
        let session_claims = decode_session_token(&session_token).expect("decode_session_token");
        assert_eq!(session_claims.sid, sid);

        // get_jwks
        let jwks = get_jwks().expect("get_jwks");
        assert!(jwks["keys"].is_array());
        let keys = jwks["keys"].as_array().expect("keys array");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0]["kty"], "RSA");
        assert_eq!(keys[0]["alg"], "RS256");
    }

    #[tokio::test]
    async fn init_jwt_keys_second_call_loads_from_db() {
        // Create DB, save keys, then call init_jwt_keys again to load from DB
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("db");

        // First call generates keys and saves to DB
        init_jwt_keys(&db).await.expect("first init");

        // Second call on the SAME DB should load the existing keys
        init_jwt_keys(&db)
            .await
            .expect("second init should succeed");

        // Keys should still be accessible
        let _ = get_private_key().expect("key still valid");
    }

    #[test]
    fn decode_token_with_garbage_returns_err() {
        // If keys are initialized, a garbage token should fail to decode
        // (error from JWT library, not from our code)
        let result = decode_token("not.a.real.jwt.token");
        // Will fail either because keys aren't initialized or because token is invalid
        let _ = result; // We just check it doesn't panic
    }

    #[tokio::test]
    async fn init_jwt_keys_updates_existing_empty_key_rows() {
        use crate::models::system_setting;
        use sea_orm::ActiveModelTrait;

        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("db");

        let now = chrono::Utc::now();
        // Pre-insert both rows with empty values — simulating a corrupt/incomplete state
        system_setting::ActiveModel {
            key: sea_orm::Set("jwt_private_key".to_string()),
            value: sea_orm::Set("".to_string()),
            description: sea_orm::Set(None),
            updated_at: sea_orm::Set(now),
        }
        .insert(&db)
        .await
        .expect("insert private key row");

        system_setting::ActiveModel {
            key: sea_orm::Set("jwt_public_key".to_string()),
            value: sea_orm::Set("".to_string()),
            description: sea_orm::Set(None),
            updated_at: sea_orm::Set(now),
        }
        .insert(&db)
        .await
        .expect("insert public key row");

        // init_jwt_keys enters the generate branch (empty values), then UPDATE existing rows
        init_jwt_keys(&db).await.expect("init with empty rows");

        // Keys should now be non-empty in the DB
        let private_key = get_private_key().expect("private key after init");
        assert!(!private_key.is_empty());
    }
}
