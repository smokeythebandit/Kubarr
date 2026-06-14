//! Tests for error handling module

use axum::http::StatusCode;
use axum::response::IntoResponse;
use http_body_util::BodyExt;
use kubarr::error::AppError;

async fn get_response_body(response: axum::response::Response) -> (StatusCode, String) {
    let status = response.status();
    let body = response.into_body();
    let bytes = body.collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();
    (status, body_str)
}

#[tokio::test]
async fn test_not_found_error() {
    let error = AppError::NotFound("User not found".to_string());
    let response = error.into_response();
    let (status, body) = get_response_body(response).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("User not found"));
}

#[tokio::test]
async fn test_bad_request_error() {
    let error = AppError::BadRequest("Invalid input".to_string());
    let response = error.into_response();
    let (status, body) = get_response_body(response).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("Invalid input"));
}

#[tokio::test]
async fn test_unauthorized_error() {
    let error = AppError::Unauthorized("Token expired".to_string());
    let response = error.into_response();
    let (status, body) = get_response_body(response).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("Token expired"));
}

#[tokio::test]
async fn test_forbidden_error() {
    let error = AppError::Forbidden("Admin access required".to_string());
    let response = error.into_response();
    let (status, body) = get_response_body(response).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("Admin access required"));
}

#[tokio::test]
async fn test_conflict_error() {
    let error = AppError::Conflict("Username already exists".to_string());
    let response = error.into_response();
    let (status, body) = get_response_body(response).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body.contains("Username already exists"));
}

#[tokio::test]
async fn test_internal_error() {
    let error = AppError::Internal("Something went wrong".to_string());
    let response = error.into_response();
    let (status, body) = get_response_body(response).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("Something went wrong"));
}

#[tokio::test]
async fn test_service_unavailable_error() {
    let error = AppError::ServiceUnavailable("Service down".to_string());
    let response = error.into_response();
    let (status, body) = get_response_body(response).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains("Service down"));
}

#[tokio::test]
async fn test_json_error_response_format() {
    let error = AppError::NotFound("Resource not found".to_string());
    let response = error.into_response();
    let (_, body) = get_response_body(response).await;

    // Response should be JSON with "detail" field
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(parsed.get("detail").is_some());
    assert_eq!(parsed.get("detail").unwrap(), "Resource not found");
}

#[test]
fn test_error_display_impl() {
    assert_eq!(
        AppError::NotFound("test".to_string()).to_string(),
        "Not found: test"
    );
    assert_eq!(
        AppError::BadRequest("test".to_string()).to_string(),
        "Bad request: test"
    );
    assert_eq!(
        AppError::Unauthorized("test".to_string()).to_string(),
        "Unauthorized: test"
    );
    assert_eq!(
        AppError::Forbidden("test".to_string()).to_string(),
        "Forbidden: test"
    );
    assert_eq!(
        AppError::Conflict("test".to_string()).to_string(),
        "Conflict: test"
    );
    assert_eq!(
        AppError::Internal("test".to_string()).to_string(),
        "Internal server error: test"
    );
    assert_eq!(
        AppError::ServiceUnavailable("test".to_string()).to_string(),
        "Service unavailable: test"
    );
}

#[test]
fn test_json_error_from_conversion() {
    let json_err = serde_json::from_str::<serde_json::Value>("invalid json");
    assert!(json_err.is_err());
    let app_error: AppError = json_err.unwrap_err().into();
    assert!(matches!(app_error, AppError::Json(_)));
}

#[test]
fn test_io_error_from_conversion() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let app_error: AppError = io_err.into();
    assert!(matches!(app_error, AppError::Io(_)));
}

#[tokio::test]
async fn test_bad_gateway_error() {
    let error = AppError::BadGateway("upstream timeout".to_string());
    let response = error.into_response();
    let (status, body) = get_response_body(response).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body.contains("upstream timeout"));
}

#[tokio::test]
async fn test_database_error_response() {
    let db_err = sea_orm::DbErr::Custom("test db error".to_string());
    let error = AppError::Database(db_err);
    let response = error.into_response();
    let (status, body) = get_response_body(response).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    // Database errors have their message hidden from clients
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(parsed.get("detail").is_some());
}

#[tokio::test]
async fn test_yaml_error_response() {
    let yaml_err = serde_yaml::from_str::<serde_json::Value>("{unclosed: [").unwrap_err();
    let error = AppError::Yaml(yaml_err);
    let response = error.into_response();
    let (status, body) = get_response_body(response).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("YAML error") || body.contains("detail"));
}

#[tokio::test]
async fn test_io_error_response() {
    let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
    let error = AppError::Io(io_err);
    let response = error.into_response();
    let (status, body) = get_response_body(response).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("IO error") || body.contains("detail"));
}

#[tokio::test]
async fn test_json_parse_error_response() {
    let json_err = serde_json::from_str::<serde_json::Value>("not valid json").unwrap_err();
    let error = AppError::Json(json_err);
    let response = error.into_response();
    let (status, body) = get_response_body(response).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("JSON error") || body.contains("detail"));
}

#[test]
fn test_error_display_remaining_variants() {
    assert_eq!(
        AppError::BadGateway("proxy error".to_string()).to_string(),
        "Bad gateway: proxy error"
    );
}

#[test]
fn test_database_error_from_conversion() {
    let db_err = sea_orm::DbErr::Custom("connection failed".to_string());
    let app_error: AppError = db_err.into();
    assert!(matches!(app_error, AppError::Database(_)));
}

#[test]
fn test_yaml_error_from_conversion() {
    let yaml_err = serde_yaml::from_str::<serde_json::Value>("{bad yaml: [").unwrap_err();
    let app_error: AppError = yaml_err.into();
    assert!(matches!(app_error, AppError::Yaml(_)));
}
