use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use sea_orm::EntityTrait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio_util::io::ReaderStream;

use crate::error::{AppError, Result};
use crate::middleware::permissions::{
    Authorized, StorageDelete, StorageDownload, StorageView, StorageWrite,
};
use crate::models::prelude::*;
use crate::state::{AppState, DbConn};

/// Protected top-level folders that cannot be deleted
const PROTECTED_FOLDERS: &[&str] = &["downloads", "media"];

/// Create storage routes
pub fn storage_routes(state: AppState) -> Router {
    Router::new()
        .route("/browse", get(browse_directory))
        .route("/stats", get(get_storage_stats))
        .route("/file-info", get(get_file_info))
        .route("/mkdir", post(create_directory))
        .route("/delete", delete(delete_path))
        .route("/download", get(download_file))
        .with_state(state)
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct BrowseQuery {
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PathQuery {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub file_type: String,
    pub size: u64,
    pub modified: String,
    pub permissions: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DirectoryListing {
    pub path: String,
    pub parent: Option<String>,
    pub items: Vec<FileInfo>,
    pub total_items: usize,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct StorageStats {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub usage_percent: f64,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateDirectoryRequest {
    pub path: String,
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Get the configured storage path
async fn get_storage_path(db: &DbConn) -> Result<PathBuf> {
    // Check if storage is configured in storage_config.
    let storage_cfg = StorageConfig::find().one(db).await?;

    // Use environment variable for the actual path inside the container
    let storage_path = std::env::var("KUBARR_STORAGE_PATH").unwrap_or_else(|_| "/data".to_string());

    let path = PathBuf::from(&storage_path);

    if path.exists() {
        return Ok(path);
    }

    // If no mount path but storage is configured, storage might not be mounted
    if storage_cfg.is_some() {
        return Err(AppError::Internal(
            "Storage is configured but not mounted. Check kubarr deployment.".to_string(),
        ));
    }

    // Storage not configured at all
    Err(AppError::Internal(
        "Storage not configured. Please complete initial setup.".to_string(),
    ))
}

/// Validate and resolve a requested path to prevent directory traversal
fn validate_path(requested_path: &str, storage_path: &Path) -> Result<PathBuf> {
    let base_path = storage_path
        .canonicalize()
        .map_err(|e| AppError::Internal(format!("Failed to resolve storage path: {}", e)))?;

    // Normalize the requested path
    let clean_path = requested_path.trim_start_matches('/');
    let full_path = base_path.join(clean_path);

    // Resolve to absolute path
    let resolved = full_path
        .canonicalize()
        .map_err(|_| AppError::NotFound(format!("Path not found: {}", requested_path)))?;

    // Ensure the resolved path is within the storage base
    if !resolved.starts_with(&base_path) {
        return Err(AppError::Forbidden(
            "Access denied: path traversal attempt detected".to_string(),
        ));
    }

    Ok(resolved)
}

/// Get information about a file or directory
fn get_file_info_internal(file_path: &PathBuf, base_path: &PathBuf) -> Result<FileInfo> {
    let metadata = std::fs::metadata(file_path)
        .map_err(|e| AppError::Internal(format!("Failed to get file metadata: {}", e)))?;

    let relative_path = file_path
        .strip_prefix(base_path)
        .map_err(|_| AppError::Internal("Failed to calculate relative path".to_string()))?;

    let file_type = if metadata.is_dir() {
        "directory"
    } else {
        "file"
    };

    let size = if metadata.is_dir() { 0 } else { metadata.len() };

    let modified = metadata
        .modified()
        .map(|t| {
            chrono::DateTime::<chrono::Utc>::from(t)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string()
        })
        .unwrap_or_else(|_| "unknown".to_string());

    // Get permissions (Unix only, fallback for Windows)
    #[cfg(unix)]
    let permissions = {
        use std::os::unix::fs::PermissionsExt;
        format!("{:o}", metadata.permissions().mode() & 0o777)
    };

    #[cfg(not(unix))]
    let permissions = if metadata.permissions().readonly() {
        "444".to_string()
    } else {
        "644".to_string()
    };

    Ok(FileInfo {
        name: file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        path: relative_path
            .to_string_lossy()
            .to_string()
            .replace('\\', "/"),
        file_type: file_type.to_string(),
        size,
        modified,
        permissions,
    })
}

// ============================================================================
// Endpoint Handlers
// ============================================================================

/// Browse a directory in the shared storage
#[utoipa::path(
    get,
    path = "/api/storage/browse",
    tag = "Storage",
    params(
        ("path" = String, Query, description = "Directory path to browse"),
    ),
    responses(
        (status = 200, body = DirectoryListing)
    )
)]
async fn browse_directory(
    State(state): State<AppState>,
    Query(query): Query<BrowseQuery>,
    _auth: Authorized<StorageView>,
) -> Result<Json<DirectoryListing>> {
    let db = state.get_db().await?;
    let storage_path = get_storage_path(&db).await?;
    let base_path = storage_path
        .canonicalize()
        .map_err(|e| AppError::Internal(format!("Failed to resolve storage path: {}", e)))?;

    // Handle empty path as root
    let dir_path = if query.path.is_empty() || query.path == "/" {
        base_path.clone()
    } else {
        validate_path(&query.path, &storage_path)?
    };

    if !dir_path.exists() {
        return Err(AppError::NotFound(format!(
            "Path not found: {}",
            query.path
        )));
    }

    if !dir_path.is_dir() {
        return Err(AppError::BadRequest(format!(
            "Path is not a directory: {}",
            query.path
        )));
    }

    // List directory contents
    let mut items: Vec<FileInfo> = Vec::new();
    let entries = std::fs::read_dir(&dir_path)
        .map_err(|e| AppError::Forbidden(format!("Permission denied: {}", e)))?;

    let mut entries_vec: Vec<_> = entries.filter_map(|e| e.ok()).collect();

    // Sort: directories first, then by name
    entries_vec.sort_by(|a, b| {
        let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);

        match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a
                .file_name()
                .to_ascii_lowercase()
                .cmp(&b.file_name().to_ascii_lowercase()),
        }
    });

    for entry in entries_vec {
        let entry_path = entry.path();
        if let Ok(info) = get_file_info_internal(&entry_path, &base_path) {
            items.push(info);
        }
    }

    // Calculate relative path and parent
    let relative_path = if dir_path == base_path {
        String::new()
    } else {
        dir_path
            .strip_prefix(&base_path)
            .map(|p| p.to_string_lossy().to_string().replace('\\', "/"))
            .unwrap_or_default()
    };

    let parent = if dir_path == base_path {
        None
    } else {
        let parent_path = dir_path.parent();
        match parent_path {
            Some(p) if p == base_path => Some(String::new()),
            Some(p) => Some(
                p.strip_prefix(&base_path)
                    .map(|rel| rel.to_string_lossy().to_string().replace('\\', "/"))
                    .unwrap_or_default(),
            ),
            None => None,
        }
    };

    let total_items = items.len();

    Ok(Json(DirectoryListing {
        path: relative_path,
        parent,
        items,
        total_items,
    }))
}

/// Get storage usage statistics
#[utoipa::path(
    get,
    path = "/api/storage/stats",
    tag = "Storage",
    responses(
        (status = 200, body = StorageStats)
    )
)]
async fn get_storage_stats(
    State(state): State<AppState>,
    _auth: Authorized<StorageView>,
) -> Result<Json<StorageStats>> {
    let db = state.get_db().await?;
    let storage_path = get_storage_path(&db).await?;

    if !storage_path.exists() {
        return Err(AppError::NotFound("Storage path not found".to_string()));
    }

    // Get disk usage
    #[cfg(unix)]
    let stats = {
        let output = std::process::Command::new("df")
            .arg("-B1")
            .arg(&storage_path)
            .output()
            .map_err(|e| AppError::Internal(format!("Failed to get disk usage: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();

        if lines.len() < 2 {
            return Err(AppError::Internal("Failed to parse disk usage".to_string()));
        }

        let parts: Vec<&str> = lines[1].split_whitespace().collect();
        if parts.len() < 4 {
            return Err(AppError::Internal("Failed to parse disk usage".to_string()));
        }

        let total: u64 = parts[1].parse().unwrap_or(0);
        let used: u64 = parts[2].parse().unwrap_or(0);
        let free: u64 = parts[3].parse().unwrap_or(0);

        StorageStats {
            total_bytes: total,
            used_bytes: used,
            free_bytes: free,
            usage_percent: if total > 0 {
                ((used as f64 / total as f64) * 100.0 * 100.0).round() / 100.0
            } else {
                0.0
            },
        }
    };

    #[cfg(windows)]
    let stats = {
        // Windows implementation using GetDiskFreeSpaceExW would be complex
        // For now, return dummy data on Windows
        StorageStats {
            total_bytes: 0,
            used_bytes: 0,
            free_bytes: 0,
            usage_percent: 0.0,
        }
    };

    Ok(Json(stats))
}

/// Get detailed information about a specific file or directory
#[utoipa::path(
    get,
    path = "/api/storage/file-info",
    tag = "Storage",
    params(
        ("path" = String, Query, description = "File or directory path"),
    ),
    responses(
        (status = 200, body = FileInfo)
    )
)]
async fn get_file_info(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
    _auth: Authorized<StorageView>,
) -> Result<Json<FileInfo>> {
    let db = state.get_db().await?;
    let storage_path = get_storage_path(&db).await?;
    let base_path = storage_path
        .canonicalize()
        .map_err(|e| AppError::Internal(format!("Failed to resolve storage path: {}", e)))?;

    let file_path = validate_path(&query.path, &storage_path)?;

    if !file_path.exists() {
        return Err(AppError::NotFound(format!(
            "Path not found: {}",
            query.path
        )));
    }

    let info = get_file_info_internal(&file_path, &base_path)?;
    Ok(Json(info))
}

/// Create a new directory (requires storage.write permission)
#[utoipa::path(
    post,
    path = "/api/storage/mkdir",
    tag = "Storage",
    request_body = CreateDirectoryRequest,
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn create_directory(
    State(state): State<AppState>,
    _auth: Authorized<StorageWrite>,
    Json(request): Json<CreateDirectoryRequest>,
) -> Result<Json<serde_json::Value>> {
    let db = state.get_db().await?;
    let storage_path = get_storage_path(&db).await?;
    let base_path = storage_path
        .canonicalize()
        .map_err(|e| AppError::Internal(format!("Failed to resolve storage path: {}", e)))?;

    // Build the target path
    let clean_path = request.path.trim_start_matches('/');
    let dir_path = base_path.join(clean_path);

    // Verify it's within the storage base
    if !dir_path.starts_with(&base_path) {
        return Err(AppError::Forbidden(
            "Access denied: path traversal attempt detected".to_string(),
        ));
    }

    if dir_path.exists() {
        return Err(AppError::BadRequest(format!(
            "Path already exists: {}",
            request.path
        )));
    }

    std::fs::create_dir_all(&dir_path)
        .map_err(|e| AppError::Internal(format!("Failed to create directory: {}", e)))?;

    // Set permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir_path, std::fs::Permissions::from_mode(0o775));
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "message": format!("Directory created: {}", request.path)
    })))
}

/// Delete a file or empty directory (requires storage.delete permission)
#[utoipa::path(
    delete,
    path = "/api/storage/delete",
    tag = "Storage",
    params(
        ("path" = String, Query, description = "Path to delete"),
    ),
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn delete_path(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
    _auth: Authorized<StorageDelete>,
) -> Result<Json<serde_json::Value>> {
    let db = state.get_db().await?;
    let storage_path = get_storage_path(&db).await?;
    let base_path = storage_path
        .canonicalize()
        .map_err(|e| AppError::Internal(format!("Failed to resolve storage path: {}", e)))?;

    let target_path = validate_path(&query.path, &storage_path)?;

    if !target_path.exists() {
        return Err(AppError::NotFound(format!(
            "Path not found: {}",
            query.path
        )));
    }

    // Check if trying to delete the root
    if target_path == base_path {
        return Err(AppError::Forbidden(
            "Cannot delete the storage root".to_string(),
        ));
    }

    // Check if trying to delete a protected top-level folder
    let relative_path = target_path
        .strip_prefix(&base_path)
        .map_err(|_| AppError::Internal("Failed to calculate relative path".to_string()))?;

    let parts: Vec<_> = relative_path.components().collect();
    if parts.len() == 1 {
        if let Some(std::path::Component::Normal(name)) = parts.first() {
            let name_str = name.to_string_lossy();
            if PROTECTED_FOLDERS.contains(&name_str.as_ref()) {
                return Err(AppError::Forbidden(format!(
                    "Cannot delete protected folder: {}",
                    name_str
                )));
            }
        }
    }

    if target_path.is_dir() {
        // Only delete empty directories
        let is_empty = std::fs::read_dir(&target_path)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);

        if !is_empty {
            return Err(AppError::BadRequest(
                "Cannot delete non-empty directory".to_string(),
            ));
        }

        std::fs::remove_dir(&target_path)
            .map_err(|e| AppError::Internal(format!("Failed to delete directory: {}", e)))?;
    } else {
        std::fs::remove_file(&target_path)
            .map_err(|e| AppError::Internal(format!("Failed to delete file: {}", e)))?;
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "message": format!("Deleted: {}", query.path)
    })))
}

/// Download a file from storage
#[utoipa::path(
    get,
    path = "/api/storage/download",
    tag = "Storage",
    params(
        ("path" = String, Query, description = "File path to download"),
    ),
    responses(
        (status = 200, description = "File download")
    )
)]
async fn download_file(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
    _auth: Authorized<StorageDownload>,
) -> Result<Response> {
    let db = state.get_db().await?;
    let storage_path = get_storage_path(&db).await?;
    let file_path = validate_path(&query.path, &storage_path)?;

    if !file_path.exists() {
        return Err(AppError::NotFound(format!(
            "File not found: {}",
            query.path
        )));
    }

    if file_path.is_dir() {
        return Err(AppError::BadRequest(
            "Cannot download a directory".to_string(),
        ));
    }

    let file_name = file_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());

    // Open file for streaming
    let file = tokio::fs::File::open(&file_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to open file: {}", e)))?;

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (
                header::CONTENT_DISPOSITION,
                &format!("attachment; filename=\"{}\"", file_name),
            ),
        ],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // -------------------------------------------------------------------------
    // validate_path
    // -------------------------------------------------------------------------

    #[test]
    fn validate_path_resolves_valid_subpath() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        fs::create_dir_all(base.join("subdir")).expect("create subdir");

        let result = validate_path("subdir", base);
        assert!(result.is_ok());
        assert!(result.unwrap().starts_with(base));
    }

    #[test]
    fn validate_path_root_slash_resolves_to_base() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let result = validate_path("/", base);
        // "/" strips to "" and joins to base itself
        assert!(result.is_ok());
    }

    #[test]
    fn validate_path_empty_resolves_to_base() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let result = validate_path("", base);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_path_traversal_blocked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        // Attempt to traverse above base
        let result = validate_path("../etc/passwd", base);
        // Either NotFound (path doesn't exist) or Forbidden is acceptable
        assert!(result.is_err());
    }

    #[test]
    fn validate_path_nonexistent_returns_err() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let result = validate_path("does_not_exist", base);
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // get_file_info_internal
    // -------------------------------------------------------------------------

    #[test]
    fn get_file_info_internal_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().to_path_buf();
        let file_path = base.join("test.txt");
        fs::write(&file_path, b"hello").expect("write");

        let info = get_file_info_internal(&file_path, &base).expect("file info");
        assert_eq!(info.name, "test.txt");
        assert_eq!(info.file_type, "file");
        assert_eq!(info.size, 5);
        assert_eq!(info.path, "test.txt");
    }

    #[test]
    fn get_file_info_internal_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().to_path_buf();
        let sub_dir = base.join("mydir");
        fs::create_dir(&sub_dir).expect("mkdir");

        let info = get_file_info_internal(&sub_dir, &base).expect("dir info");
        assert_eq!(info.name, "mydir");
        assert_eq!(info.file_type, "directory");
        assert_eq!(info.size, 0);
    }

    // -------------------------------------------------------------------------
    // Serde tests for request/response structs
    // -------------------------------------------------------------------------

    #[test]
    fn browse_query_deser_default_path() {
        let q: BrowseQuery = serde_json::from_str("{}").expect("deser");
        assert_eq!(q.path, "");
    }

    #[test]
    fn browse_query_deser_with_path() {
        let q: BrowseQuery = serde_json::from_str(r#"{"path":"/media"}"#).expect("deser");
        assert_eq!(q.path, "/media");
    }

    #[test]
    fn path_query_deser() {
        let q: PathQuery = serde_json::from_str(r#"{"path":"/data/file.txt"}"#).expect("deser");
        assert_eq!(q.path, "/data/file.txt");
    }

    #[test]
    fn file_info_ser() {
        let f = FileInfo {
            name: "test.txt".to_string(),
            path: "test.txt".to_string(),
            file_type: "file".to_string(),
            size: 1024,
            modified: "2026-01-01T00:00:00Z".to_string(),
            permissions: "rw-r--r--".to_string(),
        };
        let json = serde_json::to_string(&f).expect("ser");
        assert!(json.contains("\"name\":\"test.txt\""));
        assert!(json.contains("\"type\":\"file\"")); // renamed field
        assert!(json.contains("\"size\":1024"));
    }

    #[test]
    fn directory_listing_ser() {
        let listing = DirectoryListing {
            path: "/media".to_string(),
            parent: Some("/".to_string()),
            items: vec![],
            total_items: 0,
        };
        let json = serde_json::to_string(&listing).expect("ser");
        assert!(json.contains("\"path\":\"/media\""));
        assert!(json.contains("\"parent\":\"/\""));
        assert!(json.contains("\"total_items\":0"));
    }

    #[test]
    fn directory_listing_null_parent_ser() {
        let listing = DirectoryListing {
            path: "/".to_string(),
            parent: None,
            items: vec![],
            total_items: 0,
        };
        let json = serde_json::to_string(&listing).expect("ser");
        assert!(json.contains("\"parent\":null"));
    }

    #[test]
    fn storage_stats_ser() {
        let stats = StorageStats {
            total_bytes: 1_000_000,
            used_bytes: 500_000,
            free_bytes: 500_000,
            usage_percent: 50.0,
        };
        let json = serde_json::to_string(&stats).expect("ser");
        assert!(json.contains("\"total_bytes\":1000000"));
        assert!(json.contains("\"usage_percent\":50.0"));
    }

    #[test]
    fn create_directory_request_deser() {
        let r: CreateDirectoryRequest =
            serde_json::from_str(r#"{"path":"/media/new_dir"}"#).expect("deser");
        assert_eq!(r.path, "/media/new_dir");
    }
}
