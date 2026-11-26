//! `/api/assistant` integration tests: the API key is write-only, the
//! endpoints are permission-gated, the key test talks to a local stand-in for
//! the Messages API, and the status reports budget state.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, HeaderMap, Request, StatusCode},
    routing::get,
    Json, Router,
};
use http_body_util::BodyExt;
use parking_lot::Mutex;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

use kubarr::endpoints::create_router;
use kubarr::error::AppError;
use kubarr::models::{assistant_usage, audit_log, role, role_permission};
use kubarr::services::assistant::{AssistantDeps, KeyStore, MemoryKeyStore, MessagesClient};
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

/// A key-shaped value assembled at runtime: no credential literal in source.
fn test_key() -> String {
    format!("sk-ant-api03-{}", uuid::Uuid::new_v4().simple())
}

struct Harness {
    app: Router,
    cookie: String,
    db: DatabaseConnection,
    store: Arc<dyn KeyStore>,
}

async fn login(app: Router, username: &str, password: &str) -> String {
    let body = serde_json::json!({ "username": username, "password": password }).to_string();
    let response = app
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
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            (s.starts_with("kubarr_session=") && !s.contains("kubarr_session_"))
                .then(|| s.split(';').next().unwrap().to_string())
        })
        .expect("login must set a session cookie")
}

async fn harness_with(
    role: &str,
    store: impl KeyStore + 'static,
    base_url: Option<&str>,
) -> Harness {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;
    let name = format!("{role}-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let password = uuid::Uuid::new_v4().to_string();
    create_test_user_with_role(&db, &name, &format!("{name}@example.com"), &password, role).await;
    let mut state: AppState = build_test_app_state_with_db(db.clone()).await;
    let store: Arc<dyn KeyStore> = Arc::new(store);
    state.assistant = AssistantDeps {
        key_store: store.clone(),
        client: MessagesClient::new(base_url.unwrap_or("http://127.0.0.1:9")),
        releases: std::sync::Arc::new(kubarr::services::assistant::actions::HelmReleaseHistory),
    };
    let cookie = login(create_router(state.clone()), &name, &password).await;
    Harness {
        app: create_router(state),
        cookie,
        db,
        store,
    }
}

async fn harness(role: &str) -> Harness {
    harness_with(role, MemoryKeyStore::default(), None).await
}

impl Harness {
    async fn call(&self, method: &str, uri: &str, body: Option<String>) -> (StatusCode, String) {
        let mut request = Request::builder()
            .uri(uri)
            .method(method)
            .header("Cookie", &self.cookie);
        if body.is_some() {
            request = request.header("content-type", "application/json");
        }
        let response = self
            .app
            .clone()
            .oneshot(
                request
                    .body(body.map(Body::from).unwrap_or_else(Body::empty))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    async fn json(&self, method: &str, uri: &str, body: Option<String>) -> serde_json::Value {
        let (status, text) = self.call(method, uri, body).await;
        assert_eq!(status, StatusCode::OK, "{method} {uri}: {text}");
        serde_json::from_str(&text).unwrap()
    }
}

/// A stand-in for `GET /v1/models/{model}` that records request headers and
/// paths and answers with a fixed status and body.
#[derive(Clone, Default)]
struct Seen {
    requests: Arc<Mutex<Vec<(HeaderMap, String)>>>,
}

async fn mock_models_api(status: StatusCode, reply: serde_json::Value) -> (String, Seen) {
    let seen = Seen::default();
    let recorder = seen.clone();
    let app = Router::new().route(
        "/v1/models/{model}",
        get(move |headers: HeaderMap, uri: axum::http::Uri| {
            let recorder = recorder.clone();
            let reply = reply.clone();
            async move {
                recorder
                    .requests
                    .lock()
                    .push((headers, uri.path().to_string()));
                (status, Json(reply))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), seen)
}

fn model_reply() -> serde_json::Value {
    serde_json::json!({
        "type": "model", "id": "claude-haiku-5-5",
        "display_name": "Claude Haiku 5.5", "created_at": "2026-10-01T00:00:00Z"
    })
}

/// A key store whose Secret cannot be read (RBAC, cluster down).
struct BrokenStore;

#[async_trait::async_trait]
impl KeyStore for BrokenStore {
    async fn get(&self) -> kubarr::error::Result<Option<String>> {
        Err(AppError::Internal(
            "Failed to read the assistant API key (403)".into(),
        ))
    }
    async fn set(&self, _key: &str) -> kubarr::error::Result<()> {
        Err(AppError::Internal(
            "Failed to store the assistant API key (403)".into(),
        ))
    }
    async fn delete(&self) -> kubarr::error::Result<()> {
        Err(AppError::Internal(
            "Failed to remove the assistant API key (403)".into(),
        ))
    }
}

async fn grant(db: &DatabaseConnection, role_name: &str, permission: &str) {
    let role = role::Entity::find()
        .filter(role::Column::Name.eq(role_name))
        .one(db)
        .await
        .unwrap()
        .unwrap();
    role_permission::ActiveModel {
        role_id: Set(role.id),
        permission: Set(permission.into()),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
}

#[tokio::test]
async fn admin_role_is_seeded_with_assistant_use() {
    let db = create_test_db_with_seed().await;
    let admin = role::Entity::find()
        .filter(role::Column::Name.eq("admin"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let granted = role_permission::Entity::find()
        .filter(role_permission::Column::RoleId.eq(admin.id))
        .filter(role_permission::Column::Permission.eq("assistant.use"))
        .one(&db)
        .await
        .unwrap();
    assert!(granted.is_some());
    let viewer_has_it = role_permission::Entity::find()
        .filter(role_permission::Column::Permission.eq("assistant.use"))
        .all(&db)
        .await
        .unwrap()
        .len();
    assert_eq!(viewer_has_it, 1, "only the admin role gets it by default");
}

#[tokio::test]
async fn budget_warning_event_is_seeded_enabled_and_routed() {
    let db = create_test_db_with_seed().await;
    let event = kubarr::models::notification_event::Entity::find()
        .filter(
            kubarr::models::notification_event::Column::EventType.eq("assistant_budget_warning"),
        )
        .one(&db)
        .await
        .unwrap()
        .expect("event seeded");
    assert!(event.enabled);
    assert_eq!(event.severity, "warning");
    let routed =
        kubarr::services::notification::routing::routed_channels(&db, "assistant_budget_warning")
            .await
            .unwrap();
    assert_eq!(routed.len(), 1);
    assert!(routed.contains(&kubarr::services::notification::ChannelType::Telegram));
}

#[tokio::test]
async fn the_key_is_write_only() {
    let h = harness("admin").await;
    let key = test_key();

    assert_eq!(
        h.json("GET", "/api/assistant/key", None).await,
        serde_json::json!({"configured": false})
    );

    let (status, body) = h
        .call(
            "PUT",
            "/api/assistant/key",
            Some(serde_json::json!({ "api_key": format!(" {key}\n") }).to_string()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body.contains(&key));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"configured": true})
    );
    // Stored trimmed.
    assert_eq!(h.store.get().await.unwrap().as_deref(), Some(key.as_str()));

    for uri in [
        "/api/assistant/key",
        "/api/assistant/status",
        "/api/assistant/settings",
        "/api/settings",
    ] {
        let (status, body) = h.call("GET", uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert!(!body.contains(&key), "{uri} leaks the key");
    }

    // The audit trail records the change, not the key.
    let audit = audit_log::Entity::find().all(&h.db).await.unwrap();
    let change = audit
        .iter()
        .find(|row| row.resource_id.as_deref() == Some("assistant.api_key"))
        .expect("key change audited");
    assert_eq!(change.action, "system_setting_changed");
    for row in &audit {
        let text = serde_json::to_string(row).unwrap();
        assert!(!text.contains(&key), "audit row leaks the key: {text}");
    }

    let (status, body) = h.call("DELETE", "/api/assistant/key", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        h.json("GET", "/api/assistant/key", None).await,
        serde_json::json!({"configured": false})
    );
}

#[tokio::test]
async fn malformed_keys_are_rejected_without_echo() {
    let h = harness("admin").await;
    let not_anthropic = format!("sk-proj-{}", uuid::Uuid::new_v4().simple());
    let (status, body) = h
        .call(
            "PUT",
            "/api/assistant/key",
            Some(serde_json::json!({ "api_key": not_anthropic }).to_string()),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("api_key"), "{body}");
    assert!(!body.contains(&not_anthropic), "{body}");

    let key = test_key();
    let (status, body) = h
        .call(
            "PUT",
            "/api/assistant/key",
            Some(format!("{{\"api_key\": [\"{key}\"]}}")),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!body.contains(&key), "{body}");
}

#[tokio::test]
async fn viewers_cannot_touch_the_key_or_settings() {
    let h = harness_with("viewer", MemoryKeyStore::with_key(&test_key()), None).await;
    for (method, uri, body) in [
        ("GET", "/api/assistant/key", None),
        (
            "PUT",
            "/api/assistant/key",
            Some(serde_json::json!({"api_key": test_key()}).to_string()),
        ),
        ("DELETE", "/api/assistant/key", None),
        ("POST", "/api/assistant/key/test", None),
        ("GET", "/api/assistant/settings", None),
        (
            "PUT",
            "/api/assistant/settings",
            Some(
                serde_json::json!({"enabled": true, "model": "claude-opus-5-5", "effort": "low",
                                   "monthly_budget_usd": 20.0, "retention_days": 30})
                .to_string(),
            ),
        ),
        // No settings.view and no assistant.use.
        ("GET", "/api/assistant/status", None),
    ] {
        let (status, text) = h.call(method, uri, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {text}");
    }
}

#[tokio::test]
async fn assistant_users_see_status_but_not_settings() {
    let h = harness("viewer").await;
    grant(&h.db, "viewer", "assistant.use").await;
    // Permissions are resolved per request, so a fresh request sees the grant.
    let status = h.json("GET", "/api/assistant/status", None).await;
    assert_eq!(status["configured"], false);
    let (code, _) = h.call("GET", "/api/assistant/key", None).await;
    assert_eq!(code, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn status_reports_settings_spend_and_pause() {
    let h = harness_with("admin", MemoryKeyStore::with_key(&test_key()), None).await;
    let status = h.json("GET", "/api/assistant/status", None).await;
    assert_eq!(
        status,
        serde_json::json!({
            "configured": true, "key_error": false, "enabled": true,
            "model": "claude-opus-5-5", "effort": "low",
            "month_spend_usd": 0.0, "monthly_budget_usd": 20.0, "paused": false
        })
    );

    assistant_usage::ActiveModel {
        model: Set("claude-opus-5-5".into()),
        cost_usd_micros: Set(20_000_000),
        created_at: Set(chrono::Utc::now()),
        ..Default::default()
    }
    .insert(&h.db)
    .await
    .unwrap();
    let status = h.json("GET", "/api/assistant/status", None).await;
    assert_eq!(status["month_spend_usd"], 20.0);
    assert_eq!(status["paused"], true);

    // Raising the budget resumes the assistant.
    let saved = h
        .json(
            "PUT",
            "/api/assistant/settings",
            Some(
                serde_json::json!({"enabled": false, "model": "claude-sonnet-5-5", "effort": "medium",
                                   "monthly_budget_usd": 40.0, "retention_days": 14})
                .to_string(),
            ),
        )
        .await;
    assert_eq!(saved["model"], "claude-sonnet-5-5");
    let status = h.json("GET", "/api/assistant/status", None).await;
    assert_eq!(status["paused"], false);
    assert_eq!(status["enabled"], false);
    assert_eq!(status["effort"], "medium");
    assert_eq!(status["monthly_budget_usd"], 40.0);
}

#[tokio::test]
async fn settings_reject_unknown_models_and_bad_values() {
    let h = harness("admin").await;
    let base = serde_json::json!({"enabled": true, "model": "claude-opus-5-5", "effort": "low",
                                  "monthly_budget_usd": 20.0, "retention_days": 30});
    for (field, value) in [
        ("model", serde_json::json!("gpt-5")),
        ("effort", serde_json::json!("max")),
        ("monthly_budget_usd", serde_json::json!(-5)),
        ("retention_days", serde_json::json!(0)),
    ] {
        let mut body = base.clone();
        body[field] = value;
        let (status, text) = h
            .call("PUT", "/api/assistant/settings", Some(body.to_string()))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {text}");
    }
}

#[tokio::test]
async fn key_test_checks_the_model_without_spending_tokens() {
    let (base_url, seen) = mock_models_api(StatusCode::OK, model_reply()).await;
    let key = test_key();
    let h = harness_with("admin", MemoryKeyStore::with_key(&key), Some(&base_url)).await;
    h.json(
        "PUT",
        "/api/assistant/settings",
        Some(
            serde_json::json!({"enabled": true, "model": "claude-haiku-5-5", "effort": "low",
                               "monthly_budget_usd": 20.0, "retention_days": 30})
            .to_string(),
        ),
    )
    .await;

    let result = h.json("POST", "/api/assistant/key/test", None).await;
    assert_eq!(result, serde_json::json!({"ok": true}));

    let requests = seen.requests.lock().clone();
    assert_eq!(requests.len(), 1);
    let (headers, path) = &requests[0];
    assert_eq!(path, "/v1/models/claude-haiku-5-5");
    assert_eq!(headers["x-api-key"], key.as_str());
    assert_eq!(headers["anthropic-version"], "2023-06-01");
    assert!(assistant_usage::Entity::find()
        .all(&h.db)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn key_test_maps_statuses_to_fixed_messages() {
    for (status, kind, expected) in [
        (
            StatusCode::FORBIDDEN,
            "permission_error",
            "Anthropic rejected the API key.",
        ),
        (
            StatusCode::NOT_FOUND,
            "not_found_error",
            "The selected model is not available for this API key.",
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "api_error",
            "Anthropic is temporarily unavailable. Try again later.",
        ),
    ] {
        let reply = serde_json::json!({"type": "error", "error": {"type": kind, "message": "x"}});
        let (base_url, _) = mock_models_api(status, reply).await;
        let h = harness_with(
            "admin",
            MemoryKeyStore::with_key(&test_key()),
            Some(&base_url),
        )
        .await;
        assert_eq!(
            h.json("POST", "/api/assistant/key/test", None).await,
            serde_json::json!({"ok": false, "error": expected}),
            "{status}"
        );
    }
}

#[tokio::test]
async fn an_unreadable_key_secret_is_reported_not_hidden() {
    let h = harness_with("admin", BrokenStore, None).await;
    let status = h.json("GET", "/api/assistant/status", None).await;
    assert_eq!(status["configured"], false);
    assert_eq!(status["key_error"], true);
    let (code, body) = h.call("GET", "/api/assistant/key", None).await;
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE, "{body}");
}

#[tokio::test]
async fn key_test_failures_are_generic_and_never_echo_the_key() {
    let key = test_key();
    // Providers sometimes quote the credential back; it must not reach the UI.
    let reply = serde_json::json!({
        "type": "error",
        "error": {"type": "authentication_error", "message": format!("invalid x-api-key {key}")}
    });
    let (base_url, _) = mock_models_api(StatusCode::UNAUTHORIZED, reply).await;
    let h = harness_with("admin", MemoryKeyStore::with_key(&key), Some(&base_url)).await;
    let (status, body) = h.call("POST", "/api/assistant/key/test", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body.contains(&key), "{body}");
    assert!(!body.contains("invalid x-api-key"), "{body}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"ok": false, "error": "Anthropic rejected the API key."})
    );
    assert!(assistant_usage::Entity::find()
        .all(&h.db)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn key_test_without_a_key_or_server() {
    let h = harness("admin").await;
    assert_eq!(
        h.json("POST", "/api/assistant/key/test", None).await,
        serde_json::json!({"ok": false, "error": "No API key is configured."})
    );

    // Nothing listens on the discard port: a transport failure, not a 500.
    let h = harness_with("admin", MemoryKeyStore::with_key(&test_key()), None).await;
    assert_eq!(
        h.json("POST", "/api/assistant/key/test", None).await,
        serde_json::json!({"ok": false, "error": "Could not reach the Anthropic API."})
    );
}
