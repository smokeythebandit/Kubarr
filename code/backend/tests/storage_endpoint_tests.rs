//! Storage endpoint integration tests
//!
//! Covers endpoints under `/api/storage`:
//!
//! - `GET  /api/storage/browse`    — list directory contents (requires storage.view)
//! - `GET  /api/storage/stats`     — disk usage statistics (requires storage.view)
//! - `GET  /api/storage/file-info` — file/directory metadata (requires storage.view)
//! - `POST /api/storage/mkdir`     — create directory (requires storage.write)
//! - `DELETE /api/storage/delete`  — delete file or empty dir (requires storage.delete)
//! - `GET  /api/storage/download`  — stream file download (requires storage.download)
//!
//! Storage path is controlled by the `KUBARR_STORAGE_PATH` environment variable.
//! Each test that touches the filesystem creates its own unique temp directory and
//! sets that env var before calling the endpoint.  Because env vars are
//! process-global, a static Mutex serialises all storage tests.
//!
//! Auth checks:
//! - Unauthenticated access → 401
//! - Viewer role (has storage.view / storage.download but NOT storage.write or
//!   storage.delete) → 403 on write/delete endpoints
//! - Admin role (has all storage permissions) → success

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use once_cell::sync::Lazy;
use tokio::sync::Mutex;
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

use kubarr::endpoints::create_router;

// Serialise all storage tests that mutate KUBARR_STORAGE_PATH.
static STORAGE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

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

/// POST /auth/login and return (status, Set-Cookie header value).
async fn do_login(
    app: axum::Router,
    username: &str,
    password: &str,
) -> (StatusCode, Option<String>) {
    let body = serde_json::json!({
        "username": username,
        "password": password
    })
    .to_string();

    let request = Request::builder()
        .uri("/auth/login")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();

    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            if s.starts_with("kubarr_session=") && !s.contains("kubarr_session_") {
                Some(s.split(';').next().unwrap().to_string())
            } else {
                None
            }
        });

    (status, cookie)
}

/// Make an authenticated GET request and return (status, body_string).
async fn authenticated_get(app: axum::Router, uri: &str, cookie: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(uri)
        .method("GET")
        .header("Cookie", cookie)
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

/// Make an authenticated POST request and return (status, body_string).
async fn authenticated_post(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    json_body: &str,
) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(uri)
        .method("POST")
        .header("Cookie", cookie)
        .header("content-type", "application/json")
        .body(Body::from(json_body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

/// Make an authenticated DELETE request (query string on URI) and return (status, body_string).
async fn authenticated_delete(app: axum::Router, uri: &str, cookie: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(uri)
        .method("DELETE")
        .header("Cookie", cookie)
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

/// A simple RAII wrapper around a temp directory that deletes it on drop.
struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Create a unique temp directory for a single test.
fn make_temp_dir(test_name: &str) -> TempDir {
    let base = std::env::temp_dir();
    let dir_name = format!(
        "kubarr_storage_test_{}_{}",
        test_name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos()
    );
    let path = base.join(dir_name);
    std::fs::create_dir_all(&path).expect("Failed to create temp directory");
    TempDir { path }
}

// ============================================================================
// GET /api/storage/browse — requires storage.view (401/403 checks)
// ============================================================================

#[tokio::test]
async fn test_browse_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/storage/browse")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/storage/browse without auth must return 401"
    );
}

#[tokio::test]
async fn test_browse_root_as_admin() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("browse_root");
    // Create a subdirectory in the temp dir so browse has something to return
    std::fs::create_dir(tmp.path().join("movies")).unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagebrowseadmin",
        "storagebrowseadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagebrowseadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/storage/browse", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/storage/browse must return 200 for admin. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("path").is_some(),
        "Response must have 'path' field"
    );
    assert!(
        json.get("items").is_some(),
        "Response must have 'items' field"
    );
    assert!(
        json.get("total_items").is_some(),
        "Response must have 'total_items' field"
    );

    let items = json["items"].as_array().unwrap();
    assert!(
        !items.is_empty(),
        "Browse must return the 'movies' subdirectory we created"
    );

    let names: Vec<&str> = items.iter().filter_map(|i| i["name"].as_str()).collect();
    assert!(
        names.contains(&"movies"),
        "Items must include 'movies' directory. Got: {:?}",
        names
    );
}

#[tokio::test]
async fn test_browse_viewer_has_storage_view_returns_200() {
    // The viewer role has storage.view, so it can browse.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("browse_viewer");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagebrowseviewer",
        "storagebrowseviewer@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagebrowseviewer",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(create_router(state), "/api/storage/browse", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Viewer with storage.view must be able to browse"
    );
}

#[tokio::test]
async fn test_browse_nonexistent_path_returns_404() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("browse_notfound");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagenotfoundadmin",
        "storagenotfoundadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagenotfoundadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(
        create_router(state),
        "/api/storage/browse?path=this_path_does_not_exist_xyz",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Browsing a non-existent path must return 404"
    );
}

#[tokio::test]
async fn test_browse_path_traversal_returns_error() {
    // Attempting to browse outside the storage root via path traversal must
    // be rejected with 403 or 404.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("browse_traversal");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagetraversaladmin",
        "storagetraversaladmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagetraversaladmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Try to escape the storage root with a traversal
    let (status, _) = authenticated_get(
        create_router(state),
        "/api/storage/browse?path=../../etc",
        &cookie,
    )
    .await;

    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "Path traversal must return 403 or 404. Got: {}",
        status
    );
}

// ============================================================================
// GET /api/storage/stats — requires storage.view
// ============================================================================

#[tokio::test]
async fn test_stats_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/storage/stats")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/storage/stats without auth must return 401"
    );
}

#[tokio::test]
async fn test_stats_as_admin_returns_200() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("stats_admin");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagestatsadmin",
        "storagestatsadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagestatsadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/storage/stats", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/storage/stats must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("total_bytes").is_some(),
        "Stats must include 'total_bytes'"
    );
    assert!(
        json.get("used_bytes").is_some(),
        "Stats must include 'used_bytes'"
    );
    assert!(
        json.get("free_bytes").is_some(),
        "Stats must include 'free_bytes'"
    );
    assert!(
        json.get("usage_percent").is_some(),
        "Stats must include 'usage_percent'"
    );
}

// ============================================================================
// GET /api/storage/file-info — requires storage.view
// ============================================================================

#[tokio::test]
async fn test_file_info_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/storage/file-info?path=test.txt")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/storage/file-info without auth must return 401"
    );
}

#[tokio::test]
async fn test_file_info_for_existing_file() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("file_info");
    // Create a test file
    let test_file = tmp.path().join("hello.txt");
    std::fs::write(&test_file, b"hello world").unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagefileinfoadmin",
        "storagefileinfoadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagefileinfoadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(
        create_router(state),
        "/api/storage/file-info?path=hello.txt",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/storage/file-info must return 200 for existing file. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["name"], "hello.txt", "File name must match");
    assert_eq!(json["type"], "file", "Type must be 'file'");
    assert_eq!(json["size"], 11, "File size must be 11 bytes");
    assert!(
        json.get("modified").is_some(),
        "Response must include 'modified' timestamp"
    );
    assert!(
        json.get("permissions").is_some(),
        "Response must include 'permissions'"
    );
}

#[tokio::test]
async fn test_file_info_for_nonexistent_path_returns_404() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("file_info_404");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storageinfo404admin",
        "storageinfo404admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storageinfo404admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(
        create_router(state),
        "/api/storage/file-info?path=this_file_does_not_exist.xyz",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "file-info on a nonexistent path must return 404"
    );
}

// ============================================================================
// POST /api/storage/mkdir — requires storage.write
// ============================================================================

#[tokio::test]
async fn test_mkdir_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/storage/mkdir")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"path":"newdir"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "POST /api/storage/mkdir without auth must return 401"
    );
}

#[tokio::test]
async fn test_mkdir_viewer_lacks_storage_write_returns_403() {
    // Viewer role has storage.view but NOT storage.write.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("mkdir_viewer");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagemkdirviewer",
        "storagemkdirviewer@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagemkdirviewer",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_post(
        create_router(state),
        "/api/storage/mkdir",
        &cookie,
        r#"{"path":"testdir"}"#,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer without storage.write must get 403 on POST /api/storage/mkdir"
    );
}

#[tokio::test]
async fn test_mkdir_creates_directory() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("mkdir_create");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagemkdiradmin",
        "storagemkdiradmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagemkdiradmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let mkdir_body = serde_json::json!({ "path": "new_test_directory" }).to_string();

    let (status, body) = authenticated_post(
        create_router(state.clone()),
        "/api/storage/mkdir",
        &cookie,
        &mkdir_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "POST /api/storage/mkdir must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["success"], true, "Response must confirm success");

    // Verify the directory was actually created on disk
    assert!(
        tmp.path().join("new_test_directory").exists(),
        "Directory must exist on disk after mkdir"
    );
    assert!(
        tmp.path().join("new_test_directory").is_dir(),
        "Created path must be a directory"
    );
}

#[tokio::test]
async fn test_mkdir_already_exists_returns_400() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("mkdir_exists");
    // Pre-create the directory
    std::fs::create_dir(tmp.path().join("already_exists")).unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagemkdirexists",
        "storagemkdirexists@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagemkdirexists",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_post(
        create_router(state),
        "/api/storage/mkdir",
        &cookie,
        r#"{"path":"already_exists"}"#,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "mkdir on an existing path must return 400"
    );
}

// ============================================================================
// DELETE /api/storage/delete — requires storage.delete
// ============================================================================

#[tokio::test]
async fn test_delete_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/storage/delete?path=somefile.txt")
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "DELETE /api/storage/delete without auth must return 401"
    );
}

#[tokio::test]
async fn test_delete_viewer_lacks_storage_delete_returns_403() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("delete_viewer");
    let file_path = tmp.path().join("test.txt");
    std::fs::write(&file_path, b"content").unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagedelviewer",
        "storagedelviewer@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagedelviewer",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_delete(
        create_router(state),
        "/api/storage/delete?path=test.txt",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer without storage.delete must get 403 on DELETE /api/storage/delete"
    );
}

#[tokio::test]
async fn test_delete_file_as_admin() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("delete_file");
    let file_path = tmp.path().join("deleteme.txt");
    std::fs::write(&file_path, b"to be deleted").unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagedeladmin",
        "storagedeladmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagedeladmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_delete(
        create_router(state),
        "/api/storage/delete?path=deleteme.txt",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "DELETE /api/storage/delete must return 200 for a file. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["success"], true);

    // Verify the file is actually gone
    assert!(
        !file_path.exists(),
        "Deleted file must no longer exist on disk"
    );
}

#[tokio::test]
async fn test_delete_empty_directory_as_admin() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("delete_emptydir");
    std::fs::create_dir(tmp.path().join("emptydir")).unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagedemdadmin",
        "storagedemdadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagedemdadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_delete(
        create_router(state),
        "/api/storage/delete?path=emptydir",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "DELETE /api/storage/delete must succeed for an empty directory. Body: {}",
        body
    );
    assert!(!tmp.path().join("emptydir").exists());
}

#[tokio::test]
async fn test_delete_protected_folder_returns_403() {
    // The 'downloads' folder is in PROTECTED_FOLDERS and must never be deletable.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("delete_protected");
    std::fs::create_dir(tmp.path().join("downloads")).unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagedelprotadmin",
        "storagedelprotadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagedelprotadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_delete(
        create_router(state),
        "/api/storage/delete?path=downloads",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Deleting a protected folder ('downloads') must return 403. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_delete_media_protected_folder_returns_403() {
    // The 'media' folder is also in PROTECTED_FOLDERS.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("delete_media");
    std::fs::create_dir(tmp.path().join("media")).unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagedelmediadmin",
        "storagedelmediadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagedelmediadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_delete(
        create_router(state),
        "/api/storage/delete?path=media",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Deleting the protected 'media' folder must return 403"
    );
}

#[tokio::test]
async fn test_delete_nonexistent_path_returns_404() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("delete_404");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagedel404admin",
        "storagedel404admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagedel404admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_delete(
        create_router(state),
        "/api/storage/delete?path=does_not_exist_abc.txt",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Deleting a nonexistent path must return 404"
    );
}

#[tokio::test]
async fn test_delete_nonempty_directory_returns_400() {
    // The endpoint only allows deleting empty directories.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("delete_nonempty");
    let dir = tmp.path().join("nonempty_dir");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("child.txt"), b"child").unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagedelnonemptyadmin",
        "storagedelnonemptyadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagedelnonemptyadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_delete(
        create_router(state),
        "/api/storage/delete?path=nonempty_dir",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Deleting a non-empty directory must return 400. Body: {}",
        body
    );
}

// ============================================================================
// GET /api/storage/download — requires storage.download
// ============================================================================

#[tokio::test]
async fn test_download_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/storage/download?path=test.txt")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/storage/download without auth must return 401"
    );
}

#[tokio::test]
async fn test_download_file_as_admin() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("download_file");
    let file_content = b"Hello, download test!";
    std::fs::write(tmp.path().join("sample.txt"), file_content).unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagedownloadadmin",
        "storagedownloadadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagedownloadadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let request = Request::builder()
        .uri("/api/storage/download?path=sample.txt")
        .method("GET")
        .header("Cookie", &cookie)
        .body(Body::empty())
        .unwrap();

    let response = create_router(state).oneshot(request).await.unwrap();
    let status = response.status();

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/storage/download must return 200 for an existing file"
    );

    // Check content-disposition header
    let content_disp = response
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_disp.contains("attachment"),
        "Download must set Content-Disposition: attachment. Got: {}",
        content_disp
    );
    assert!(
        content_disp.contains("sample.txt"),
        "Content-Disposition must include filename. Got: {}",
        content_disp
    );

    // Read body and verify content
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        body.as_ref(),
        file_content,
        "Downloaded file content must match the original"
    );
}

#[tokio::test]
async fn test_download_nonexistent_file_returns_404() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("download_404");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagedownload404admin",
        "storagedownload404admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagedownload404admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(
        create_router(state),
        "/api/storage/download?path=nonexistent_file.bin",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Downloading a nonexistent file must return 404"
    );
}

#[tokio::test]
async fn test_download_directory_returns_400() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("download_dir");
    std::fs::create_dir(tmp.path().join("a_directory")).unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagedownloaddiradmin",
        "storagedownloaddiradmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagedownloaddiradmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(
        create_router(state),
        "/api/storage/download?path=a_directory",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Downloading a directory must return 400"
    );
}

#[tokio::test]
async fn test_download_viewer_can_download() {
    // The viewer role has storage.download permission.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("download_viewer");
    std::fs::write(tmp.path().join("viewer_file.txt"), b"content for viewer").unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagedownloadviewer",
        "storagedownloadviewer@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagedownloadviewer",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(
        create_router(state),
        "/api/storage/download?path=viewer_file.txt",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Viewer with storage.download must be able to download files"
    );
}

// ============================================================================
// Browse: directory item structure validation
// ============================================================================

#[tokio::test]
async fn test_browse_item_structure() {
    // Items in the browse response must have the expected fields.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("browse_structure");
    // Create a file and a subdirectory
    std::fs::write(tmp.path().join("info.txt"), b"data").unwrap();
    std::fs::create_dir(tmp.path().join("subdir")).unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "storagestructureadmin",
        "storagestructureadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "storagestructureadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/storage/browse", &cookie).await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let items = json["items"].as_array().unwrap();

    assert!(
        items.len() >= 2,
        "Must have at least 2 items (file + directory)"
    );

    for item in items {
        assert!(item.get("name").is_some(), "Item must have 'name'");
        assert!(item.get("path").is_some(), "Item must have 'path'");
        assert!(item.get("type").is_some(), "Item must have 'type'");
        assert!(item.get("size").is_some(), "Item must have 'size'");
        assert!(item.get("modified").is_some(), "Item must have 'modified'");
    }

    // Directories must come before files (sorted by the endpoint)
    let types: Vec<&str> = items.iter().filter_map(|i| i["type"].as_str()).collect();
    let first_dir_idx = types.iter().position(|&t| t == "directory");
    let first_file_idx = types.iter().position(|&t| t == "file");

    if let (Some(dir_idx), Some(file_idx)) = (first_dir_idx, first_file_idx) {
        assert!(
            dir_idx < file_idx,
            "Directories must appear before files in browse results"
        );
    }
}

// ============================================================================
// GET /api/storage/browse — subdirectory navigation
// ============================================================================

#[tokio::test]
async fn test_browse_subdirectory() {
    // After creating a subdirectory and placing a file inside it, browsing the
    // subdirectory path must return only the contents of that subdirectory.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("browse_subdir");
    // Create a sub-directory with a file inside it
    let subdir = tmp.path().join("videos");
    std::fs::create_dir(&subdir).unwrap();
    std::fs::write(subdir.join("movie.mp4"), b"fake-video-content").unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "browssubdiradmin",
        "browsesubdiradmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "browssubdiradmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(
        create_router(state),
        "/api/storage/browse?path=videos",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Browsing a subdirectory must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["path"].as_str().unwrap_or(""),
        "videos",
        "Response path must reflect the browsed subdirectory"
    );

    let items = json["items"].as_array().unwrap();
    assert_eq!(
        items.len(),
        1,
        "Subdirectory must contain exactly one item (movie.mp4)"
    );
    assert_eq!(items[0]["name"], "movie.mp4");
}

#[tokio::test]
async fn test_browse_subdirectory_has_parent_link() {
    // When browsing a subdirectory the response must include a non-null
    // `parent` field so the UI can navigate back up.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("browse_parent");
    std::fs::create_dir(tmp.path().join("music")).unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "browseparentadmin",
        "browseparentadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "browseparentadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(
        create_router(state),
        "/api/storage/browse?path=music",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Browse must succeed. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    // `parent` is Some("") when the parent is the root
    assert!(
        json.get("parent").is_some(),
        "Response must include a 'parent' field for non-root paths"
    );
    assert!(
        !json["parent"].is_null(),
        "parent field must not be null for a first-level subdirectory"
    );
}

#[tokio::test]
async fn test_browse_root_parent_is_null() {
    // Browsing the root (empty path) must set `parent` to null — there is no
    // parent directory above the storage root.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("browse_rootparent");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "browserootparentadmin",
        "browserootparentadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "browserootparentadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/storage/browse", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Browse root must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json["parent"].is_null(),
        "Root browse must have null parent. Got: {:?}",
        json["parent"]
    );
}

// ============================================================================
// GET /api/storage/file-info — directory info
// ============================================================================

#[tokio::test]
async fn test_file_info_for_directory() {
    // file-info should work for directories as well as files, returning
    // type = "directory" and size = 0.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("file_info_dir");
    std::fs::create_dir(tmp.path().join("mydir")).unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "fileinfodirtestadmin",
        "fileinfodirtest@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "fileinfodirtestadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(
        create_router(state),
        "/api/storage/file-info?path=mydir",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "file-info for a directory must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["name"], "mydir", "Directory name must match");
    assert_eq!(
        json["type"], "directory",
        "Type must be 'directory' for a directory path"
    );
    assert_eq!(json["size"], 0, "Directory size must be reported as 0");
}

#[tokio::test]
async fn test_file_info_path_traversal_rejected() {
    // Attempting to get file-info for a path outside the storage root via
    // directory traversal must be rejected.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("file_info_traversal");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "fileinfotraversaladmin",
        "fileinfotraversal@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "fileinfotraversaladmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(
        create_router(state),
        "/api/storage/file-info?path=../../etc/passwd",
        &cookie,
    )
    .await;

    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "Path traversal in file-info must return 403 or 404. Got: {}",
        status
    );
}

// ============================================================================
// POST /api/storage/mkdir — nested directory creation
// ============================================================================

#[tokio::test]
async fn test_mkdir_nested_path_creates_all_parents() {
    // mkdir with a path like "a/b/c" must create all intermediate directories
    // because the endpoint uses create_dir_all internally.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("mkdir_nested");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "mkdirnestadmin",
        "mkdirnestadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "mkdirnestadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_post(
        create_router(state),
        "/api/storage/mkdir",
        &cookie,
        r#"{"path":"parent/child/grandchild"}"#,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "mkdir with nested path must succeed. Body: {}",
        body
    );

    assert!(
        tmp.path().join("parent/child/grandchild").exists(),
        "Nested directory structure must be created on disk"
    );
}

// ============================================================================
// GET /api/storage/download — content-type header
// ============================================================================

#[tokio::test]
async fn test_download_sets_octet_stream_content_type() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("download_ct");
    std::fs::write(tmp.path().join("data.bin"), b"\x00\x01\x02\x03").unwrap();

    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "downloadctadmin",
        "downloadctadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "downloadctadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let request = Request::builder()
        .uri("/api/storage/download?path=data.bin")
        .method("GET")
        .header("Cookie", &cookie)
        .body(Body::empty())
        .unwrap();

    let response = create_router(state).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    assert!(
        content_type.contains("application/octet-stream"),
        "Download must set Content-Type: application/octet-stream. Got: {}",
        content_type
    );
}

// ============================================================================
// GET /api/storage/stats — viewer can access stats
// ============================================================================

#[tokio::test]
async fn test_stats_viewer_can_access() {
    // The viewer role has storage.view and must be able to read storage stats.
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("stats_viewer");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "statsvieweruser",
        "statsvieweruser@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "statsvieweruser",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/storage/stats", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Viewer must be allowed to GET /api/storage/stats. Body: {}",
        body
    );
}

// ============================================================================
// DELETE /api/storage/delete — path traversal rejected
// ============================================================================

#[tokio::test]
async fn test_delete_path_traversal_rejected() {
    ensure_jwt_keys().await;
    let _lock = STORAGE_LOCK.lock().await;

    let tmp = make_temp_dir("delete_traversal");
    unsafe {
        std::env::set_var("KUBARR_STORAGE_PATH", tmp.path().to_str().unwrap());
    }

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "deltraversaladmin",
        "deltraversaladmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "deltraversaladmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_delete(
        create_router(state),
        "/api/storage/delete?path=../../tmp/attack",
        &cookie,
    )
    .await;

    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "Path traversal in delete must be rejected with 403 or 404. Got: {}",
        status
    );
}
