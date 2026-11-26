//! Assistant actions: `propose_action` validates and stores a card but never
//! acts; only `POST /api/assistant/actions/{id}/run` (owner only, once,
//! before expiry, re-validated) queues the work. Also cancel, audit rows,
//! the outcome message and parity with the app endpoints' rules.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use chrono::{Duration, Utc};
use http_body_util::BodyExt;
use parking_lot::Mutex;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set,
};
use serde_json::{json, Value};
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

use kubarr::endpoints::create_router;
use kubarr::models::{
    app_operation, app_state, assistant_message, assistant_pending_action, audit_log, role,
    role_app_permission, role_permission,
};
use kubarr::services::assistant::actions::{self, ActionKind, ActionParams, Actor, MAJOR_REFUSAL};
use kubarr::services::assistant::{AssistantDeps, MemoryKeyStore, MessagesClient};
use kubarr::services::catalog::{AppCatalog, AppConfig, ResourceRequirements};
use kubarr::state::AppState;

static JWT_INIT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
static CHARTS: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();

/// Fake charts on disk (catalog versions), set before CONFIG is first read.
fn chart_fixture() {
    CHARTS.get_or_init(|| {
        let charts = tempfile::tempdir().unwrap();
        for (name, version) in [
            ("jellyfin", "1.1.0"),
            ("sonarr", "2.0.0"),
            ("radarr", "1.0.0"),
            ("lidarr", "1.2.3"),
            ("bazarr", "1.1.0"),
            ("kubarr-dashboard", "1.0.0"),
        ] {
            let dir = charts.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(
                dir.join("Chart.yaml"),
                format!("name: {name}\nversion: {version}\n"),
            )
            .unwrap();
            std::fs::write(dir.join("values.yaml"), "storage:\n  mounts: []\n").unwrap();
        }
        std::env::set_var("KUBARR_CHARTS_DIR", charts.path());
        charts
    });
}

async fn ensure_jwt_keys() {
    chart_fixture();
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

/// Answers request `n` with `script(n, body)`; records the bodies.
async fn stub_api(script: Script) -> (String, Arc<Mutex<Vec<Value>>>) {
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let recorder = seen.clone();
    let app = Router::new().route(
        "/v1/messages",
        post(move |Json(body): Json<Value>| {
            let recorder = recorder.clone();
            let script = script.clone();
            async move {
                let n = {
                    let mut requests = recorder.lock();
                    requests.push(body.clone());
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
        "usage": {"input_tokens": 10, "output_tokens": 1}}})
}

fn stop(reason: &str) -> Vec<Value> {
    vec![
        json!({"type": "message_delta", "delta": {"stop_reason": reason}, "usage": {"output_tokens": 5}}),
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

fn tool_reply(id: &str, name: &str, input: &Value) -> String {
    let mut events = vec![
        start(),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}),
        json!({"type": "content_block_stop", "index": 0}),
    ];
    events.extend(stop("tool_use"));
    sse(&events)
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    app: Router,
    state: AppState,
    db: DatabaseConnection,
}

struct User {
    id: i64,
    name: String,
    cookie: String,
    password: String,
}

fn app_config(name: &str, is_system: bool) -> AppConfig {
    AppConfig {
        name: name.to_string(),
        display_name: name.to_string(),
        description: "Test app".to_string(),
        icon: String::new(),
        container_image: format!("test/{name}:latest"),
        default_port: 8080,
        resource_requirements: ResourceRequirements {
            cpu_request: "100m".to_string(),
            cpu_limit: "500m".to_string(),
            memory_request: "128Mi".to_string(),
            memory_limit: "512Mi".to_string(),
        },
        volumes: vec![],
        environment_variables: HashMap::new(),
        category: "media".to_string(),
        is_system,
        is_hidden: false,
        is_browseable: true,
    }
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

async fn installed(db: &DatabaseConnection, app: &str, from: &str, to: &str) {
    app_state::ActiveModel {
        app_name: Set(app.into()),
        namespace: Set(app.into()),
        desired_state: Set("installed".into()),
        observed_state: Set("installed".into()),
        healthy: Set(true),
        message: Set(None),
        installed_chart_version: Set(Some(from.into())),
        available_chart_version: Set(Some(to.into())),
        update_available: Set(from != to),
        auto_update: Set(true),
        last_operation_id: Set(None),
        last_checked_at: Set(None),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .unwrap();
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
    };
    *state.catalog.write().await = AppCatalog::with_apps(HashMap::from([
        ("jellyfin".to_string(), app_config("jellyfin", false)),
        ("sonarr".to_string(), app_config("sonarr", false)),
        ("radarr".to_string(), app_config("radarr", false)),
        ("lidarr".to_string(), app_config("lidarr", false)),
        ("bazarr".to_string(), app_config("bazarr", false)),
        (
            "kubarr-dashboard".to_string(),
            app_config("kubarr-dashboard", true),
        ),
    ]));
    installed(&db, "jellyfin", "1.0.0", "1.1.0").await;
    installed(&db, "sonarr", "1.0.0", "2.0.0").await;
    installed(&db, "kubarr-dashboard", "1.0.0", "1.0.0").await;
    Harness {
        app: create_router(state.clone()),
        state,
        db,
    }
}

async fn make_role(db: &DatabaseConnection, name: &str, permissions: &[&str]) {
    let role = role::ActiveModel {
        name: Set(name.into()),
        description: Set(None),
        is_system: Set(false),
        requires_2fa: Set(false),
        created_at: Set(Utc::now()),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();
    for permission in permissions {
        role_permission::ActiveModel {
            role_id: Set(role.id),
            permission: Set((*permission).into()),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
    }
}

async fn revoke(db: &DatabaseConnection, role_name: &str, permission: &str) {
    let role = role::Entity::find()
        .filter(role::Column::Name.eq(role_name))
        .one(db)
        .await
        .unwrap()
        .unwrap();
    role_permission::Entity::delete_many()
        .filter(role_permission::Column::RoleId.eq(role.id))
        .filter(role_permission::Column::Permission.eq(permission))
        .exec(db)
        .await
        .unwrap();
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
            name,
            password,
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

    async fn ask(&self, user: &User, conversation: &str, text: &str) -> Vec<(String, Value)> {
        let (status, body) = self
            .call(
                user,
                "POST",
                &format!("/api/assistant/conversations/{conversation}/messages"),
                Some(json!({ "text": text })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        parse_events(&body)
    }

    async fn detail(&self, user: &User, conversation: &str) -> Value {
        let (status, body) = self
            .call(
                user,
                "GET",
                &format!("/api/assistant/conversations/{conversation}"),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        serde_json::from_str(&body).unwrap()
    }

    async fn run(&self, user: &User, id: &str) -> (StatusCode, Value) {
        let (status, body) = self
            .call(
                user,
                "POST",
                &format!("/api/assistant/actions/{id}/run"),
                None,
            )
            .await;
        (status, serde_json::from_str(&body).unwrap_or(Value::Null))
    }

    async fn cancel(&self, user: &User, id: &str) -> StatusCode {
        self.call(
            user,
            "POST",
            &format!("/api/assistant/actions/{id}/cancel"),
            None,
        )
        .await
        .0
    }

    async fn operations(&self) -> u64 {
        app_operation::Entity::find().count(&self.db).await.unwrap()
    }

    async fn audits(&self, action: &str) -> Vec<audit_log::Model> {
        audit_log::Entity::find()
            .filter(audit_log::Column::Action.eq(action))
            .all(&self.db)
            .await
            .unwrap()
    }

    async fn actor(&self, user: &User) -> Actor {
        Actor {
            user_id: user.id,
            username: user.name.clone(),
            permissions: kubarr::endpoints::extractors::get_user_permissions(&self.db, user.id)
                .await,
        }
    }

    async fn plan(
        &self,
        user: &User,
        kind: ActionKind,
        app: &str,
        params: ActionParams,
    ) -> Result<actions::Plan, String> {
        actions::plan(
            &self.state,
            &self.db,
            &self.actor(user).await,
            kind,
            app,
            &params,
        )
        .await
        .map_err(|e| actions::refusal_text(&e))
    }

    /// Store a pending action through the propose tool path, without the
    /// model, and return its id.
    async fn propose(
        &self,
        user: &User,
        conversation: &str,
        kind: &str,
        app: &str,
        params: Value,
    ) -> String {
        let ctx = kubarr::services::assistant::tools::ToolContext {
            state: self.state.clone(),
            db: self.db.clone(),
            user_id: user.id,
            username: user.name.clone(),
            permissions: self.actor(user).await.permissions,
            conversation_id: conversation.to_string(),
            monitoring: Default::default(),
            cluster: false,
            message_id: None,
        };
        let outcome = kubarr::services::assistant::tools::execute(
            &ctx,
            "propose_action",
            Some(&json!({"kind": kind, "app": app, "params": params, "reason": "test"})),
        )
        .await;
        assert!(!outcome.is_error, "{}", outcome.content);
        outcome.card.expect("a card").id
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_proposal_shows_a_card_and_only_run_queues_the_operation() {
    // The model proposes a restart and then (wrongly) claims it is done; a
    // second turn tries to propose again. Nothing runs without the click.
    let (url, seen) = stub_api(Arc::new(|n, _| match n {
        0 => tool_reply(
            "toolu_1",
            "propose_action",
            &json!({"kind": "restart_app", "app": "jellyfin", "params": {}, "reason": "It is stuck"}),
        ),
        _ => text_reply("I restarted Jellyfin for you."),
    }))
    .await;
    let h = harness(&url).await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;

    let events = h
        .ask(&admin, &conversation, "Jellyfin is stuck, restart it")
        .await;
    let names: Vec<&str> = events.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["status", "action_card", "text_delta", "sources", "done"]
    );
    let card = &events[1].1;
    assert_eq!(card["kind"], "restart_app");
    assert_eq!(card["app"], "jellyfin");
    assert_eq!(card["status"], "pending");
    assert_eq!(card["summary"]["title"], "Restart jellyfin");
    assert!(card["expires_at"].is_string());
    let id = card["id"].as_str().unwrap().to_string();

    // The tool result tells the model nothing happened yet.
    let bodies = seen.lock().clone();
    assert!(bodies[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "propose_action"));
    let result = bodies[1]["messages"].as_array().unwrap().last().unwrap()["content"][0].clone();
    assert!(result.get("is_error").is_none());
    assert!(result["content"]
        .as_str()
        .unwrap()
        .contains("Nothing has happened yet"));
    assert!(bodies[0]["system"][0]["text"]
        .as_str()
        .unwrap()
        .contains("only their click runs it"));

    // Proposed, audited, attached to the answer; nothing queued.
    assert_eq!(h.operations().await, 0);
    let proposed = h.audits("assistant_action_proposed").await;
    assert_eq!(proposed.len(), 1);
    assert!(proposed[0].success);
    let detail = h.detail(&admin, &conversation).await;
    let cards = detail["actions"].as_array().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["status"], "pending");
    assert_eq!(cards[0]["message_id"], detail["messages"][1]["id"]);

    // Another turn: still nothing runs.
    h.ask(&admin, &conversation, "Did it work?").await;
    assert_eq!(h.operations().await, 0);

    // The click runs it through the operation queue.
    let (status, body) = h.run(&admin, &id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "running");
    let operation_id = body["operation_id"].as_str().unwrap().to_string();
    let operation = app_operation::Entity::find_by_id(operation_id.clone())
        .one(&h.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(operation.operation, "restart");
    assert_eq!(operation.app_name, "jellyfin");
    assert_eq!(operation.status, "queued");
    assert_eq!(operation.created_by, Some(admin.id));

    let confirmed = h.audits("assistant_action_confirmed").await;
    assert_eq!(confirmed.len(), 1);
    assert!(confirmed[0].success);
    let details: Value = serde_json::from_str(confirmed[0].details.as_deref().unwrap()).unwrap();
    assert_eq!(details["operation_id"], operation_id.as_str());
    assert_eq!(details["action_id"], id.as_str());

    // The outcome is in the conversation, and the card shows it.
    let detail = h.detail(&admin, &conversation).await;
    let last = detail["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(last["role"], "assistant");
    assert!(
        last["text"].as_str().unwrap().contains(&operation_id),
        "{last}"
    );
    assert_eq!(detail["actions"][0]["status"], "running");
    assert_eq!(detail["actions"][0]["operation_id"], operation_id.as_str());

    // The finished operation decides the card's outcome at read time.
    let mut active: app_operation::ActiveModel = operation.into();
    active.status = Set("succeeded".into());
    active.update(&h.db).await.unwrap();
    let detail = h.detail(&admin, &conversation).await;
    assert_eq!(detail["actions"][0]["status"], "done");

    // Single use.
    let (status, _) = h.run(&admin, &id).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(h.operations().await, 1);
}

#[tokio::test]
async fn two_concurrent_runs_queue_one_operation() {
    let h = harness("http://127.0.0.1:9").await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    let id = h
        .propose(&admin, &conversation, "restart_app", "jellyfin", json!({}))
        .await;

    let (a, b) = tokio::join!(h.run(&admin, &id), h.run(&admin, &id));
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::CONFLICT],
        "{a:?} {b:?}"
    );
    assert_eq!(h.operations().await, 1);
    assert_eq!(h.audits("assistant_action_confirmed").await.len(), 1);
}

#[tokio::test]
async fn expired_actions_cannot_run() {
    let h = harness("http://127.0.0.1:9").await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    let id = h
        .propose(&admin, &conversation, "restart_app", "jellyfin", json!({}))
        .await;
    let stored = assistant_pending_action::Entity::find_by_id(id.clone())
        .one(&h.db)
        .await
        .unwrap()
        .unwrap();
    assert!(stored.expires_at > Utc::now() + Duration::minutes(9));
    assert!(stored.expires_at <= Utc::now() + Duration::minutes(10));
    let mut active: assistant_pending_action::ActiveModel = stored.into();
    active.expires_at = Set(Utc::now() - Duration::seconds(1));
    active.update(&h.db).await.unwrap();

    let (status, body) = h.run(&admin, &id).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("expired"),
        "{body}"
    );
    assert_eq!(h.operations().await, 0);
    let detail = h.detail(&admin, &conversation).await;
    assert_eq!(detail["actions"][0]["status"], "expired");
}

#[tokio::test]
async fn only_the_owner_can_run_or_cancel() {
    let h = harness("http://127.0.0.1:9").await;
    let owner = h.user("admin").await;
    let other = h.user("admin").await;
    let conversation = h.conversation(&owner).await;
    let id = h
        .propose(&owner, &conversation, "restart_app", "jellyfin", json!({}))
        .await;

    assert_eq!(h.run(&other, &id).await.0, StatusCode::NOT_FOUND);
    assert_eq!(h.cancel(&other, &id).await, StatusCode::NOT_FOUND);
    assert_eq!(
        h.run(&owner, "no-such-action").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(h.operations().await, 0);

    // Another session of the owner may run it.
    let second = User {
        id: owner.id,
        name: owner.name.clone(),
        cookie: login(h.app.clone(), &owner.name, &owner.password).await,
        password: owner.password.clone(),
    };
    assert_ne!(second.cookie, owner.cookie);
    assert_eq!(h.run(&second, &id).await.0, StatusCode::OK);
    assert_eq!(h.operations().await, 1);
}

#[tokio::test]
async fn cancel_marks_the_action_and_blocks_run() {
    let h = harness("http://127.0.0.1:9").await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    let id = h
        .propose(&admin, &conversation, "restart_app", "jellyfin", json!({}))
        .await;

    assert_eq!(h.cancel(&admin, &id).await, StatusCode::NO_CONTENT);
    assert_eq!(h.cancel(&admin, &id).await, StatusCode::CONFLICT);
    assert_eq!(h.run(&admin, &id).await.0, StatusCode::CONFLICT);
    assert_eq!(h.operations().await, 0);
    let cancelled = h.audits("assistant_action_cancelled").await;
    assert_eq!(cancelled.len(), 1);
    let details: Value = serde_json::from_str(cancelled[0].details.as_deref().unwrap()).unwrap();
    assert_eq!(details["action_id"], id.as_str());
    assert_eq!(details["target"], "jellyfin");
    let detail = h.detail(&admin, &conversation).await;
    assert_eq!(detail["actions"][0]["status"], "cancelled");
}

#[tokio::test]
async fn permissions_are_checked_again_at_run_time() {
    let h = harness("http://127.0.0.1:9").await;
    make_role(
        &h.db,
        "operator",
        &["assistant.use", "apps.view", "apps.restart", "app.*"],
    )
    .await;
    let operator = h.user("operator").await;
    let conversation = h.conversation(&operator).await;
    let id = h
        .propose(
            &operator,
            &conversation,
            "restart_app",
            "jellyfin",
            json!({}),
        )
        .await;

    revoke(&h.db, "operator", "apps.restart").await;
    let (status, body) = h.run(&operator, &id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(h.operations().await, 0);

    let stored = assistant_pending_action::Entity::find_by_id(id.clone())
        .one(&h.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, "failed");
    let confirmed = h.audits("assistant_action_confirmed").await;
    assert_eq!(confirmed.len(), 1);
    assert!(!confirmed[0].success);
    let last = assistant_message::Entity::find()
        .filter(assistant_message::Column::ConversationId.eq(conversation.clone()))
        .all(&h.db)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert!(
        last.content_json.contains("did not run"),
        "{}",
        last.content_json
    );
    // Refused once; not runnable again.
    assert_eq!(h.run(&operator, &id).await.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_newer_available_version_invalidates_a_proposed_update() {
    let h = harness("http://127.0.0.1:9").await;
    installed(&h.db, "bazarr", "1.0.0", "1.1.0").await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    let id = h
        .propose(&admin, &conversation, "update_app", "bazarr", json!({}))
        .await;
    // A newer chart arrives before the click (only this test uses bazarr).
    let chart = CHARTS
        .get()
        .unwrap()
        .path()
        .join("bazarr")
        .join("Chart.yaml");
    std::fs::write(chart, "name: bazarr\nversion: 1.2.0\n").unwrap();

    let (status, body) = h.run(&admin, &id).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["detail"].as_str().unwrap().contains("1.2.0"), "{body}");
    assert_eq!(h.operations().await, 0);
}

#[tokio::test]
async fn an_update_queues_the_proposed_minor_version() {
    let h = harness("http://127.0.0.1:9").await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    let id = h
        .propose(
            &admin,
            &conversation,
            "update_app",
            "jellyfin",
            json!({"version": "1.1.0"}),
        )
        .await;
    let card = &h.detail(&admin, &conversation).await["actions"][0];
    assert_eq!(card["summary"]["from_version"], "1.0.0");
    assert_eq!(card["summary"]["to_version"], "1.1.0");

    let (status, body) = h.run(&admin, &id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let operation =
        app_operation::Entity::find_by_id(body["operation_id"].as_str().unwrap().to_string())
            .one(&h.db)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(operation.operation, "update");
    let config = operation.custom_config.unwrap();
    assert!(config.contains("1.1.0"), "{config}");
}

#[tokio::test]
async fn auto_update_toggle_is_done_at_once() {
    let h = harness("http://127.0.0.1:9").await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    let id = h
        .propose(
            &admin,
            &conversation,
            "set_app_auto_update",
            "jellyfin",
            json!({"enabled": false}),
        )
        .await;
    let (status, body) = h.run(&admin, &id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "done");
    assert!(body.get("operation_id").is_none());
    let state = app_state::Entity::find_by_id("jellyfin".to_string())
        .one(&h.db)
        .await
        .unwrap()
        .unwrap();
    assert!(!state.auto_update);
    assert_eq!(
        h.detail(&admin, &conversation).await["actions"][0]["status"],
        "done"
    );
}

#[tokio::test]
async fn validation_matches_the_app_endpoints() {
    let h = harness("http://127.0.0.1:9").await;
    grant(&h.db, "viewer", "assistant.use").await;
    grant(&h.db, "downloader", "assistant.use").await;
    let admin = h.user("admin").await;
    let viewer = h.user("viewer").await;
    let downloader = h.user("downloader").await;
    let none = ActionParams::default;

    // A viewer may not restart or update; neither may they through the API.
    let refused = h
        .plan(&viewer, ActionKind::RestartApp, "jellyfin", none())
        .await;
    assert!(refused.unwrap_err().contains("apps.restart"));
    assert_eq!(
        h.call(&viewer, "POST", "/api/apps/jellyfin/restart", None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // ... and is not even offered the tool.
    let ctx_tools = kubarr::services::assistant::tools::ToolContext {
        state: h.state.clone(),
        db: h.db.clone(),
        user_id: viewer.id,
        username: viewer.name.clone(),
        permissions: h.actor(&viewer).await.permissions,
        conversation_id: String::new(),
        monitoring: Default::default(),
        cluster: false,
        message_id: None,
    }
    .tools();
    assert!(!ctx_tools.iter().any(|t| t.name() == "propose_action"));

    // A downloader may restart, but only their own apps, and may not update.
    let refused = h
        .plan(&downloader, ActionKind::RestartApp, "jellyfin", none())
        .await;
    assert!(refused.unwrap_err().contains("visible"));
    let refused = h
        .plan(&downloader, ActionKind::UpdateApp, "jellyfin", none())
        .await;
    assert!(refused.unwrap_err().contains("apps.install"));
    assert_eq!(
        h.call(&downloader, "POST", "/api/apps/jellyfin/update", None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let role = role::Entity::find()
        .filter(role::Column::Name.eq("downloader"))
        .one(&h.db)
        .await
        .unwrap()
        .unwrap();
    role_app_permission::ActiveModel {
        role_id: Set(role.id),
        app_name: Set("radarr".into()),
        ..Default::default()
    }
    .insert(&h.db)
    .await
    .unwrap();
    installed(&h.db, "radarr", "1.0.0", "1.0.0").await;
    assert!(h
        .plan(&downloader, ActionKind::RestartApp, "radarr", none())
        .await
        .is_ok());

    // Majors always go through the app page.
    let refused = h
        .plan(&admin, ActionKind::UpdateApp, "sonarr", none())
        .await;
    assert_eq!(refused.unwrap_err(), MAJOR_REFUSAL);
    // Only the available version.
    let refused = h
        .plan(
            &admin,
            ActionKind::UpdateApp,
            "jellyfin",
            ActionParams {
                version: Some("9.9.9".into()),
                ..Default::default()
            },
        )
        .await;
    assert!(refused.unwrap_err().contains("available version 1.1.0"));
    let planned = h
        .plan(&admin, ActionKind::UpdateApp, "jellyfin", none())
        .await
        .unwrap();
    assert_eq!(planned.params.version.as_deref(), Some("1.1.0"));
    // No update available.
    let refused = h
        .plan(&admin, ActionKind::UpdateApp, "kubarr-dashboard", none())
        .await;
    assert!(refused.unwrap_err().contains("no update available"));

    // Platform rules: system apps are not installed from the catalog or
    // rolled back from the assistant; unknown apps are refused.
    let refused = h
        .plan(&admin, ActionKind::InstallApp, "kubarr-dashboard", none())
        .await;
    assert!(refused.unwrap_err().contains("already installed"));
    app_state::Entity::delete_by_id("kubarr-dashboard".to_string())
        .exec(&h.db)
        .await
        .unwrap();
    let refused = h
        .plan(&admin, ActionKind::InstallApp, "kubarr-dashboard", none())
        .await;
    assert!(refused.unwrap_err().contains("part of Kubarr"));
    let refused = h
        .plan(&admin, ActionKind::RollbackApp, "kubarr-dashboard", none())
        .await;
    assert!(refused.unwrap_err().contains("part of Kubarr"));
    let refused = h.plan(&admin, ActionKind::RestartApp, "plex", none()).await;
    assert!(refused.unwrap_err().contains("catalog"));
    let refused = h
        .plan(&admin, ActionKind::RestartApp, "Not An App", none())
        .await;
    assert!(refused.is_err());

    // Rollback needs a successful update to undo.
    let refused = h
        .plan(&admin, ActionKind::RollbackApp, "jellyfin", none())
        .await;
    assert!(refused.unwrap_err().contains("nothing to roll back"));
    let now = Utc::now();
    let op = |id: &str, operation: &str, status: &str, message: &str| app_operation::ActiveModel {
        id: Set(id.into()),
        app_name: Set("jellyfin".into()),
        operation: Set(operation.into()),
        status: Set(status.into()),
        stop_requested: Set(false),
        message: Set(Some(message.into())),
        error: Set(None),
        custom_config: Set(Some("{}".into())),
        attempts: Set(1),
        created_by: Set(Some(admin.id)),
        trigger: Set(Some("manual".into())),
        diagnostics: Set(None),
        created_at: Set(now),
        started_at: Set(None),
        finished_at: Set(Some(now)),
        updated_at: Set(now),
    };
    op(
        "op-update",
        "update",
        "succeeded",
        "Updated jellyfin 0.9.0 → 1.0.0",
    )
    .insert(&h.db)
    .await
    .unwrap();
    let planned = h
        .plan(&admin, ActionKind::RollbackApp, "jellyfin", none())
        .await
        .unwrap();
    assert_eq!(planned.summary.from_version.as_deref(), Some("1.0.0"));
    assert_eq!(planned.summary.to_version.as_deref(), Some("0.9.0"));

    // Retry: failed operations only, never a removal, same permission as
    // the retry endpoint.
    let mut later = op("op-delete", "delete", "failed", "delete failed");
    later.created_at = Set(now + Duration::seconds(1));
    later.insert(&h.db).await.unwrap();
    let retry = |id: &str| ActionParams {
        operation_id: Some(id.into()),
        ..Default::default()
    };
    let refused = h
        .plan(
            &admin,
            ActionKind::RetryOperation,
            "jellyfin",
            retry("op-delete"),
        )
        .await;
    assert!(refused.unwrap_err().contains("does not uninstall"));
    let refused = h
        .plan(
            &admin,
            ActionKind::RetryOperation,
            "jellyfin",
            retry("op-update"),
        )
        .await;
    assert!(refused.unwrap_err().contains("Only failed operations"));
    let refused = h
        .plan(
            &admin,
            ActionKind::RetryOperation,
            "sonarr",
            retry("op-update"),
        )
        .await;
    assert!(refused.unwrap_err().contains("No operation"));
    let mut failed_restart = op("op-restart", "restart", "failed", "restart failed");
    failed_restart.created_at = Set(now + Duration::seconds(2));
    failed_restart.insert(&h.db).await.unwrap();
    make_role(
        &h.db,
        "restarter-less",
        &["assistant.use", "apps.install", "app.*"],
    )
    .await;
    let installer = h.user("restarter-less").await;
    let refused = h
        .plan(
            &installer,
            ActionKind::RetryOperation,
            "jellyfin",
            retry("op-restart"),
        )
        .await;
    assert!(refused.unwrap_err().contains("apps.restart"));
    assert_eq!(
        h.call(
            &installer,
            "POST",
            "/api/apps/operations/op-restart/retry",
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert!(h
        .plan(
            &admin,
            ActionKind::RetryOperation,
            "jellyfin",
            retry("op-restart")
        )
        .await
        .is_ok());

    // Auto-update toggle needs a change and apps.install.
    let refused = h
        .plan(
            &admin,
            ActionKind::SetAppAutoUpdate,
            "jellyfin",
            ActionParams {
                enabled: Some(true),
                ..Default::default()
            },
        )
        .await;
    assert!(refused.unwrap_err().contains("already on"));
    let refused = h
        .plan(&admin, ActionKind::SetAppAutoUpdate, "jellyfin", none())
        .await;
    assert!(refused.unwrap_err().contains("enabled"));

    // Refusals reach the model as tool errors and store nothing.
    let ctx = kubarr::services::assistant::tools::ToolContext {
        state: h.state.clone(),
        db: h.db.clone(),
        user_id: admin.id,
        username: admin.name.clone(),
        permissions: h.actor(&admin).await.permissions,
        conversation_id: h.conversation(&admin).await,
        monitoring: Default::default(),
        cluster: false,
        message_id: None,
    };
    let outcome = kubarr::services::assistant::tools::execute(
        &ctx,
        "propose_action",
        Some(&json!({"kind": "update_app", "app": "sonarr", "params": {}, "reason": "x"})),
    )
    .await;
    assert!(outcome.is_error);
    assert!(outcome
        .content
        .contains("Major updates go through the app page"));
    assert!(outcome.card.is_none());
    let outcome = kubarr::services::assistant::tools::execute(
        &ctx,
        "propose_action",
        Some(&json!({"kind": "delete_app", "app": "sonarr", "reason": "x"})),
    )
    .await;
    assert!(outcome.is_error);
    assert_eq!(
        assistant_pending_action::Entity::find()
            .count(&h.db)
            .await
            .unwrap(),
        0
    );
    let refusals = h.audits("assistant_action_proposed").await;
    assert_eq!(refusals.len(), 1);
    assert!(!refusals[0].success);
}

#[tokio::test]
async fn install_uses_catalog_defaults_through_the_wizard_path() {
    let h = harness("http://127.0.0.1:9").await;
    let admin = h.user("admin").await;
    let conversation = h.conversation(&admin).await;
    let id = h
        .propose(&admin, &conversation, "install_app", "lidarr", json!({}))
        .await;
    let card = &h.detail(&admin, &conversation).await["actions"][0];
    assert_eq!(card["summary"]["to_version"], "1.2.3");
    assert!(card["summary"]["effect"]
        .as_str()
        .unwrap()
        .contains("default settings"));

    let (status, body) = h.run(&admin, &id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let operation =
        app_operation::Entity::find_by_id(body["operation_id"].as_str().unwrap().to_string())
            .one(&h.db)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(operation.operation, "install");
    assert_eq!(operation.app_name, "lidarr");
    assert!(operation.custom_config.unwrap().contains("1.2.3"));

    // Installed (queued) now: a second proposal is refused.
    let refused = h
        .plan(
            &admin,
            ActionKind::InstallApp,
            "lidarr",
            ActionParams::default(),
        )
        .await;
    assert!(
        refused.unwrap_err().contains("already installed"),
        "install twice"
    );
}
