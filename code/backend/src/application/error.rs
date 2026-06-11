use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Internal server error: {0}")]
    Internal(String),

    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("Bad gateway: {0}")]
    BadGateway(String),

    #[error("Database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("Kubernetes error: {0}")]
    Kubernetes(#[from] kube::Error),

    #[error("Kubernetes config error: {0}")]
    KubeConfig(#[from] kube::config::KubeconfigError),

    #[error("Kubernetes in-cluster config error: {0}")]
    KubeInCluster(#[from] kube::config::InClusterError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JWT error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),

    #[error("Bcrypt error: {0}")]
    Bcrypt(#[from] bcrypt::BcryptError),

    #[error("HTTP client error: {0}")]
    HttpClient(#[from] reqwest::Error),
}

#[derive(Serialize)]
struct ErrorResponse {
    detail: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            AppError::ServiceUnavailable(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg.clone()),
            AppError::BadGateway(msg) => (StatusCode::BAD_GATEWAY, msg.clone()),
            AppError::Database(e) => {
                tracing::error!("Database error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error".to_string(),
                )
            }
            AppError::Kubernetes(e) => {
                tracing::error!("Kubernetes error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Kubernetes error: {}", e),
                )
            }
            AppError::KubeConfig(e) => {
                tracing::error!("Kubernetes config error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Kubernetes config error: {}", e),
                )
            }
            AppError::KubeInCluster(e) => {
                tracing::error!("Kubernetes in-cluster config error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Kubernetes in-cluster config error: {}", e),
                )
            }
            AppError::Json(e) => (StatusCode::BAD_REQUEST, format!("JSON error: {}", e)),
            AppError::Yaml(e) => (StatusCode::BAD_REQUEST, format!("YAML error: {}", e)),
            AppError::Io(e) => {
                tracing::error!("IO error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("IO error: {}", e),
                )
            }
            AppError::Jwt(e) => (StatusCode::UNAUTHORIZED, format!("JWT error: {}", e)),
            AppError::Bcrypt(e) => {
                tracing::error!("Bcrypt error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Authentication error".to_string(),
                )
            }
            AppError::HttpClient(e) => {
                tracing::error!("HTTP client error: {}", e);
                (
                    StatusCode::BAD_GATEWAY,
                    format!("Upstream service error: {}", e),
                )
            }
        };

        (status, Json(ErrorResponse { detail: message })).into_response()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;

    /// Helper: convert an AppError into an HTTP response and return (status_code, body_string).
    async fn into_parts(err: AppError) -> (u16, String) {
        let response = err.into_response();
        let status = response.status().as_u16();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        (status, body)
    }

    // -------------------------------------------------------------------------
    // Simple string-wrapped variants
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_not_found_returns_404() {
        let (status, body) = into_parts(AppError::NotFound("thing not found".into())).await;
        assert_eq!(status, 404);
        assert!(body.contains("thing not found"));
    }

    #[tokio::test]
    async fn test_bad_request_returns_400() {
        let (status, body) = into_parts(AppError::BadRequest("bad input".into())).await;
        assert_eq!(status, 400);
        assert!(body.contains("bad input"));
    }

    #[tokio::test]
    async fn test_unauthorized_returns_401() {
        let (status, body) = into_parts(AppError::Unauthorized("not logged in".into())).await;
        assert_eq!(status, 401);
        assert!(body.contains("not logged in"));
    }

    #[tokio::test]
    async fn test_forbidden_returns_403() {
        let (status, body) = into_parts(AppError::Forbidden("access denied".into())).await;
        assert_eq!(status, 403);
        assert!(body.contains("access denied"));
    }

    #[tokio::test]
    async fn test_internal_returns_500() {
        let (status, body) = into_parts(AppError::Internal("unexpected failure".into())).await;
        assert_eq!(status, 500);
        assert!(body.contains("unexpected failure"));
    }

    #[tokio::test]
    async fn test_service_unavailable_returns_503() {
        let (status, body) =
            into_parts(AppError::ServiceUnavailable("down for maintenance".into())).await;
        assert_eq!(status, 503);
        assert!(body.contains("down for maintenance"));
    }

    #[tokio::test]
    async fn test_bad_gateway_returns_502() {
        let (status, body) = into_parts(AppError::BadGateway("upstream timeout".into())).await;
        assert_eq!(status, 502);
        assert!(body.contains("upstream timeout"));
    }

    // -------------------------------------------------------------------------
    // #[from] variants – constructed via From/Into
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_database_error_returns_500() {
        let db_err = sea_orm::DbErr::Custom("db went away".to_string());
        let (status, body) = into_parts(AppError::Database(db_err)).await;
        assert_eq!(status, 500);
        // Body contains the generic message, NOT the raw DB error (for security)
        assert!(body.contains("Database error"));
    }

    #[tokio::test]
    async fn test_json_error_returns_400() {
        // Trigger a real serde_json::Error
        let raw_err: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let (status, body) = into_parts(AppError::Json(raw_err)).await;
        assert_eq!(status, 400);
        assert!(body.contains("JSON error"));
    }

    #[tokio::test]
    async fn test_yaml_error_returns_400() {
        // Trigger a real serde_yaml::Error via an invalid YAML value parse
        let raw_err: serde_yaml::Error =
            serde_yaml::from_str::<serde_json::Value>(":\ninvalid: [unclosed").unwrap_err();
        let (status, body) = into_parts(AppError::Yaml(raw_err)).await;
        assert_eq!(status, 400);
        assert!(body.contains("YAML error"));
    }

    #[tokio::test]
    async fn test_io_error_returns_500() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let (status, body) = into_parts(AppError::Io(io_err)).await;
        assert_eq!(status, 500);
        assert!(body.contains("IO error"));
    }

    #[tokio::test]
    async fn test_bcrypt_error_returns_500() {
        // BcryptError::CostNotAllowed is a simple unit-like variant
        let bcrypt_err = bcrypt::BcryptError::CostNotAllowed(1);
        let (status, body) = into_parts(AppError::Bcrypt(bcrypt_err)).await;
        assert_eq!(status, 500);
        // Generic message is returned (not the raw error)
        assert!(body.contains("Authentication error"));
    }

    #[tokio::test]
    async fn test_jwt_error_returns_401() {
        // Construct a JWT error from an invalid token
        let jwt_err =
            jsonwebtoken::errors::Error::from(jsonwebtoken::errors::ErrorKind::InvalidToken);
        let (status, body) = into_parts(AppError::Jwt(jwt_err)).await;
        assert_eq!(status, 401);
        assert!(body.contains("JWT error"));
    }

    #[tokio::test]
    async fn test_kubernetes_api_error_returns_500() {
        let kube_status = kube::core::response::Status {
            code: 404,
            message: "pods not found".to_string(),
            reason: "NotFound".to_string(),
            ..Default::default()
        };
        let kube_err = kube::Error::Api(Box::new(kube_status));
        let (status, body) = into_parts(AppError::Kubernetes(kube_err)).await;
        assert_eq!(status, 500);
        assert!(body.contains("Kubernetes error"));
    }

    // -------------------------------------------------------------------------
    // Response body shape – must be JSON with a "detail" key
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_response_body_is_json_with_detail_key() {
        let (_, body) = into_parts(AppError::NotFound("widget".into())).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed.get("detail").is_some());
        assert_eq!(parsed["detail"], "widget");
    }

    #[tokio::test]
    async fn test_internal_response_body_detail_matches_message() {
        let (_, body) = into_parts(AppError::Internal("something broke".into())).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["detail"], "something broke");
    }

    // -------------------------------------------------------------------------
    // Display / thiserror formatting (does not require async)
    // -------------------------------------------------------------------------

    #[test]
    fn test_not_found_display() {
        let err = AppError::NotFound("widget".into());
        assert_eq!(format!("{}", err), "Not found: widget");
    }

    #[test]
    fn test_bad_request_display() {
        let err = AppError::BadRequest("missing field".into());
        assert_eq!(format!("{}", err), "Bad request: missing field");
    }

    #[test]
    fn test_unauthorized_display() {
        let err = AppError::Unauthorized("token expired".into());
        assert_eq!(format!("{}", err), "Unauthorized: token expired");
    }

    #[test]
    fn test_forbidden_display() {
        let err = AppError::Forbidden("no access".into());
        assert_eq!(format!("{}", err), "Forbidden: no access");
    }

    #[test]
    fn test_conflict_display() {
        let err = AppError::Conflict("duplicate key".into());
        assert_eq!(format!("{}", err), "Conflict: duplicate key");
    }

    #[test]
    fn test_internal_display() {
        let err = AppError::Internal("crash".into());
        assert_eq!(format!("{}", err), "Internal server error: crash");
    }

    #[test]
    fn test_service_unavailable_display() {
        let err = AppError::ServiceUnavailable("offline".into());
        assert_eq!(format!("{}", err), "Service unavailable: offline");
    }

    #[test]
    fn test_bad_gateway_display() {
        let err = AppError::BadGateway("proxy error".into());
        assert_eq!(format!("{}", err), "Bad gateway: proxy error");
    }

    #[tokio::test]
    async fn test_kube_config_error_returns_500() {
        let kube_cfg_err = kube::config::KubeconfigError::CurrentContextNotSet;
        let (status, body) = into_parts(AppError::KubeConfig(kube_cfg_err)).await;
        assert_eq!(status, 500);
        assert!(body.contains("Kubernetes config error"));
    }

    #[tokio::test]
    async fn test_kube_in_cluster_error_returns_500() {
        let env_err = std::env::VarError::NotPresent;
        let in_cluster_err = kube::config::InClusterError::ReadEnvironmentVariable(env_err);
        let (status, body) = into_parts(AppError::KubeInCluster(in_cluster_err)).await;
        assert_eq!(status, 500);
        assert!(body.contains("Kubernetes in-cluster config error"));
    }

    #[tokio::test]
    async fn test_http_client_error_returns_502() {
        // Build a real reqwest::Error by making a request to an invalid URL
        let reqwest_err = reqwest::Client::new()
            .get("http://0.0.0.0:0/invalid")
            .send()
            .await
            .unwrap_err();
        let (status, body) = into_parts(AppError::HttpClient(reqwest_err)).await;
        assert_eq!(status, 502);
        assert!(body.contains("Upstream service error"));
    }
}
