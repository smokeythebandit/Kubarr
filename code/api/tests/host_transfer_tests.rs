//! Exercise the public transfer endpoints as the gateway calls them: /auth/
//! is proxied to the API, while / on a host assignment routes to the app.
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use chrono::Utc;
use kubarr::{
    endpoints::create_router,
    models::{app_domain_assignment, domain, prelude::*, session},
    services::{create_session_token, init_jwt_keys},
    state::AppState,
};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use tower::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

static JWT_INIT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn setup() -> (AppState, DatabaseConnection, String) {
    JWT_INIT
        .get_or_init(|| async {
            let db = create_test_db_with_seed().await;
            init_jwt_keys(&db).await.unwrap();
        })
        .await;
    let db = create_test_db_with_seed().await;
    let user = create_test_user_with_role(
        &db,
        "transfer-user",
        "transfer@example.test",
        "password",
        "admin",
    )
    .await;
    let sid = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    session::ActiveModel {
        id: Set(sid.clone()),
        user_id: Set(user.id),
        user_agent: Set(None),
        ip_address: Set(None),
        created_at: Set(now),
        last_accessed_at: Set(now),
        expires_at: Set(now + chrono::Duration::hours(1)),
        is_revoked: Set(false),
    }
    .insert(&db)
    .await
    .unwrap();
    let cookie = format!("kubarr_session={}", create_session_token(&sid).unwrap());
    (build_test_app_state_with_db(db.clone()).await, db, cookie)
}

async fn assign(db: &DatabaseConnection, app: &str, mode: &str, hostname: Option<&str>) {
    let now = Utc::now();
    let domain = domain::ActiveModel {
        domain: Set("example.test".into()),
        kind: Set("custom".into()),
        scope: Set("public".into()),
        primary: Set(true),
        enabled: Set(true),
        dns_mode: Set("manual".into()),
        ddns_profile_id: Set(None),
        dns_status: Set("unknown".into()),
        tls_mode: Set("none".into()),
        letsencrypt_profile_id: Set(None),
        tls_secret_name: Set(None),
        certificate_status: Set("unknown".into()),
        certificate_expires_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
    app_domain_assignment::ActiveModel {
        app_name: Set(app.into()),
        domain_id: Set(domain.id),
        route_mode: Set(mode.into()),
        hostname: Set(hostname.map(str::to_owned)),
        path_prefix: Set((mode == "path").then(|| format!("/{app}"))),
        primary: Set(true),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
}

async fn start(state: AppState, host: &str, cookie: &str) -> axum::response::Response {
    create_router(state)
        .oneshot(
            Request::builder()
                .uri("/auth/host-transfer/start?app=plex")
                .header(header::HOST, host)
                .header(header::COOKIE, cookie)
                .header("x-forwarded-proto", "https")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn path_assignment_stays_on_the_app_path_even_on_the_same_host() {
    let (state, db, cookie) = setup().await;
    assign(&db, "plex", "path", None).await;
    let response = start(state, "example.test:18080", &cookie).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], "/plex/");
    assert_eq!(
        Session::find().all(&db).await.unwrap().len(),
        1,
        "path routing needs no ticket"
    );
}

#[tokio::test]
async fn same_host_assignment_lands_at_root_without_creating_a_ticket() {
    let (state, db, cookie) = setup().await;
    assign(&db, "plex", "exact_host", Some("example.test")).await;
    let response = start(state, "example.test:18080", &cookie).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], "/");
    assert_eq!(Session::find().all(&db).await.unwrap().len(), 1);
}

#[tokio::test]
async fn external_host_drops_source_port_and_ticket_is_host_bound_and_single_use() {
    let (state, db, cookie) = setup().await;
    assign(&db, "plex", "exact_host", Some("plex.example.test")).await;
    let response = start(state.clone(), "example.test:18080", &cookie).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers()[header::LOCATION].to_str().unwrap();
    assert!(
        location.starts_with("https://plex.example.test/auth/host-transfer/complete?ticket="),
        "{location}"
    );
    assert!(!location.contains(":18080"));
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let uri = location.split("plex.example.test").nth(1).unwrap();
    let complete = |host| {
        Request::builder()
            .uri(uri)
            .header(header::HOST, host)
            .header("x-forwarded-proto", "https")
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        create_router(state.clone())
            .oneshot(complete("wrong.example.test"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let good = create_router(state.clone())
        .oneshot(complete("plex.example.test"))
        .await
        .unwrap();
    assert_eq!(good.status(), StatusCode::SEE_OTHER);
    assert_eq!(good.headers()[header::LOCATION], "/");
    assert!(good
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .any(|v| v.to_str().unwrap().starts_with("kubarr_session_0=")));
    assert_eq!(
        create_router(state)
            .oneshot(complete("plex.example.test"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn local_host_alias_keeps_shared_gateway_port() {
    let (state, db, cookie) = setup().await;
    assign(&db, "plex", "exact_host", Some("plex.localhost")).await;
    let response = start(state, "localhost:18080", &cookie).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .starts_with("https://plex.localhost:18080/auth/host-transfer/complete?ticket="));
}

#[tokio::test]
async fn removed_assignment_cannot_redeem_an_issued_ticket() {
    let (state, db, cookie) = setup().await;
    assign(&db, "plex", "exact_host", Some("plex.example.test")).await;
    let response = start(state.clone(), "example.test", &cookie).await;
    let uri = response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .split("plex.example.test")
        .nth(1)
        .unwrap()
        .to_string();
    let assignment = AppDomainAssignment::find().one(&db).await.unwrap().unwrap();
    let mut assignment: app_domain_assignment::ActiveModel = assignment.into();
    assignment.enabled = Set(false);
    assignment.update(&db).await.unwrap();
    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(header::HOST, "plex.example.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .next()
        .is_none());
}
