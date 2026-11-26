//! Assistant chat integration tests against a local stand-in for the
//! streaming Messages API: the tool loop, limits, budget, permission scoping,
//! conversation ownership, the rate limit, usage, refusals, interrupted
//! streams and retention.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, HeaderMap, Request, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use chrono::{Duration, Utc};
use http_body_util::BodyExt;
use parking_lot::Mutex;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde_json::{json, Value};
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

use kubarr::endpoints::create_router;
use kubarr::models::{
    app_state, assistant_conversation, assistant_message, assistant_request, assistant_usage,
    audit_log, role, role_permission, user_notification,
};
use kubarr::services::assistant::chat::{MAX_TOOL_CALLS, REFUSAL_TEXT};
use kubarr::services::assistant::conversations;
use kubarr::services::assistant::{
    settings, AssistantDeps, AssistantSettings, MemoryKeyStore, MessagesClient,
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

// ---------------------------------------------------------------------------
// Stub Messages API
// ---------------------------------------------------------------------------

type Script = Arc<dyn Fn(usize, &Value) -> String + Send + Sync>;

#[derive(Clone, Default)]
struct Seen {
    requests: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
}

impl Seen {
    fn bodies(&self) -> Vec<Value> {
        self.requests
            .lock()
            .iter()
            .map(|(_, b)| b.clone())
            .collect()
    }
}

/// Answers request `n` with `script(n, body)` as an SSE stream.
async fn stub_api(script: Script) -> (String, Seen) {
    let seen = Seen::default();
    let recorder = seen.clone();
    let app = Router::new().route(
        "/v1/messages",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let recorder = recorder.clone();
            let script = script.clone();
            async move {
                let n = {
                    let mut requests = recorder.requests.lock();
                    requests.push((headers, body.clone()));
                    requests.len() - 1
                };
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    script(n, &body),
                )
                    .into_response()
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

fn sse(events: &[Value]) -> String {
    events
        .iter()
        .map(|e| format!("event: {}\ndata: {}\n\n", e["type"].as_str().unwrap(), e))
        .collect()
}

fn start() -> Value {
    json!({"type": "message_start", "message": {"id": "msg", "model": "claude-opus-5-5",
        "usage": {"input_tokens": 100, "output_tokens": 1, "cache_read_input_tokens": 50,
                  "cache_creation_input_tokens": 10}}})
}

fn stop(reason: &str) -> Vec<Value> {
    vec![
        json!({"type": "message_delta", "delta": {"stop_reason": reason}, "usage": {"output_tokens": 20}}),
        json!({"type": "message_stop"}),
    ]
}

fn text_reply(text: &str) -> String {
    let mut events = vec![
        start(),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
        json!({"type": "content_block_stop", "index": 0}),
    ];
    events.extend(stop("end_turn"));
    sse(&events)
}

/// A thinking block followed by one tool call whose input arrives in two pieces.
fn tool_reply(id: &str, name: &str, input: &Value) -> String {
    let raw = input.to_string();
    let (a, b) = raw.split_at(raw.len() / 2);
    let mut events = vec![
        start(),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": "", "signature": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "sig-1"}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": a}}),
        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": b}}),
        json!({"type": "content_block_stop", "index": 1}),
    ];
    events.extend(stop("tool_use"));
    sse(&events)
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    app: Router,
    db: DatabaseConnection,
}

struct User {
    id: i64,
    cookie: String,
}

async fn login(app: Router, username: &str, password: &str) -> String {
    let body = json!({ "username": username, "password": password }).to_string();
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

async fn harness(base_url: &str) -> Harness {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;
    let mut state: AppState = build_test_app_state_with_db(db.clone()).await;
    state.assistant = AssistantDeps {
        key_store: Arc::new(MemoryKeyStore::with_key(&format!(
            "sk-ant-api03-{}",
            uuid::Uuid::new_v4().simple()
        ))),
        client: MessagesClient::new(base_url),
        releases: Arc::new(kubarr::services::assistant::actions::HelmReleaseHistory),
    };
    for app in ["jellyfin", "sonarr"] {
        app_state::ActiveModel {
            app_name: Set(app.into()),
            namespace: Set(app.into()),
            desired_state: Set("installed".into()),
            observed_state: Set("installed".into()),
            healthy: Set(true),
            message: Set(Some("password=hunter2hunter2 ok".into())),
            installed_chart_version: Set(Some("1.0.0".into())),
            available_chart_version: Set(Some("1.0.0".into())),
            update_available: Set(false),
            auto_update: Set(true),
            last_operation_id: Set(None),
            last_checked_at: Set(None),
            updated_at: Set(Utc::now()),
        }
        .insert(&db)
        .await
        .unwrap();
    }
    Harness {
        app: create_router(state),
        db,
    }
}

impl Harness {
    async fn user(&self, role_name: &str) -> User {
        let name = format!(
            "{role_name}-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let password = uuid::Uuid::new_v4().to_string();
        let user = create_test_user_with_role(
            &self.db,
            &name,
            &format!("{name}@example.com"),
            &password,
            role_name,
        )
        .await;
        User {
            id: user.id,
            cookie: login(self.app.clone(), &name, &password).await,
        }
    }

    async fn call(
        &self,
        user: &User,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, String) {
        let mut request = Request::builder()
            .uri(uri)
            .method(method)
            .header("Cookie", &user.cookie);
        if body.is_some() {
            request = request.header("content-type", "application/json");
        }
        let response = self
            .app
            .clone()
            .oneshot(
                request
                    .body(
                        body.map(|b| Body::from(b.to_string()))
                            .unwrap_or_else(Body::empty),
                    )
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    async fn conversation(&self, user: &User) -> String {
        let (status, body) = self
            .call(
                user,
                "POST",
                "/api/assistant/conversations",
                Some(json!({})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        serde_json::from_str::<Value>(&body).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// Ask and collect the whole event stream.
    async fn ask(
        &self,
        user: &User,
        conversation: &str,
        text: &str,
    ) -> (StatusCode, Vec<(String, Value)>) {
        let (status, body) = self
            .call(
                user,
                "POST",
                &format!("/api/assistant/conversations/{conversation}/messages"),
                Some(json!({"text": text, "context": {"page": "/apps", "app": "jellyfin"}})),
            )
            .await;
        if status != StatusCode::OK {
            return (status, vec![("http".into(), json!({"body": body}))]);
        }
        (status, parse_events(&body))
    }
}

fn parse_events(body: &str) -> Vec<(String, Value)> {
    body.split("\n\n")
        .filter(|chunk| !chunk.trim().is_empty())
        .map(|chunk| {
            let mut event = String::new();
            let mut data = String::new();
            for line in chunk.lines() {
                if let Some(v) = line.strip_prefix("event: ") {
                    event = v.to_string();
                } else if let Some(v) = line.strip_prefix("data: ") {
                    data = v.to_string();
                }
            }
            (event, serde_json::from_str(&data).unwrap())
        })
        .collect()
}

fn names(events: &[(String, Value)]) -> Vec<&str> {
    events.iter().map(|(n, _)| n.as_str()).collect()
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

/// The tool_result blocks of a request's last message.
fn tool_results(body: &Value) -> Vec<Value> {
    body["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Number of cache breakpoints in a request.
fn breakpoints(body: &Value) -> usize {
    body.to_string().matches("\"cache_control\"").count()
}

fn tool_names(body: &Value) -> Vec<String> {
    body["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tool_round_trip_streams_status_text_sources_and_records_usage() {
    let (url, seen) = stub_api(Arc::new(|n, _| match n {
        0 => tool_reply("toolu_1", "get_app", &json!({"name": "jellyfin"})),
        _ => text_reply("Jellyfin is **healthy**."),
    }))
    .await;
    let h = harness(&url).await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;

    let (status, events) = h.ask(&admin, &conversation, "How is Jellyfin?").await;
    assert_eq!(status, StatusCode::OK, "{events:?}");
    assert_eq!(names(&events), ["status", "text_delta", "sources", "done"]);
    assert_eq!(events[0].1["text"], "Looking at jellyfin…");
    assert_eq!(events[1].1["text"], "Jellyfin is **healthy**.");
    assert_eq!(
        events[2].1["items"],
        json!([{"tool": "get_app", "target": "jellyfin", "label": "App jellyfin"}])
    );

    let requests = seen.requests.lock().clone();
    assert_eq!(requests.len(), 2);
    let (headers, first) = &requests[0];
    assert_eq!(headers["anthropic-version"], "2023-06-01");
    assert_eq!(headers["anthropic-beta"], "server-side-fallback-2026-07-01");
    assert_eq!(first["fallbacks"], "default");
    assert_eq!(first["stream"], true);
    assert_eq!(first["model"], "claude-opus-5-5");
    assert_eq!(first["output_config"]["effort"], "low");
    assert_eq!(first["tool_choice"], json!({"type": "auto"}));
    assert!(first.get("thinking").is_none());
    assert_eq!(first["system"][0]["cache_control"]["type"], "ephemeral");
    assert!(first["tools"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["strict"] == true));
    // The question, then the per-question context as a system message.
    let messages = first["messages"].as_array().unwrap();
    // The rolling cache breakpoint sits on the question, before the context.
    assert_eq!(
        messages[0],
        json!({"role": "user", "content": [{"type": "text", "text": "How is Jellyfin?",
            "cache_control": {"type": "ephemeral"}}]})
    );
    assert_eq!(messages[1]["role"], "system");
    assert!(messages[1].get("cache_control").is_none());
    assert_eq!(breakpoints(first), 2);
    assert!(messages[1]["content"]
        .as_str()
        .unwrap()
        .contains("`jellyfin`"));

    // The second request replays the assistant content unchanged and sends
    // the tool result in one user message.
    let second = &requests[1].1;
    let messages = second["messages"].as_array().unwrap();
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(
        messages[2]["content"][0],
        json!({"type": "thinking", "thinking": "", "signature": "sig-1"})
    );
    assert_eq!(
        messages[2]["content"][1]["input"],
        json!({"name": "jellyfin"})
    );
    // The breakpoint moved from the question to the newest tool result.
    assert_eq!(messages[0]["content"], json!("How is Jellyfin?"));
    assert_eq!(breakpoints(second), 2);
    let results = tool_results(second);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["cache_control"], json!({"type": "ephemeral"}));
    assert_eq!(results[0]["tool_use_id"], "toolu_1");
    assert!(results[0].get("is_error").is_none());
    let content = results[0]["content"].as_str().unwrap();
    assert!(content.contains("\"installed\":true"), "{content}");
    // Inline secrets in app messages are scrubbed before the model sees them.
    assert!(!content.contains("hunter2hunter2"), "{content}");

    let usage = assistant_usage::Entity::find()
        .filter(assistant_usage::Column::ConversationId.eq(conversation.clone()))
        .all(&h.db)
        .await
        .unwrap();
    assert_eq!(usage.len(), 2);
    assert!(usage.iter().all(|u| u.input_tokens == 100
        && u.output_tokens == 20
        && u.cache_read_tokens == 50
        && u.cache_write_tokens == 10
        && u.user_id == Some(admin.id)
        && u.cost_usd_micros > 0));

    let audits = audit_log::Entity::find()
        .filter(audit_log::Column::Action.eq("assistant_tool_call"))
        .all(&h.db)
        .await
        .unwrap();
    assert_eq!(audits.len(), 1);
    let details: Value = serde_json::from_str(audits[0].details.as_deref().unwrap()).unwrap();
    assert_eq!(details["tool"], "get_app");
    assert_eq!(details["target"], "jellyfin");
    assert_eq!(details["conversation_id"], conversation.as_str());

    let (status, body) = h
        .call(
            &admin,
            "GET",
            &format!("/api/assistant/conversations/{conversation}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let detail: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(detail["title"], "How is Jellyfin?");
    let messages = detail["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[1]["text"], "Jellyfin is **healthy**.");
    assert_eq!(messages[1]["status"], "complete");
    assert_eq!(messages[1]["sources"][0]["tool"], "get_app");
    assert_eq!(events[3].1["message_id"], messages[1]["id"]);

    // A follow-up sends the earlier turn as plain text; the breakpoint has
    // moved to the new question.
    let (_, events) = h.ask(&admin, &conversation, "And now?").await;
    assert_eq!(names(&events), ["text_delta", "done"]);
    let third = &seen.bodies()[2];
    assert_eq!(breakpoints(third), 2);
    let messages = third["messages"].as_array().unwrap();
    assert_eq!(
        messages[0],
        json!({"role": "user", "content": "How is Jellyfin?"})
    );
    assert_eq!(
        messages[1],
        json!({"role": "assistant", "content": "Jellyfin is **healthy**."})
    );
    assert_eq!(
        messages[2],
        json!({"role": "user", "content": [{"type": "text", "text": "And now?",
            "cache_control": {"type": "ephemeral"}}]})
    );
    assert_eq!(messages[3]["role"], "system");
}

#[tokio::test]
async fn tool_calls_stop_at_the_cap_and_the_model_must_answer() {
    let (url, seen) = stub_api(Arc::new(|n, body| {
        if body["tool_choice"]["type"] == "none" {
            text_reply("Here is what I found.")
        } else {
            tool_reply(&format!("toolu_{n}"), "list_apps", &json!({}))
        }
    }))
    .await;
    let h = harness(&url).await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;

    let (_, events) = h.ask(&admin, &conversation, "Look at everything").await;
    let statuses = events.iter().filter(|(n, _)| n == "status").count();
    assert_eq!(statuses, MAX_TOOL_CALLS);
    assert_eq!(names(&events).last(), Some(&"done"));
    let bodies = seen.bodies();
    assert_eq!(bodies.len(), MAX_TOOL_CALLS + 1);
    assert_eq!(
        bodies.last().unwrap()["tool_choice"],
        json!({"type": "none"})
    );
    let audits = audit_log::Entity::find()
        .filter(audit_log::Column::Action.eq("assistant_tool_call"))
        .all(&h.db)
        .await
        .unwrap();
    assert_eq!(audits.len(), MAX_TOOL_CALLS);
}

#[tokio::test]
async fn a_used_up_budget_refuses_before_any_api_call() {
    let (url, seen) = stub_api(Arc::new(|_, _| text_reply("never"))).await;
    let h = harness(&url).await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    settings::save(
        &h.db,
        &AssistantSettings {
            monthly_budget_usd: 0.0,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();

    let (status, events) = h.ask(&admin, &conversation, "Hi").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(events[0].1["body"].as_str().unwrap().contains("budget"));
    assert!(seen.bodies().is_empty());
}

#[tokio::test]
async fn the_budget_is_checked_before_every_api_call() {
    // The first call costs more than the whole budget; the loop must not
    // make a second call.
    let (url, seen) = stub_api(Arc::new(|n, _| match n {
        0 => tool_reply("toolu_1", "list_apps", &json!({})),
        _ => text_reply("never"),
    }))
    .await;
    let h = harness(&url).await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    settings::save(
        &h.db,
        &AssistantSettings {
            monthly_budget_usd: 0.000_001,
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();

    let (_, events) = h.ask(&admin, &conversation, "Hi").await;
    assert_eq!(
        seen.bodies().len(),
        1,
        "no new round after the budget is hit"
    );
    assert_eq!(names(&events), ["status", "text_delta", "sources", "done"]);
    assert!(events[1].1["text"]
        .as_str()
        .unwrap()
        .contains("Budget reached"));
}

#[tokio::test]
async fn tools_are_scoped_to_the_users_apps_and_own_notifications() {
    let (url, seen) = stub_api(Arc::new(|n, _| match n {
        0 => tool_reply("toolu_1", "list_apps", &json!({})),
        1 => tool_reply("toolu_2", "list_notifications", &json!({})),
        2 => tool_reply("toolu_3", "get_app", &json!({"name": "sonarr"})),
        _ => text_reply("Done."),
    }))
    .await;
    let h = harness(&url).await;
    grant(&h.db, "viewer", "assistant.use").await;
    let admin = h.user("admin").await;
    let viewer = h.user("viewer").await;
    for (user_id, title) in [
        (admin.id, "admin-only notice"),
        (viewer.id, "viewer notice"),
    ] {
        user_notification::ActiveModel {
            user_id: Set(user_id),
            title: Set(title.into()),
            message: Set("m".into()),
            event_type: Set(None),
            severity: Set("info".into()),
            read: Set(false),
            created_at: Set(Utc::now()),
            ..Default::default()
        }
        .insert(&h.db)
        .await
        .unwrap();
    }
    let conversation = h.conversation(&viewer).await;

    let (_, events) = h.ask(&viewer, &conversation, "What can I see?").await;
    assert_eq!(names(&events).last(), Some(&"done"));
    let bodies = seen.bodies();
    // Tools needing permissions the viewer lacks are not offered.
    let offered = tool_names(&bodies[0]);
    assert!(offered.contains(&"list_apps".to_string()));
    assert!(!offered.contains(&"get_update_settings".to_string()));
    // Monitoring apps are not installed: no metrics/logs tools.
    assert!(!offered.contains(&"query_metrics".to_string()));
    assert!(!offered.contains(&"query_logs".to_string()));
    // No reachable cluster in tests: the cluster tools are left out.
    for tool in [
        "k8s_list",
        "k8s_describe",
        "k8s_events",
        "pod_logs",
        "helm_status",
    ] {
        assert!(!offered.contains(&tool.to_string()), "{tool}");
    }
    assert!(bodies[0]["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("can only see these apps"));

    let apps = tool_results(&bodies[1])[0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(apps.contains("jellyfin"), "{apps}");
    assert!(!apps.contains("sonarr"), "{apps}");
    let notifications = tool_results(&bodies[2])[0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(notifications.contains("viewer notice"), "{notifications}");
    assert!(
        !notifications.contains("admin-only notice"),
        "{notifications}"
    );
    let hidden = &tool_results(&bodies[3])[0];
    assert_eq!(hidden["is_error"], true);
    assert!(hidden["content"].as_str().unwrap().contains("No app named"));
}

#[tokio::test]
async fn conversations_belong_to_their_owner() {
    let (url, seen) = stub_api(Arc::new(|_, _| text_reply("hi"))).await;
    let h = harness(&url).await;
    grant(&h.db, "viewer", "assistant.use").await;
    let admin = h.user("admin").await;
    let viewer = h.user("viewer").await;
    let conversation = h.conversation(&admin).await;
    let uri = format!("/api/assistant/conversations/{conversation}");

    assert_eq!(
        h.call(&viewer, "GET", &uri, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.call(&viewer, "DELETE", &uri, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.ask(&viewer, &conversation, "steal").await.0,
        StatusCode::NOT_FOUND
    );
    assert!(seen.bodies().is_empty());
    let (_, list) = h
        .call(&viewer, "GET", "/api/assistant/conversations", None)
        .await;
    assert_eq!(serde_json::from_str::<Value>(&list).unwrap(), json!([]));
    let (_, list) = h
        .call(&admin, "GET", "/api/assistant/conversations", None)
        .await;
    assert_eq!(
        serde_json::from_str::<Value>(&list).unwrap()[0]["id"],
        conversation.as_str()
    );

    assert_eq!(
        h.call(&admin, "DELETE", &uri, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.call(&admin, "GET", &uri, None).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn users_without_assistant_use_are_refused() {
    let (url, _) = stub_api(Arc::new(|_, _| text_reply("hi"))).await;
    let h = harness(&url).await;
    let viewer = h.user("viewer").await;
    let (status, _) = h
        .call(
            &viewer,
            "POST",
            "/api/assistant/conversations",
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = h
        .call(&viewer, "GET", "/api/assistant/conversations", None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn questions_are_rate_limited_per_user() {
    let (url, seen) = stub_api(Arc::new(|_, _| text_reply("hi"))).await;
    let h = harness(&url).await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    for _ in 0..conversations::RATE_LIMIT {
        assistant_request::ActiveModel {
            user_id: Set(admin.id),
            created_at: Set(Utc::now()),
            ..Default::default()
        }
        .insert(&h.db)
        .await
        .unwrap();
    }
    let (status, _) = h.ask(&admin, &conversation, "one more").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(seen.bodies().is_empty());

    // Deleting conversations does not reset the limit.
    h.call(
        &admin,
        "DELETE",
        &format!("/api/assistant/conversations/{conversation}"),
        None,
    )
    .await;
    let conversation = h.conversation(&admin).await;
    assert_eq!(
        h.ask(&admin, &conversation, "again").await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn a_refusal_ends_with_a_polite_message() {
    let (url, _) = stub_api(Arc::new(|_, _| {
        let mut events = vec![start()];
        events.extend(stop("refusal"));
        sse(&events)
    }))
    .await;
    let h = harness(&url).await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    let (_, events) = h.ask(&admin, &conversation, "something bad").await;
    assert_eq!(names(&events), ["text_delta", "done"]);
    assert_eq!(events[0].1["text"], REFUSAL_TEXT);
}

#[tokio::test]
async fn an_api_error_is_reported_without_provider_text() {
    let (url, _) = stub_api(Arc::new(|_, _| {
        sse(&[
            start(),
            json!({"type": "error", "error": {"type": "overloaded_error", "message": "secret provider detail"}}),
        ])
    }))
    .await;
    let h = harness(&url).await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    let (_, events) = h.ask(&admin, &conversation, "hi").await;
    let (name, data) = events.last().unwrap();
    assert_eq!(name, "error");
    assert!(!data["message"]
        .as_str()
        .unwrap()
        .contains("secret provider detail"));
    let stored = assistant_message::Entity::find()
        .filter(assistant_message::Column::Role.eq("assistant"))
        .one(&h.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "error");
}

#[tokio::test]
async fn a_broken_stream_saves_the_partial_answer_as_interrupted() {
    // Far more text than the response pipe buffers, so the turn is still
    // writing when the browser goes away.
    let (url, _) = stub_api(Arc::new(|_, _| {
        let mut events = vec![
            start(),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        ];
        for _ in 0..3_000 {
            events.push(json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": "lorem ipsum dolor sit amet "}}));
        }
        events.push(json!({"type": "content_block_stop", "index": 0}));
        events.extend(stop("end_turn"));
        sse(&events)
    }))
    .await;
    let h = harness(&url).await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    let response = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/assistant/conversations/{conversation}/messages"
                ))
                .method("POST")
                .header("Cookie", &admin.cookie)
                .header("content-type", "application/json")
                .body(Body::from(json!({"text": "Tell me a lot"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let first = body.frame().await.unwrap().unwrap();
    assert!(first.is_data());
    drop(body);

    let mut stored = None;
    for _ in 0..200 {
        let message = assistant_message::Entity::find()
            .filter(assistant_message::Column::Role.eq("assistant"))
            .one(&h.db)
            .await
            .unwrap()
            .unwrap();
        if message.status == "interrupted" {
            stored = Some(message);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let stored = stored.expect("the answer must be marked interrupted");
    assert!(stored.content_json.contains("lorem ipsum"));
    let usage = assistant_usage::Entity::find().all(&h.db).await.unwrap();
    assert_eq!(usage.len(), 1, "usage of the broken call is still recorded");
    // No final count arrived: output is estimated from the streamed text,
    // not left at message_start's 1 token.
    assert!(usage[0].output_tokens > 1, "{}", usage[0].output_tokens);
}

#[tokio::test]
async fn retention_deletes_old_conversations_only() {
    let db = create_test_db_with_seed().await;
    let user =
        create_test_user_with_role(&db, "keeper", "keeper@example.com", "pw-123456", "admin").await;
    let old = conversations::create(&db, user.id, Some("old"), &Default::default())
        .await
        .unwrap();
    let fresh = conversations::create(&db, user.id, Some("fresh"), &Default::default())
        .await
        .unwrap();
    conversations::start_turn(&db, &old, "q").await.unwrap();
    let mut active: assistant_conversation::ActiveModel = old.clone().into();
    active.updated_at = Set(Utc::now() - Duration::days(31));
    active.update(&db).await.unwrap();

    let deleted = conversations::delete_expired(&db, 30, Utc::now())
        .await
        .unwrap();
    assert_eq!(deleted, 1);
    let left: Vec<String> = assistant_conversation::Entity::find()
        .all(&db)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(left, vec![fresh.id]);
    let orphans = assistant_message::Entity::find()
        .filter(assistant_message::Column::ConversationId.eq(old.id))
        .all(&db)
        .await
        .unwrap();
    assert!(orphans.is_empty());
}

#[tokio::test]
async fn invalid_questions_are_rejected() {
    let (url, _) = stub_api(Arc::new(|_, _| text_reply("hi"))).await;
    let h = harness(&url).await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    assert_eq!(
        h.ask(&admin, &conversation, "   ").await.0,
        StatusCode::BAD_REQUEST
    );
    let long = "x".repeat(conversations::MAX_QUESTION_CHARS + 1);
    assert_eq!(
        h.ask(&admin, &conversation, &long).await.0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn long_conversations_send_the_last_20_turns_and_a_running_summary() {
    let (url, seen) = stub_api(Arc::new(|_, body| {
        if body.get("stream").is_none() {
            json!({"id": "msg_s", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
                "stop_reason": "end_turn", "content": [{"type": "text", "text": "Earlier: asked about q0 and q1."}],
                "usage": {"input_tokens": 30, "output_tokens": 10}})
            .to_string()
        } else {
            text_reply("ok")
        }
    }))
    .await;
    let h = harness(&url).await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    let model = conversations::owned(&h.db, admin.id, &conversation)
        .await
        .unwrap();
    for i in 0..22 {
        let (_, answer) = conversations::start_turn(&h.db, &model, &format!("q{i}"))
            .await
            .unwrap();
        conversations::save_answer(
            &h.db,
            answer,
            &conversations::MessageContent {
                text: format!("a{i}"),
                ..Default::default()
            },
            "complete",
        )
        .await
        .unwrap();
    }

    let (_, events) = h.ask(&admin, &conversation, "latest").await;
    assert_eq!(names(&events), ["status", "text_delta", "done"]);
    let bodies = seen.bodies();
    assert_eq!(bodies.len(), 2);
    let summary_request = bodies[0]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(summary_request.contains("User: q0") && summary_request.contains("User: q1"));
    assert!(!summary_request.contains("User: q2\n"));
    let messages = bodies[1]["messages"].as_array().unwrap();
    assert!(messages[0]["content"]
        .as_str()
        .unwrap()
        .contains("Earlier: asked about q0 and q1."));
    assert_eq!(messages[1], json!({"role": "user", "content": "q2"}));
    // summary + 20 turns x 2 + question + context
    assert_eq!(messages.len(), 1 + 40 + 2);
    let stored = conversations::owned(&h.db, admin.id, &conversation)
        .await
        .unwrap();
    assert!(stored.summary.unwrap().contains("q0"));

    // The next question reuses the stored summary without a new summary call
    // until another turn falls out of the window.
    h.ask(&admin, &conversation, "again").await;
    let bodies = seen.bodies();
    assert_eq!(bodies.len(), 4, "one more turn fell out: summary + answer");
}

#[tokio::test]
async fn usage_is_priced_by_the_model_that_answered() {
    // A server-side fallback answers with another model than requested.
    let (url, _) = stub_api(Arc::new(|_, _| {
        text_reply("ok").replace(
            "\"model\":\"claude-opus-5-5\"",
            "\"model\":\"claude-sonnet-5-5\"",
        )
    }))
    .await;
    let h = harness(&url).await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    h.ask(&admin, &conversation, "hi").await;
    let usage = assistant_usage::Entity::find().all(&h.db).await.unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].model, "claude-sonnet-5-5");
    // 100 input, 20 output, 50 cache-read and 10 cache-write tokens at Sonnet
    // 5.5 prices (nano-dollars per token), rounded up to micro-dollars.
    let expected = (100 * 2_000 + 20 * 10_000 + 50 * 100 + 10 * 2_500u64).div_ceil(1_000);
    assert_eq!(usage[0].cost_usd_micros, expected as i64);
}

#[tokio::test]
async fn non_admins_cannot_read_logs_or_metrics_of_system_or_shared_namespaces() {
    let (url, seen) = stub_api(Arc::new(|n, _| match n {
        0 => tool_reply("toolu_1", "query_logs", &json!({"app": "victorialogs"})),
        1 => tool_reply("toolu_2", "query_logs", &json!({"app": "shared-a"})),
        2 => tool_reply(
            "toolu_3",
            "query_metrics",
            &json!({"app": "shared-a", "metric": "cpu"}),
        ),
        3 => tool_reply(
            "toolu_4",
            "query_metrics",
            &json!({"app": "jellyfin", "metric": "cpu"}),
        ),
        _ => text_reply("Done."),
    }))
    .await;
    let h = harness(&url).await;
    for (app, namespace) in [
        ("victorialogs", "victorialogs"),
        ("victoriametrics", "victoriametrics"),
        ("shared-a", "media"),
        ("shared-b", "media"),
    ] {
        app_state::ActiveModel {
            app_name: Set(app.into()),
            namespace: Set(namespace.into()),
            desired_state: Set("installed".into()),
            observed_state: Set("installed".into()),
            healthy: Set(true),
            message: Set(None),
            installed_chart_version: Set(None),
            available_chart_version: Set(None),
            update_available: Set(false),
            auto_update: Set(false),
            last_operation_id: Set(None),
            last_checked_at: Set(None),
            updated_at: Set(Utc::now()),
        }
        .insert(&h.db)
        .await
        .unwrap();
    }
    grant(&h.db, "viewer", "assistant.use").await;
    grant(&h.db, "viewer", "app.*").await;
    let viewer = h.user("viewer").await;
    let conversation = h.conversation(&viewer).await;

    let (_, events) = h.ask(&viewer, &conversation, "Show me logs").await;
    assert_eq!(names(&events).last(), Some(&"done"));
    let bodies = seen.bodies();
    let offered = tool_names(&bodies[0]);
    assert!(offered.contains(&"query_logs".to_string()));
    assert!(offered.contains(&"query_metrics".to_string()));
    for (i, what) in [
        (1, "system namespace logs"),
        (2, "shared namespace logs"),
        (3, "shared namespace metrics"),
    ] {
        let result = &tool_results(&bodies[i])[0];
        assert_eq!(result["is_error"], true, "{what}");
        assert!(
            result["content"]
                .as_str()
                .unwrap()
                .contains("only available to administrators"),
            "{what}: {result}"
        );
    }
    // The viewer's own single-app namespace is allowed (VictoriaMetrics is
    // unreachable in tests, so the answer is "no data", not a denial).
    let own = &tool_results(&bodies[4])[0];
    assert!(own.get("is_error").is_none(), "{own}");
    assert!(own["content"].as_str().unwrap().contains("\"points\":0"));

    let audits = audit_log::Entity::find()
        .filter(audit_log::Column::Action.eq("assistant_tool_call"))
        .filter(audit_log::Column::Success.eq(false))
        .all(&h.db)
        .await
        .unwrap();
    assert_eq!(audits.len(), 3);
}

#[tokio::test]
async fn admins_can_read_logs_of_system_namespaces() {
    let (url, seen) = stub_api(Arc::new(|n, _| match n {
        0 => tool_reply(
            "toolu_1",
            "query_metrics",
            &json!({"app": "victoriametrics", "metric": "memory"}),
        ),
        _ => text_reply("Done."),
    }))
    .await;
    let h = harness(&url).await;
    app_state::ActiveModel {
        app_name: Set("victoriametrics".into()),
        namespace: Set("victoriametrics".into()),
        desired_state: Set("installed".into()),
        observed_state: Set("installed".into()),
        healthy: Set(true),
        message: Set(None),
        installed_chart_version: Set(None),
        available_chart_version: Set(None),
        update_available: Set(false),
        auto_update: Set(false),
        last_operation_id: Set(None),
        last_checked_at: Set(None),
        updated_at: Set(Utc::now()),
    }
    .insert(&h.db)
    .await
    .unwrap();
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    h.ask(&admin, &conversation, "memory?").await;
    let result = &tool_results(&seen.bodies()[1])[0];
    assert!(result.get("is_error").is_none(), "{result}");
}

#[tokio::test]
async fn parallel_questions_cannot_exceed_the_rate_limit() {
    let db = create_test_db_with_seed().await;
    let user =
        create_test_user_with_role(&db, "racer", "racer@example.com", "pw-123456", "admin").await;
    for _ in 0..conversations::RATE_LIMIT - 1 {
        assistant_request::ActiveModel {
            user_id: Set(user.id),
            created_at: Set(Utc::now()),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();
    }
    let attempts = (0..5).map(|_| {
        let db = db.clone();
        tokio::spawn(async move { conversations::take_rate_slot(&db, user.id).await.is_ok() })
    });
    let mut granted = 0;
    for attempt in attempts {
        if attempt.await.unwrap() {
            granted += 1;
        }
    }
    assert_eq!(granted, 1);
}
