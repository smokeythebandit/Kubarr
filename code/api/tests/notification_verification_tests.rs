//! Destination verification for user notification preferences, and the
//! enum-derived event list.
//!
//! External sends go through a capturing provider installed with
//! `NotificationService::set_provider_override`, so no network is touched.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

use kubarr::endpoints::create_router;
use kubarr::models::audit_log::AuditAction;
use kubarr::models::{audit_log, notification_event, notification_verification};
use kubarr::services::notification::{
    ChannelType, NotificationMessage, NotificationProvider, SendResult,
};
use kubarr::state::AppState;

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
// Capturing provider
// ============================================================================

#[derive(Clone)]
struct Capture {
    channel: ChannelType,
    sent: Arc<Mutex<Vec<NotificationMessage>>>,
    fail: bool,
}

impl Capture {
    fn new(channel: ChannelType, fail: bool) -> Self {
        Self {
            channel,
            sent: Arc::new(Mutex::new(Vec::new())),
            fail,
        }
    }

    fn messages(&self) -> Vec<NotificationMessage> {
        self.sent.lock().unwrap().clone()
    }

    fn last_code(&self) -> String {
        let messages = self.messages();
        let body = &messages.last().expect("a message was sent").body;
        body.rsplit(' ').next().unwrap().to_string()
    }
}

#[async_trait]
impl NotificationProvider for Capture {
    fn channel_type(&self) -> ChannelType {
        self.channel
    }

    async fn send(&self, message: &NotificationMessage) -> SendResult {
        self.sent.lock().unwrap().push(message.clone());
        if self.fail {
            SendResult {
                success: false,
                error: Some("stub failure".to_string()),
            }
        } else {
            SendResult {
                success: true,
                error: None,
            }
        }
    }

    async fn test(&self, destination: &str) -> SendResult {
        self.send(&NotificationMessage {
            recipient: destination.to_string(),
            title: "test".to_string(),
            body: "test".to_string(),
            severity: kubarr::services::notification::NotificationSeverity::Info,
        })
        .await
    }
}

// ============================================================================
// Helpers
// ============================================================================

struct Ctx {
    state: AppState,
    db: sea_orm::DatabaseConnection,
    cookie: String,
    user_id: i64,
    capture: Capture,
}

async fn setup_with(fail: bool) -> Ctx {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;
    let user =
        create_test_user_with_role(&db, "verifier", "v@example.com", "password123", "viewer").await;
    let state = build_test_app_state_with_db(db.clone()).await;
    let capture = Capture::new(ChannelType::Telegram, fail);
    state
        .notification
        .set_provider_override(Arc::new(capture.clone()))
        .await;

    let body = serde_json::json!({"username": "verifier", "password": "password123"}).to_string();
    let response = create_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/auth/login")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            (s.starts_with("kubarr_session=") && !s.contains("kubarr_session_"))
                .then(|| s.split(';').next().unwrap().to_string())
        })
        .expect("login sets a session cookie");

    Ctx {
        state,
        db,
        cookie,
        user_id: user.id,
        capture,
    }
}

async fn setup() -> Ctx {
    setup_with(false).await
}

async fn call(
    ctx: &Ctx,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .uri(uri)
        .method(method)
        .header("Cookie", &ctx.cookie);
    let body = match body {
        Some(json) => {
            builder = builder.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = create_router(ctx.state.clone())
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn set_destination(ctx: &Ctx, destination: &str) -> serde_json::Value {
    let (status, json) = call(
        ctx,
        "PUT",
        "/api/notifications/preferences/telegram",
        Some(serde_json::json!({"enabled": true, "destination": destination})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    json
}

async fn start(ctx: &Ctx) -> (StatusCode, serde_json::Value) {
    call(
        ctx,
        "POST",
        "/api/notifications/preferences/telegram/verify/start",
        None,
    )
    .await
}

async fn confirm(ctx: &Ctx, code: &str) -> (StatusCode, serde_json::Value) {
    call(
        ctx,
        "POST",
        "/api/notifications/preferences/telegram/verify/confirm",
        Some(serde_json::json!({"code": code})),
    )
    .await
}

async fn telegram_pref(ctx: &Ctx) -> serde_json::Value {
    let (status, json) = call(ctx, "GET", "/api/notifications/preferences", None).await;
    assert_eq!(status, StatusCode::OK);
    json.as_array()
        .unwrap()
        .iter()
        .find(|p| p["channel_type"] == "telegram")
        .cloned()
        .unwrap()
}

async fn verification_row(ctx: &Ctx) -> notification_verification::Model {
    notification_verification::Entity::find()
        .filter(notification_verification::Column::UserId.eq(ctx.user_id))
        .filter(notification_verification::Column::ChannelType.eq("telegram"))
        .one(&ctx.db)
        .await
        .unwrap()
        .expect("verification row")
}

/// Pretend the last send happened `secs` seconds ago.
async fn backdate_last_send(ctx: &Ctx, secs: i64) {
    let row = verification_row(ctx).await;
    let mut active: notification_verification::ActiveModel = row.into();
    active.last_sent_at = Set(Some(chrono::Utc::now() - chrono::Duration::seconds(secs)));
    active.update(&ctx.db).await.unwrap();
}

fn wrong(code: &str) -> String {
    let n: u32 = code.parse().unwrap();
    format!("{:06}", (n + 1) % 1_000_000)
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn start_then_confirm_verifies_and_delivers_events() {
    let ctx = setup().await;
    set_destination(&ctx, "123456789").await;

    let (status, json) = start(&ctx).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["expires_in_seconds"], 600);
    assert_eq!(json["resend_after_seconds"], 60);
    assert_eq!(json["destination"], "123***89");

    let messages = ctx.capture.messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].recipient, "123456789");
    assert!(messages[0]
        .body
        .starts_with("Your Kubarr verification code is "));
    let code = ctx.capture.last_code();
    assert_eq!(code.len(), 6);

    // The code itself is never stored.
    let row = verification_row(&ctx).await;
    assert!(!row.code_hash.clone().unwrap().contains(&code));

    let pref = telegram_pref(&ctx).await;
    assert_eq!(pref["verification_pending"], true);
    assert_eq!(pref["verified"], false);

    let (status, json) = confirm(&ctx, &format!(" {} {} ", &code[..3], &code[3..])).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["verified"], true);
    assert_eq!(json["verification_pending"], false);

    let pref = telegram_pref(&ctx).await;
    assert_eq!(pref["verified"], true);
    assert_eq!(pref["verification_pending"], false);
    assert!(verification_row(&ctx).await.code_hash.is_none());

    // A code is single-use.
    let (status, json) = confirm(&ctx, &code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json["detail"]
        .as_str()
        .unwrap()
        .contains("No verification code"));

    // Verified preferences now receive events.
    notification_event::ActiveModel {
        event_type: Set("app_update_failed".to_string()),
        enabled: Set(true),
        severity: Set("warning".to_string()),
        ..Default::default()
    }
    .insert(&ctx.db)
    .await
    .ok();
    ctx.state
        .notification
        .notify_event(
            &AuditAction::AppUpdateFailed,
            Some(ctx.user_id),
            Some("verifier"),
            Some("sonarr"),
        )
        .await
        .unwrap();
    let messages = ctx.capture.messages();
    assert_eq!(messages.len(), 2, "event must reach the verified channel");
    assert_eq!(messages[1].recipient, "123456789");
    assert_eq!(messages[1].title, "App Update Failed");

    // Both steps are audited without the code or the full destination.
    let logs = audit_log::Entity::find()
        .filter(audit_log::Column::UserId.eq(ctx.user_id))
        .all(&ctx.db)
        .await
        .unwrap();
    let verify_logs: Vec<_> = logs
        .iter()
        .filter(|l| l.action.starts_with("notification_"))
        .collect();
    assert!(verify_logs
        .iter()
        .any(|l| l.action == "notification_verification_sent" && l.success));
    assert!(verify_logs
        .iter()
        .any(|l| l.action == "notification_destination_verified" && l.success));
    assert!(verify_logs
        .iter()
        .any(|l| l.action == "notification_destination_verified" && !l.success));
    for log in verify_logs {
        let details = log.details.clone().unwrap_or_default();
        assert!(!details.contains(&code), "audit must not contain the code");
        assert!(
            !details.contains("123456789"),
            "audit must mask the destination"
        );
    }
}

#[tokio::test]
async fn wrong_code_counts_attempts_and_sixth_is_rejected() {
    let ctx = setup().await;
    set_destination(&ctx, "123456789").await;
    assert_eq!(start(&ctx).await.0, StatusCode::OK);
    let code = ctx.capture.last_code();

    for attempt in 1..=5 {
        let (status, json) = confirm(&ctx, &wrong(&code)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let detail = json["detail"].as_str().unwrap();
        assert!(detail.contains("Incorrect"), "{detail}");
        assert_eq!(verification_row(&ctx).await.attempts, attempt);
        if attempt == 4 {
            assert!(detail.contains("1 attempts left"), "{detail}");
        }
    }
    assert_eq!(telegram_pref(&ctx).await["verification_pending"], false);

    // Even the right code is refused after five wrong guesses.
    let (status, json) = confirm(&ctx, &code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json["detail"].as_str().unwrap().contains("Too many"));
    assert_eq!(telegram_pref(&ctx).await["verified"], false);
}

#[tokio::test]
async fn expired_code_is_rejected() {
    let ctx = setup().await;
    set_destination(&ctx, "123456789").await;
    assert_eq!(start(&ctx).await.0, StatusCode::OK);
    let code = ctx.capture.last_code();

    let row = verification_row(&ctx).await;
    let mut active: notification_verification::ActiveModel = row.into();
    active.expires_at = Set(Some(chrono::Utc::now() - chrono::Duration::seconds(1)));
    active.update(&ctx.db).await.unwrap();

    assert_eq!(telegram_pref(&ctx).await["verification_pending"], false);
    let (status, json) = confirm(&ctx, &code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json["detail"].as_str().unwrap().contains("expired"));
    assert_eq!(telegram_pref(&ctx).await["verified"], false);
}

#[tokio::test]
async fn start_is_rate_limited() {
    let ctx = setup().await;
    set_destination(&ctx, "123456789").await;

    assert_eq!(start(&ctx).await.0, StatusCode::OK);
    let (status, json) = start(&ctx).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(json["detail"].as_str().unwrap().contains("wait"));

    for _ in 2..=5 {
        backdate_last_send(&ctx, 61).await;
        assert_eq!(start(&ctx).await.0, StatusCode::OK);
    }
    backdate_last_send(&ctx, 61).await;
    let (status, json) = start(&ctx).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(json["detail"].as_str().unwrap().contains("Too many"));
    assert_eq!(ctx.capture.messages().len(), 5);

    // The hourly window resets.
    let row = verification_row(&ctx).await;
    let mut active: notification_verification::ActiveModel = row.into();
    active.window_started_at = Set(Some(chrono::Utc::now() - chrono::Duration::minutes(61)));
    active.update(&ctx.db).await.unwrap();
    assert_eq!(start(&ctx).await.0, StatusCode::OK);
}

#[tokio::test]
async fn destination_change_resets_verified_and_pending_code() {
    let ctx = setup().await;
    set_destination(&ctx, "123456789").await;
    assert_eq!(start(&ctx).await.0, StatusCode::OK);
    let code = ctx.capture.last_code();
    assert_eq!(confirm(&ctx, &code).await.0, StatusCode::OK);
    assert_eq!(telegram_pref(&ctx).await["verified"], true);

    // Pending code for the current destination, then change destination.
    backdate_last_send(&ctx, 61).await;
    assert_eq!(start(&ctx).await.0, StatusCode::OK);
    let code = ctx.capture.last_code();
    let json = set_destination(&ctx, "987654321").await;
    assert_eq!(json["verified"], false);
    assert_eq!(json["verification_pending"], false);
    assert!(verification_row(&ctx).await.code_hash.is_none());

    let (status, _) = confirm(&ctx, &code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(telegram_pref(&ctx).await["verified"], false);

    // Same destination again does not reset anything.
    backdate_last_send(&ctx, 61).await;
    assert_eq!(start(&ctx).await.0, StatusCode::OK);
    set_destination(&ctx, "987654321").await;
    assert_eq!(telegram_pref(&ctx).await["verification_pending"], true);
}

#[tokio::test]
async fn start_requires_enabled_preference_with_destination() {
    let ctx = setup().await;
    let (status, _) = start(&ctx).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &ctx,
        "PUT",
        "/api/notifications/preferences/telegram",
        Some(serde_json::json!({"enabled": false, "destination": "123456789"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = start(&ctx).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &ctx,
        "POST",
        "/api/notifications/preferences/pigeon/verify/start",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(ctx.capture.messages().is_empty());
}

#[tokio::test]
async fn failed_send_returns_502_and_leaves_no_pending_code() {
    let ctx = setup_with(true).await;
    set_destination(&ctx, "123456789").await;
    let (status, json) = start(&ctx).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{json}");
    // The provider's error text (which may echo the destination) is not returned.
    let detail = json["detail"].as_str().unwrap();
    assert!(!detail.contains("stub failure"), "{detail}");
    assert!(detail.contains("Could not send the verification code"));
    assert_eq!(telegram_pref(&ctx).await["verification_pending"], false);
    let code = ctx.capture.last_code();
    assert_eq!(confirm(&ctx, &code).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unverified_preference_receives_nothing() {
    let ctx = setup().await;
    set_destination(&ctx, "123456789").await;
    notification_event::ActiveModel {
        event_type: Set("app_updated".to_string()),
        enabled: Set(true),
        severity: Set("info".to_string()),
        ..Default::default()
    }
    .insert(&ctx.db)
    .await
    .ok();
    // The seed may already hold the row disabled; make sure it is on.
    let event = notification_event::Entity::find()
        .filter(notification_event::Column::EventType.eq("app_updated"))
        .one(&ctx.db)
        .await
        .unwrap()
        .unwrap();
    let mut active: notification_event::ActiveModel = event.into();
    active.enabled = Set(true);
    active.update(&ctx.db).await.unwrap();

    ctx.state
        .notification
        .notify_event(&AuditAction::AppUpdated, Some(ctx.user_id), None, None)
        .await
        .unwrap();
    assert!(ctx.capture.messages().is_empty());
}

#[tokio::test]
async fn events_endpoint_lists_enum_derived_types() {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "evadmin", "ev@example.com", "password123", "admin").await;
    // The seeded admin role lacks settings.view; add it.
    {
        use kubarr::models::{role, role_permission};
        let admin = kubarr::models::prelude::Role::find()
            .filter(role::Column::Name.eq("admin"))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        role_permission::ActiveModel {
            role_id: Set(admin.id),
            permission: Set("settings.view".to_string()),
            ..Default::default()
        }
        .insert(&db)
        .await
        .ok();
    }
    let state = build_test_app_state_with_db(db.clone()).await;
    let body = serde_json::json!({"username": "evadmin", "password": "password123"}).to_string();
    let response = create_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/auth/login")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            (s.starts_with("kubarr_session=") && !s.contains("kubarr_session_"))
                .then(|| s.split(';').next().unwrap().to_string())
        })
        .unwrap();
    let ctx = Ctx {
        state,
        db,
        cookie,
        user_id: 0,
        capture: Capture::new(ChannelType::Email, false),
    };

    let (status, json) = call(&ctx, "GET", "/api/notifications/events", None).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let types: Vec<String> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event_type"].as_str().unwrap().to_string())
        .collect();
    for expected in [
        "app_updated",
        "app_update_failed",
        "app_update_rollback_failed",
        "vpn_provider_created",
        "vpn_assigned",
        "vpn_removed",
        "2fa_verified",
        "2fa_failed",
        "user_activated",
        "invite_deleted",
        "login_failed",
    ] {
        assert!(types.iter().any(|t| t == expected), "missing {expected}");
    }
    for noise in ["api_access", "token_refresh", "app_accessed"] {
        assert!(
            !types.iter().any(|t| t == noise),
            "{noise} must be excluded"
        );
    }
    assert_eq!(types.len(), AuditAction::notifiable().count());

    // Seeded severities are returned for seeded rows.
    let failed = json
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["event_type"] == "app_update_rollback_failed")
        .unwrap();
    assert_eq!(failed["severity"], "critical");
}

#[tokio::test]
async fn rate_limited_start_sets_retry_after() {
    let ctx = setup().await;
    set_destination(&ctx, "123456789").await;
    assert_eq!(start(&ctx).await.0, StatusCode::OK);
    let response = create_router(ctx.state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/notifications/preferences/telegram/verify/start")
                .method("POST")
                .header("Cookie", &ctx.cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry: u64 = response
        .headers()
        .get(header::RETRY_AFTER)
        .expect("Retry-After header")
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry), "retry after {retry}");
}

#[tokio::test]
async fn parallel_first_starts_never_500() {
    let ctx = setup().await;
    set_destination(&ctx, "123456789").await;
    let (a, b) = tokio::join!(start(&ctx), start(&ctx));
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::TOO_MANY_REQUESTS],
        "{a:?} {b:?}"
    );
    assert_eq!(ctx.capture.messages().len(), 1);
}

#[tokio::test]
async fn destination_changed_between_check_and_save_is_not_verified() {
    use kubarr::models::user_notification_pref;
    use kubarr::services::notification::verification;

    let ctx = setup().await;
    set_destination(&ctx, "123456789").await;
    // The confirm handler checked a code bound to 123456789 ...
    let pref = user_notification_pref::Entity::find()
        .filter(user_notification_pref::Column::UserId.eq(ctx.user_id))
        .filter(user_notification_pref::Column::ChannelType.eq("telegram"))
        .one(&ctx.db)
        .await
        .unwrap()
        .unwrap();
    // ... and a concurrent PUT switched the destination before the save.
    set_destination(&ctx, "987654321").await;

    let saved = verification::mark_verified(&ctx.db, pref.id, ctx.user_id, "telegram", "123456789")
        .await
        .unwrap();
    assert!(!saved, "stale destination must not be verified");
    assert_eq!(telegram_pref(&ctx).await["verified"], false);

    // The current destination still verifies normally.
    assert!(
        verification::mark_verified(&ctx.db, pref.id, ctx.user_id, "telegram", "987654321")
            .await
            .unwrap()
    );
    assert_eq!(telegram_pref(&ctx).await["verified"], true);
}

#[tokio::test]
async fn stale_enabled_noise_event_does_not_dispatch() {
    let ctx = setup().await;
    set_destination(&ctx, "123456789").await;
    assert_eq!(start(&ctx).await.0, StatusCode::OK);
    let code = ctx.capture.last_code();
    assert_eq!(confirm(&ctx, &code).await.0, StatusCode::OK);

    for event in ["app_accessed", "api_access", "token_refresh"] {
        notification_event::ActiveModel {
            event_type: Set(event.to_string()),
            enabled: Set(true),
            severity: Set("info".to_string()),
            ..Default::default()
        }
        .insert(&ctx.db)
        .await
        .ok();
    }
    for action in [
        AuditAction::AppAccessed,
        AuditAction::ApiAccess,
        AuditAction::TokenRefresh,
    ] {
        ctx.state
            .notification
            .notify_event(&action, Some(ctx.user_id), Some("verifier"), None)
            .await
            .unwrap();
    }
    assert_eq!(
        ctx.state
            .notification
            .get_unread_count(ctx.user_id)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        ctx.capture.messages().len(),
        1,
        "only the verification code"
    );
}
