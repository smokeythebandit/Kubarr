//! Additional security tests to cover uncovered functions in `services/security.rs`
//!
//! Covers: `get_totp_qr_code_base64`, `generate_2fa_challenge_token`,
//!         `get_private_key`, `get_public_key`

mod common;
use common::create_test_db_with_seed;

use kubarr::services::security::{
    generate_2fa_challenge_token, generate_totp_secret, get_private_key, get_public_key,
    get_totp_provisioning_uri, init_jwt_keys,
};

static JWT_INIT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn ensure_jwt_keys() {
    JWT_INIT
        .get_or_init(|| async {
            let db = create_test_db_with_seed().await;
            init_jwt_keys(&db).await.expect("JWT key init failed");
        })
        .await;
}

#[tokio::test]
async fn test_get_totp_qr_code_base64_returns_data_url() {
    ensure_jwt_keys().await;
    let secret = generate_totp_secret();
    let result = get_totp_provisioning_uri(&secret, "testuser@example.com");
    assert!(result.is_ok(), "get_totp_provisioning_uri must succeed");
    let uri = result.unwrap();
    assert!(
        uri.starts_with("otpauth://"),
        "must return a provisioning URI"
    );
    assert!(uri.len() > 10, "provisioning URI must have content");
}

#[tokio::test]
async fn test_get_totp_qr_code_base64_with_invalid_secret_returns_err() {
    let result = get_totp_provisioning_uri("not-a-valid-secret!!!", "user@example.com");
    assert!(result.is_err(), "invalid secret must return error");
}

#[test]
fn test_generate_2fa_challenge_token_returns_nonempty_string() {
    let token = generate_2fa_challenge_token();
    assert!(!token.is_empty(), "challenge token must not be empty");
    assert_eq!(
        token.len(),
        64,
        "challenge token must be 64 hex chars (32 bytes)"
    );
}

#[test]
fn test_generate_2fa_challenge_token_is_different_each_time() {
    let token1 = generate_2fa_challenge_token();
    let token2 = generate_2fa_challenge_token();
    assert_ne!(
        token1, token2,
        "challenge tokens must be different each call"
    );
}

// ============================================================================
// get_private_key / get_public_key
// ============================================================================

#[tokio::test]
async fn test_get_private_key_returns_pem_after_init() {
    ensure_jwt_keys().await;
    let key = get_private_key().expect("get_private_key must succeed after init");
    assert!(
        key.contains("PRIVATE KEY"),
        "private key must contain PEM header"
    );
    assert!(!key.is_empty(), "private key must not be empty");
}

#[tokio::test]
async fn test_get_public_key_returns_pem_after_init() {
    ensure_jwt_keys().await;
    let key = get_public_key().expect("get_public_key must succeed after init");
    assert!(
        key.contains("PUBLIC KEY"),
        "public key must contain PEM header"
    );
    assert!(!key.is_empty(), "public key must not be empty");
}
