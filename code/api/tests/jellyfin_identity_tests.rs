//! Kubarr as Jellyfin's identity provider: the plugin's service endpoints,
//! app passwords, and the gateway decisions for an app-native app.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use std::collections::HashMap;
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state, create_test_user_with_role};

use kubarr::endpoints::create_router;
use kubarr::services::catalog::{AppCatalog, GatewayTraits};
use kubarr::services::managed_key::JELLYFIN_IDENTITY;
use kubarr::state::AppState;

static JWT_INIT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn ensure_jwt_keys() {
    JWT_INIT
        .get_or_init(|| async {
            let db = common::create_test_db_with_seed().await;
            kubarr::services::init_jwt_keys(&db).await.unwrap();
        })
        .await;
}

async fn state() -> AppState {
    ensure_jwt_keys().await;
    build_test_app_state().await
}

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<serde_json::Value>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let mut builder = Request::builder()
        .uri(uri)
        .method(method)
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let body = body
        .map(|b| Body::from(b.to_string()))
        .unwrap_or_else(Body::empty);
    let response = create_router(state.clone())
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&bytes).to_string())
}

async fn login(state: &AppState, username: &str, password: &str) -> String {
    let (status, headers, body) = send(
        state,
        "POST",
        "/auth/login",
        &[],
        Some(serde_json::json!({"username": username, "password": password})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    headers
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|s| s.starts_with("kubarr_session_0=") && !s.contains("Max-Age=0"))
        .map(|s| s.split(';').next().unwrap().to_string())
        .expect("session cookie")
}

async fn identity_key(state: &AppState) -> String {
    let db = state.get_db().await.unwrap();
    JELLYFIN_IDENTITY.api_key(&db).await.unwrap()
}

async fn verify(
    state: &AppState,
    key: &str,
    username: &str,
    password: &str,
) -> (StatusCode, String) {
    let bearer = format!("Bearer {key}");
    let (status, _, body) = send(
        state,
        "POST",
        "/api/integrations/jellyfin/verify",
        &[("authorization", &bearer)],
        Some(serde_json::json!({"username": username, "password": password})),
    )
    .await;
    (status, body)
}

#[tokio::test]
async fn verify_requires_the_identity_key_not_a_session_or_another_key() {
    let state = state().await;
    create_test_user_with_role(
        &state.get_db().await.unwrap(),
        "viewer1",
        "viewer1@example.com",
        "pw-viewer-1",
        "viewer",
    )
    .await;

    let (status, _) = verify(&state, "0".repeat(32).as_str(), "viewer1", "pw-viewer-1").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The Jellyfin library key is held by Sonarr and Radarr too; it must not
    // open the password oracle.
    let db = state.get_db().await.unwrap();
    let library_key = kubarr::services::managed_key::JELLYFIN
        .api_key(&db)
        .await
        .unwrap();
    let (status, _) = verify(&state, &library_key, "viewer1", "pw-viewer-1").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let cookie = login(&state, "viewer1", "pw-viewer-1").await;
    let (status, _, _) = send(
        &state,
        "POST",
        "/api/integrations/jellyfin/verify",
        &[("cookie", &cookie)],
        Some(serde_json::json!({"username": "viewer1", "password": "pw-viewer-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn verify_checks_password_access_and_maps_admin() {
    let state = state().await;
    let db = state.get_db().await.unwrap();
    let viewer =
        create_test_user_with_role(&db, "viewer2", "v2@example.com", "pw-v2", "viewer").await;
    create_test_user_with_role(&db, "admin2", "a2@example.com", "pw-a2", "admin").await;
    create_test_user_with_role(&db, "dl2", "d2@example.com", "pw-d2", "downloader").await;
    let key = identity_key(&state).await;

    let (status, body) = verify(&state, &key, "viewer2", "pw-v2").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let user: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(user["user_id"], viewer.id.to_string());
    assert_eq!(user["is_admin"], false);

    let (status, body) = verify(&state, &key, "a2@example.com", "pw-a2").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"is_admin\":true"), "{body}");

    let (status, _) = verify(&state, &key, "viewer2", "wrong").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = verify(&state, &key, "nobody", "pw").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = verify(&state, &key, "dl2", "pw-d2").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn verify_throttles_repeated_failures() {
    let state = state().await;
    let db = state.get_db().await.unwrap();
    create_test_user_with_role(&db, "victim", "victim@example.com", "pw-victim", "viewer").await;
    let key = identity_key(&state).await;

    for _ in 0..5 {
        let (status, _) = verify(&state, &key, "victim", "guess").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    // Even the right password is refused until the window passes.
    let (status, _) = verify(&state, &key, "victim", "pw-victim").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn two_factor_users_need_an_app_password() {
    use kubarr::models::user;
    use sea_orm::{ActiveModelTrait, Set};

    let state = state().await;
    let db = state.get_db().await.unwrap();
    let alice = create_test_user_with_role(
        &db,
        "alice2fa",
        "alice2fa@example.com",
        "pw-alice",
        "viewer",
    )
    .await;
    let key = identity_key(&state).await;

    // Create an app password while signed in (before TOTP is switched on, so
    // the session login needs no code).
    let cookie = login(&state, "alice2fa", "pw-alice").await;
    let (status, _, body) = send(
        &state,
        "POST",
        "/api/users/me/app-passwords",
        &[("cookie", &cookie)],
        Some(serde_json::json!({"name": "Living room TV"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let created: serde_json::Value = serde_json::from_str(&body).unwrap();
    let app_password = created["password"].as_str().unwrap().to_string();
    let id = created["id"].as_i64().unwrap();

    let mut active: user::ActiveModel = alice.into();
    active.totp_enabled = Set(true);
    active.totp_secret = Set(Some("JBSWY3DPEHPK3PXP".into()));
    active.update(&db).await.unwrap();

    let (status, _) = verify(&state, &key, "alice2fa", "pw-alice").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "account password must not bypass 2FA"
    );
    let (status, body) = verify(&state, &key, "alice2fa", &app_password).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Listing never reveals the password; deleting revokes it.
    let (_, _, listed) = send(
        &state,
        "GET",
        "/api/users/me/app-passwords",
        &[("cookie", &cookie)],
        None,
    )
    .await;
    assert!(listed.contains("Living room TV"));
    assert!(!listed.contains(&app_password));
    assert!(!listed.contains("token_hash"));
    let (status, _, _) = send(
        &state,
        "DELETE",
        &format!("/api/users/me/app-passwords/{id}"),
        &[("cookie", &cookie)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = verify(&state, &key, "alice2fa", &app_password).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn snapshot_lists_jellyfin_users_for_the_plugin_only() {
    let state = state().await;
    let db = state.get_db().await.unwrap();
    create_test_user_with_role(&db, "snapv", "snapv@example.com", "pw", "viewer").await;
    create_test_user_with_role(&db, "snapd", "snapd@example.com", "pw", "downloader").await;
    let key = identity_key(&state).await;
    let bearer = format!("Bearer {key}");

    let (status, _, body) = send(
        &state,
        "GET",
        "/api/integrations/jellyfin/users",
        &[("authorization", &bearer)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"snapv\""), "{body}");
    assert!(!body.contains("\"snapd\""), "{body}");

    let (status, _, _) = send(&state, "GET", "/api/integrations/jellyfin/users", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

async fn with_jellyfin_traits(state: &AppState, traits: GatewayTraits) {
    *state.catalog.write().await =
        AppCatalog::with_apps(HashMap::new()).with_gateway_traits("jellyfin", traits);
    state
        .endpoint_cache
        .set(
            "jellyfin",
            "http://jellyfin.jellyfin.svc.cluster.local:8096".to_string(),
            Some(String::new()),
            None,
        )
        .await;
}

#[tokio::test]
async fn gateway_lets_anonymous_clients_reach_only_app_native_apps() {
    let state = state().await;

    with_jellyfin_traits(&state, GatewayTraits::default()).await;
    let (status, _, _) = send(&state, "GET", "/api/proxy-auth/jellyfin", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    with_jellyfin_traits(
        &state,
        GatewayTraits {
            identity_header: true,
            app_native_auth: true,
        },
    )
    .await;
    let (status, headers, _) = send(&state, "GET", "/api/proxy-auth/jellyfin", &[], None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        headers.get("x-kubarr-identity").is_none(),
        "anonymous gets no identity"
    );
    assert!(headers.get("x-kubarr-upstream").is_some());

    // Other apps keep requiring a session.
    let (status, _, _) = send(&state, "GET", "/api/proxy-auth/sonarr", &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn gateway_hands_signed_in_users_an_identity_and_refuses_the_unauthorised() {
    use jsonwebtoken::{decode, DecodingKey, Validation};
    use kubarr::services::app_identity::IdentityClaims;

    let state = state().await;
    let db = state.get_db().await.unwrap();
    let viewer =
        create_test_user_with_role(&db, "gwv", "gwv@example.com", "pw-gwv", "viewer").await;
    create_test_user_with_role(&db, "gwd", "gwd@example.com", "pw-gwd", "downloader").await;
    with_jellyfin_traits(
        &state,
        GatewayTraits {
            identity_header: true,
            app_native_auth: true,
        },
    )
    .await;

    let cookie = login(&state, "gwv", "pw-gwv").await;
    let (status, headers, _) = send(
        &state,
        "GET",
        "/api/proxy-auth/jellyfin",
        &[("cookie", &cookie)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let token = headers["x-kubarr-identity"].to_str().unwrap();

    let jwks: serde_json::Value = {
        let (status, _, body) = send(&state, "GET", "/auth/jwks", &[], None).await;
        assert_eq!(status, StatusCode::OK);
        serde_json::from_str(&body).unwrap()
    };
    let key = &jwks["keys"][0];
    let decoding =
        DecodingKey::from_rsa_components(key["n"].as_str().unwrap(), key["e"].as_str().unwrap())
            .unwrap();
    let mut validation = Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&["jellyfin"]);
    validation.set_issuer(&["kubarr"]);
    let claims = decode::<IdentityClaims>(token, &decoding, &validation)
        .unwrap()
        .claims;
    assert_eq!(claims.sub, viewer.id.to_string());
    assert_eq!(claims.preferred_username, "gwv");
    assert!(!claims.is_admin);

    // A signed-in user without Jellyfin access is refused, not passed through.
    let cookie = login(&state, "gwd", "pw-gwd").await;
    let (status, _, _) = send(
        &state,
        "GET",
        "/api/proxy-auth/jellyfin",
        &[("cookie", &cookie)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
