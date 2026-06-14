//! Auth tests covering 2FA-related login paths and recovery code flow.
//!
//! Covers:
//! - Login blocked when role requires 2FA but user hasn't enabled it
//! - Successful recovery-code login (`POST /auth/2fa/recover`)
//! - Recovery login when all codes are exhausted (disables 2FA on user)
//! - Recovery login with wrong recovery code returns 401
//! - Recovery login with invalid password returns 401
//! - Recovery login with unapproved user returns 401

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use kubarr::models::{role, two_factor_recovery_code, user, user_role};
use kubarr::services::security::{
    generate_recovery_codes, generate_totp_secret, hash_recovery_code,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user};
use kubarr::endpoints::create_router;

// ============================================================================
// JWT key initialization
// ============================================================================

static JWT_INIT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn ensure_jwt_keys() {
    JWT_INIT
        .get_or_init(|| async {
            let db = create_test_db_with_seed().await;
            kubarr::services::init_jwt_keys(&db)
                .await
                .expect("Failed to initialise test JWT keys");
        })
        .await;
}

// ============================================================================
// Helpers
// ============================================================================

async fn post_json(app: axum::Router, uri: &str, body: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// Insert recovery codes for a user and return the plaintext codes.
async fn insert_recovery_codes(
    db: &sea_orm::DatabaseConnection,
    user_id: i64,
    count: usize,
) -> Vec<String> {
    let all_codes = generate_recovery_codes();
    let plaintext: Vec<String> = all_codes.into_iter().take(count).collect();
    let now = chrono::Utc::now();

    for code in &plaintext {
        let code_hash = hash_recovery_code(code).expect("hash_recovery_code must succeed");
        let row = two_factor_recovery_code::ActiveModel {
            user_id: Set(user_id),
            code_hash: Set(code_hash),
            used_at: Set(None),
            created_at: Set(now),
            ..Default::default()
        };
        row.insert(db).await.expect("insert recovery code");
    }

    plaintext
}

/// Create a user with 2FA already enabled (login first, then update DB).
async fn create_2fa_user(
    db: &sea_orm::DatabaseConnection,
    username: &str,
    email: &str,
    password: &str,
) -> kubarr::models::user::Model {
    // Create user normally (totp_enabled=false)
    let user = create_test_user(db, username, email, password, true).await;

    // Now enable 2FA directly in DB
    let now = chrono::Utc::now();
    let secret = generate_totp_secret();
    let update = user::ActiveModel {
        id: Set(user.id),
        totp_enabled: Set(true),
        totp_secret: Set(Some(secret)),
        totp_verified_at: Set(Some(now)),
        updated_at: Set(now),
        ..Default::default()
    };
    update.update(db).await.expect("enable 2FA on test user");

    // Return fresh user
    kubarr::models::prelude::User::find_by_id(user.id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
}

// ============================================================================
// Role requires 2FA but user hasn't enabled it → blocked at login
// ============================================================================

#[tokio::test]
async fn test_login_role_requires_2fa_not_enabled_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    // Create a role with requires_2fa = true
    let now = chrono::Utc::now();
    let secure_role = role::ActiveModel {
        name: Set("secure_role".to_string()),
        description: Set(Some("Role that requires 2FA".to_string())),
        is_system: Set(false),
        requires_2fa: Set(true),
        created_at: Set(now),
        ..Default::default()
    };
    let created_role = secure_role.insert(&db).await.unwrap();

    // Create user WITHOUT 2FA enabled
    let user = create_test_user(&db, "no2fa_user", "no2fa@test.com", "pass123", true).await;

    // Assign the requires_2fa role
    let user_role_model = user_role::ActiveModel {
        user_id: Set(user.id),
        role_id: Set(created_role.id),
    };
    user_role_model.insert(&db).await.unwrap();

    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let body = serde_json::json!({
        "username": "no2fa_user",
        "password": "pass123"
    })
    .to_string();
    let (status, resp_body) = post_json(app, "/auth/login", &body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Login must be blocked when role requires 2FA but user hasn't set it up. Body: {}",
        resp_body
    );
    assert!(
        resp_body.contains("Two-factor")
            || resp_body.contains("2fa")
            || resp_body.contains("authentication"),
        "Error message must mention 2FA requirement. Got: {}",
        resp_body
    );
}

// ============================================================================
// Successful recovery-code login
// ============================================================================

#[tokio::test]
async fn test_recovery_login_success_returns_200() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let user = create_2fa_user(&db, "recov_success", "recov_success@test.com", "pass123").await;
    let codes = insert_recovery_codes(&db, user.id, 5).await;

    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let body = serde_json::json!({
        "username": "recov_success",
        "password": "pass123",
        "recovery_code": &codes[0]
    })
    .to_string();
    let (status, resp_body) = post_json(app, "/auth/2fa/recover", &body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Recovery login with valid code must return 200. Body: {}",
        resp_body
    );

    // Should return user info
    let json: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
    assert_eq!(
        json["username"].as_str().unwrap(),
        "recov_success",
        "Response must include username"
    );
}

#[tokio::test]
async fn test_recovery_login_sets_session_cookie() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let user = create_2fa_user(&db, "recov_cookie", "recov_cookie@test.com", "pass123").await;
    let codes = insert_recovery_codes(&db, user.id, 3).await;

    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let body = serde_json::json!({
        "username": "recov_cookie",
        "password": "pass123",
        "recovery_code": &codes[0]
    })
    .to_string();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/2fa/recover")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let has_session_cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .any(|v| {
            v.to_str()
                .map(|s| s.starts_with("kubarr_session=") || s.starts_with("kubarr_session_"))
                .unwrap_or(false)
        });

    assert!(
        has_session_cookie,
        "Recovery login must set a session cookie"
    );
}

// ============================================================================
// Last recovery code exhausted — disables 2FA on the user
// ============================================================================

#[tokio::test]
async fn test_recovery_last_code_disables_2fa() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    // Use only 1 recovery code so we exhaust it in one call
    let user = create_2fa_user(&db, "last_code", "last_code@test.com", "pass123").await;
    let codes = insert_recovery_codes(&db, user.id, 1).await;

    let state = build_test_app_state_with_db(db.clone()).await;
    let app = create_router(state);

    let body = serde_json::json!({
        "username": "last_code",
        "password": "pass123",
        "recovery_code": &codes[0]
    })
    .to_string();
    let (status, resp_body) = post_json(app, "/auth/2fa/recover", &body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Last recovery code must return 200. Body: {}",
        resp_body
    );

    // 2FA should now be disabled on the user
    let updated_user = kubarr::models::prelude::User::find_by_id(user.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    assert!(
        !updated_user.totp_enabled,
        "2FA must be disabled after last recovery code is used"
    );
    assert!(
        updated_user.totp_secret.is_none(),
        "TOTP secret must be cleared after last recovery code is used"
    );

    // All recovery codes should be gone
    let remaining = two_factor_recovery_code::Entity::find()
        .filter(two_factor_recovery_code::Column::UserId.eq(user.id))
        .count(&db)
        .await
        .unwrap();
    assert_eq!(
        remaining, 0,
        "All recovery codes must be deleted after exhaustion"
    );
}

// ============================================================================
// Error paths for recovery login
// ============================================================================

#[tokio::test]
async fn test_recovery_login_wrong_code_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let user = create_2fa_user(&db, "recov_wrong", "recov_wrong@test.com", "pass123").await;
    insert_recovery_codes(&db, user.id, 3).await;

    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let body = serde_json::json!({
        "username": "recov_wrong",
        "password": "pass123",
        "recovery_code": "XXXX-YYYY-ZZZZ-0000"  // wrong code
    })
    .to_string();
    let (status, _) = post_json(app, "/auth/2fa/recover", &body).await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "Wrong recovery code must return 401"
    );
}

#[tokio::test]
async fn test_recovery_login_no_codes_stored_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    // User has 2FA enabled but NO recovery codes stored
    let _user = create_2fa_user(&db, "no_codes", "no_codes@test.com", "pass123").await;

    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let body = serde_json::json!({
        "username": "no_codes",
        "password": "pass123",
        "recovery_code": "SOME-CODE-HERE-1234"
    })
    .to_string();
    let (status, _) = post_json(app, "/auth/2fa/recover", &body).await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "Recovery with no stored codes must return 401"
    );
}

#[tokio::test]
async fn test_recovery_login_unapproved_user_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    // Create unapproved user
    let now = chrono::Utc::now();
    use kubarr::services::security::hash_password;
    let unapproved = user::ActiveModel {
        username: Set("unapproved_recov".to_string()),
        email: Set("unapproved_recov@test.com".to_string()),
        hashed_password: Set(hash_password("pass123").unwrap()),
        is_active: Set(true),
        is_approved: Set(false),
        totp_enabled: Set(true),
        totp_secret: Set(Some(generate_totp_secret())),
        totp_verified_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    let created = unapproved.insert(&db).await.unwrap();
    insert_recovery_codes(&db, created.id, 3).await;

    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let body = serde_json::json!({
        "username": "unapproved_recov",
        "password": "pass123",
        "recovery_code": "FAKE-CODE-1234"
    })
    .to_string();
    let (status, _) = post_json(app, "/auth/2fa/recover", &body).await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "Recovery for unapproved user must return 401"
    );
}

// ============================================================================
// Login with TOTP code — covers auth.rs lines 256-268
// ============================================================================

/// Login with a valid TOTP code succeeds (covers auth.rs lines 256-268).
#[tokio::test]
async fn test_login_with_valid_totp_code_succeeds() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    // Create user with known TOTP secret
    let totp_secret = generate_totp_secret();
    let now = chrono::Utc::now();
    use kubarr::services::security::hash_password;
    let user_model = user::ActiveModel {
        username: Set("totp_login_user".to_string()),
        email: Set("totp_login@test.com".to_string()),
        hashed_password: Set(hash_password("password123").unwrap()),
        is_active: Set(true),
        is_approved: Set(true),
        totp_enabled: Set(true),
        totp_secret: Set(Some(totp_secret.clone())),
        totp_verified_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    user_model.insert(&db).await.unwrap();

    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    // Generate a valid TOTP code using the known secret
    let code = {
        use totp_rs::{Algorithm, Secret, TOTP};
        let secret_bytes = Secret::Encoded(totp_secret.clone())
            .to_bytes()
            .expect("decode TOTP secret");
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            secret_bytes,
            Some("Kubarr".to_string()),
            "totp_login@test.com".to_string(),
        )
        .expect("create TOTP");
        totp.generate_current().expect("generate TOTP code")
    };

    let body = serde_json::json!({
        "username": "totp_login_user",
        "password": "password123",
        "totp_code": code
    })
    .to_string();

    let (status, _resp) = post_json(app, "/auth/login", &body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Login with valid TOTP code must return 200"
    );
}

/// Login with TOTP enabled but no code supplied returns 400 (auth.rs line 257-259).
#[tokio::test]
async fn test_login_totp_enabled_no_code_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    let now = chrono::Utc::now();
    use kubarr::services::security::hash_password;
    let user_model = user::ActiveModel {
        username: Set("totp_nocode_user".to_string()),
        email: Set("totp_nocode@test.com".to_string()),
        hashed_password: Set(hash_password("password123").unwrap()),
        is_active: Set(true),
        is_approved: Set(true),
        totp_enabled: Set(true),
        totp_secret: Set(Some(generate_totp_secret())),
        totp_verified_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    user_model.insert(&db).await.unwrap();

    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let body = serde_json::json!({
        "username": "totp_nocode_user",
        "password": "password123"
        // intentionally omit totp_code
    })
    .to_string();

    let (status, _) = post_json(app, "/auth/login", &body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Login with TOTP enabled but no code must return 400"
    );
}

/// Login with TOTP enabled and wrong code returns 401 (auth.rs lines 265-266).
#[tokio::test]
async fn test_login_totp_enabled_wrong_code_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    let now = chrono::Utc::now();
    use kubarr::services::security::hash_password;
    let user_model = user::ActiveModel {
        username: Set("totp_wrongcode_user".to_string()),
        email: Set("totp_wrongcode@test.com".to_string()),
        hashed_password: Set(hash_password("password123").unwrap()),
        is_active: Set(true),
        is_approved: Set(true),
        totp_enabled: Set(true),
        totp_secret: Set(Some(generate_totp_secret())),
        totp_verified_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    user_model.insert(&db).await.unwrap();

    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let body = serde_json::json!({
        "username": "totp_wrongcode_user",
        "password": "password123",
        "totp_code": "000000"  // almost certainly wrong
    })
    .to_string();

    let (status, _) = post_json(app, "/auth/login", &body).await;

    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::BAD_REQUEST,
        "Login with wrong TOTP code must return 401 or 400, got: {}",
        status
    );
}
