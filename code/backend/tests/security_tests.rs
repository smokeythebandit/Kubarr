use kubarr::services::security::{
    create_access_token, create_refresh_token, create_session_token, decode_session_token,
    decode_token, generate_random_string, generate_recovery_codes, generate_rsa_key_pair,
    generate_totp_secret, get_jwks, get_totp_provisioning_uri, hash_password, hash_recovery_code,
    init_jwt_keys, verify_password, verify_recovery_code, verify_totp,
};
use once_cell::sync::Lazy;
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::{RsaPrivateKey, RsaPublicKey};
use tokio::sync::Mutex as AsyncMutex;

mod common;
use common::create_test_db;

/// Serializes all JWT tests to prevent concurrent key initialization from causing
/// InvalidSignature errors (global PRIVATE_KEY/PUBLIC_KEY statics get overwritten).
static JWT_TEST_LOCK: Lazy<AsyncMutex<()>> = Lazy::new(|| AsyncMutex::new(()));

// ==========================================================================
// Password Hashing Tests
// ==========================================================================

#[test]
fn test_password_hashing() {
    let password = "test_password123";
    let hash = hash_password(password).unwrap();
    assert!(verify_password(password, &hash));
    assert!(!verify_password("wrong_password", &hash));
}

#[test]
fn test_password_hashing_empty_password() {
    let password = "";
    let hash = hash_password(password).unwrap();
    assert!(verify_password(password, &hash));
    assert!(!verify_password("not_empty", &hash));
}

#[test]
fn test_password_hashing_unicode() {
    let password = "пароль密码🔐";
    let hash = hash_password(password).unwrap();
    assert!(verify_password(password, &hash));
}

#[test]
fn test_password_hashing_long_password() {
    let password = "a".repeat(1000);
    let hash = hash_password(&password).unwrap();
    assert!(verify_password(&password, &hash));
}

#[test]
fn test_verify_password_invalid_hash() {
    assert!(!verify_password("test", "not_a_valid_hash"));
}

// ==========================================================================
// Random String Generation Tests
// ==========================================================================

#[test]
fn test_random_string() {
    let s1 = generate_random_string(16);
    let s2 = generate_random_string(16);
    assert_eq!(s1.len(), 32); // hex encoding doubles length
    assert_ne!(s1, s2);
}

#[test]
fn test_random_string_zero_length() {
    let s = generate_random_string(0);
    assert_eq!(s.len(), 0);
}

#[test]
fn test_random_string_large_length() {
    let s = generate_random_string(1000);
    assert_eq!(s.len(), 2000); // hex encoding doubles length
}

#[test]
fn test_secure_password_generation() {
    let password = generate_random_string(10);
    assert_eq!(password.len(), 20); // hex encoding: 10 bytes → 20 hex chars
    assert!(password.chars().all(|c| c.is_ascii_hexdigit()));
}

// ==========================================================================
// RSA Key Generation Tests
// ==========================================================================

#[test]
fn test_generate_rsa_key_pair() {
    let result = generate_rsa_key_pair();
    assert!(result.is_ok());

    let (private_pem, public_pem) = result.unwrap();

    assert!(private_pem.contains("-----BEGIN PRIVATE KEY-----"));
    assert!(private_pem.contains("-----END PRIVATE KEY-----"));
    assert!(public_pem.contains("-----BEGIN PUBLIC KEY-----"));
    assert!(public_pem.contains("-----END PUBLIC KEY-----"));

    let private = RsaPrivateKey::from_pkcs8_pem(&private_pem);
    assert!(private.is_ok());

    let public = RsaPublicKey::from_public_key_pem(&public_pem);
    assert!(public.is_ok());
}

// ==========================================================================
// JWT Token Tests
// ==========================================================================

#[tokio::test]
async fn test_create_and_decode_access_token() {
    let _lock = JWT_TEST_LOCK.lock().await;
    let db = create_test_db().await;
    init_jwt_keys(&db).await.expect("Failed to init JWT keys");

    let token = create_access_token(
        "user123",
        Some("user@example.com"),
        Some("openid email"),
        Some("client_id"),
        Some(3600),
        Some(vec!["apps.view".to_string()]),
        Some(vec!["sonarr".to_string()]),
    )
    .expect("Failed to create access token");

    let claims = decode_token(&token).expect("Failed to decode access token");

    assert_eq!(claims.sub, "user123");
    assert_eq!(claims.email, Some("user@example.com".to_string()));
    assert_eq!(claims.scope, Some("openid email".to_string()));
    assert_eq!(claims.client_id, Some("client_id".to_string()));
    assert_eq!(claims.permissions, Some(vec!["apps.view".to_string()]));
    assert_eq!(claims.allowed_apps, Some(vec!["sonarr".to_string()]));
    assert!(claims.token_type.is_none());
}

#[tokio::test]
async fn test_create_and_decode_refresh_token() {
    let _lock = JWT_TEST_LOCK.lock().await;
    let db = create_test_db().await;
    init_jwt_keys(&db).await.expect("Failed to init JWT keys");

    let token = create_refresh_token(
        "user123",
        Some("user@example.com"),
        Some("openid"),
        Some("client_id"),
        None,
    )
    .expect("Failed to create refresh token");

    let claims = decode_token(&token).expect("Failed to decode refresh token");

    assert_eq!(claims.sub, "user123");
    assert_eq!(claims.token_type, Some("refresh".to_string()));
    assert!(claims.permissions.is_none());
    assert!(claims.allowed_apps.is_none());
}

#[tokio::test]
async fn test_token_expiration() {
    let _lock = JWT_TEST_LOCK.lock().await;
    let db = create_test_db().await;
    init_jwt_keys(&db).await.expect("Failed to init JWT keys");

    let token = create_access_token(
        "user123",
        None,
        None,
        None,
        Some(-10), // Expired 10 seconds ago
        None,
        None,
    )
    .expect("Failed to create expired token");

    let result = decode_token(&token);
    assert!(
        result.is_err(),
        "Expected token to be expired but decode succeeded"
    );
}

#[tokio::test]
async fn test_decode_invalid_token() {
    let _lock = JWT_TEST_LOCK.lock().await;
    let db = create_test_db().await;
    init_jwt_keys(&db).await.expect("Failed to init JWT keys");

    let result = decode_token("not.a.valid.token");
    assert!(result.is_err());
}

#[tokio::test]
async fn test_decode_malformed_token() {
    let _lock = JWT_TEST_LOCK.lock().await;
    let db = create_test_db().await;
    init_jwt_keys(&db).await.expect("Failed to init JWT keys");

    let result = decode_token("completely_invalid");
    assert!(result.is_err());
}

#[tokio::test]
async fn test_access_token_minimal() {
    let _lock = JWT_TEST_LOCK.lock().await;
    let db = create_test_db().await;
    init_jwt_keys(&db).await.expect("Failed to init JWT keys");

    let token = create_access_token("user123", None, None, None, None, None, None);

    assert!(token.is_ok());
    let claims = decode_token(&token.unwrap()).unwrap();
    assert_eq!(claims.sub, "user123");
    assert!(claims.email.is_none());
    assert!(claims.scope.is_none());
}

// ==========================================================================
// TOTP Tests
// ==========================================================================

#[test]
fn test_generate_totp_secret() {
    let secret = generate_totp_secret();

    // The secret must be non-empty
    assert!(!secret.is_empty(), "TOTP secret must not be empty");

    // The secret must be a valid base32 string (uppercase letters A-Z and digits 2-7)
    assert!(
        secret
            .chars()
            .all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c) || c == '='),
        "TOTP secret must be a valid base32-encoded string, got: {}",
        secret
    );
}

#[test]
fn test_get_totp_provisioning_uri() {
    let secret = generate_totp_secret();
    let uri = get_totp_provisioning_uri(&secret, "testuser@example.com")
        .expect("get_totp_provisioning_uri must not fail with a valid secret");

    assert!(
        uri.starts_with("otpauth://totp/"),
        "Provisioning URI must use the otpauth://totp/ scheme, got: {}",
        uri
    );
    assert!(
        uri.contains("Kubarr"),
        "Provisioning URI must contain the 'Kubarr' issuer, got: {}",
        uri
    );
    assert!(
        uri.contains("testuser"),
        "Provisioning URI must contain the account name, got: {}",
        uri
    );
}

#[test]
fn test_totp_invalid_code() {
    let secret = generate_totp_secret();

    // "000000" is almost certainly not the current valid TOTP code
    let result = verify_totp(&secret, "000000", "testuser@example.com")
        .expect("verify_totp must not return an Err for a structurally valid call");

    // This assertion could theoretically fail once in 1,000,000 runs if "000000"
    // happens to be the current code — acceptable in practice.
    assert!(
        !result,
        "verify_totp with a clearly wrong code must return false"
    );
}

// ==========================================================================
// Recovery Code Tests
// ==========================================================================

#[test]
fn test_generate_recovery_codes() {
    let codes = generate_recovery_codes();

    assert_eq!(
        codes.len(),
        8,
        "generate_recovery_codes must return exactly 8 codes"
    );

    for code in &codes {
        assert_eq!(
            code.len(),
            10,
            "Each recovery code must be exactly 10 characters long, got: {}",
            code
        );
        assert!(
            code.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
            "Each recovery code must be alphanumeric uppercase, got: {}",
            code
        );
    }
}

#[test]
fn test_recovery_code_uniqueness() {
    let codes = generate_recovery_codes();
    let unique: std::collections::HashSet<&String> = codes.iter().collect();

    assert_eq!(
        unique.len(),
        codes.len(),
        "All 8 recovery codes must be unique"
    );
}

#[test]
fn test_hash_and_verify_recovery_code() {
    let code = "ABCDE12345";

    let hash = hash_recovery_code(code).expect("hash_recovery_code must not fail");

    assert!(
        verify_recovery_code(code, &hash),
        "verify_recovery_code must return true for the correct code"
    );
    assert!(
        !verify_recovery_code("WRONGCODE0", &hash),
        "verify_recovery_code must return false for a wrong code"
    );
}

// ==========================================================================
// Session Token Tests
// ==========================================================================

#[tokio::test]
async fn test_session_token_roundtrip() {
    let _lock = JWT_TEST_LOCK.lock().await;
    let db = create_test_db().await;
    init_jwt_keys(&db).await.expect("Failed to init JWT keys");

    let session_id = "sess-abc-123";

    let token = create_session_token(session_id).expect("create_session_token must not fail");

    let claims =
        decode_session_token(&token).expect("decode_session_token must succeed for a fresh token");

    assert_eq!(
        claims.sid, session_id,
        "Decoded session ID must match the original"
    );
}

// ==========================================================================
// JWKS Tests
// ==========================================================================

#[tokio::test]
async fn test_get_jwks_structure() {
    let _lock = JWT_TEST_LOCK.lock().await;
    let db = create_test_db().await;
    init_jwt_keys(&db).await.expect("Failed to init JWT keys");

    let jwks = get_jwks().expect("get_jwks must not fail after JWT keys are initialized");

    // Must have a "keys" array
    let keys = jwks
        .get("keys")
        .and_then(|v| v.as_array())
        .expect("JWKS must contain a 'keys' array");

    assert!(!keys.is_empty(), "JWKS 'keys' array must not be empty");

    let first_key = &keys[0];

    assert_eq!(
        first_key.get("kty").and_then(|v| v.as_str()),
        Some("RSA"),
        "First key 'kty' must be 'RSA'"
    );
    assert_eq!(
        first_key.get("alg").and_then(|v| v.as_str()),
        Some("RS256"),
        "First key 'alg' must be 'RS256'"
    );
    assert!(
        first_key.get("n").and_then(|v| v.as_str()).is_some(),
        "First key must have a modulus 'n'"
    );
    assert!(
        first_key.get("e").and_then(|v| v.as_str()).is_some(),
        "First key must have an exponent 'e'"
    );
}
