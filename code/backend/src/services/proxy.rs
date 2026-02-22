//! Reverse proxy service for installed apps
//!
//! Proxies HTTP and WebSocket requests to Kubernetes services.

use axum::{
    body::Body,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::{header, HeaderMap, Method, Response},
};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use tokio_tungstenite::{connect_async, tungstenite};

use crate::error::{AppError, Result};

/// Proxy service for forwarding requests to apps
#[derive(Clone)]
pub struct ProxyService {
    client: Client,
}

impl Default for ProxyService {
    fn default() -> Self {
        Self::new()
    }
}

impl ProxyService {
    #[allow(clippy::expect_used)]
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
        }
    }

    /// Proxy an HTTP request to the target URL
    pub async fn proxy_http(
        &self,
        target_url: &str,
        method: Method,
        headers: HeaderMap,
        body: Body,
    ) -> Result<Response<Body>> {
        // Convert axum body to bytes
        let body_bytes = axum::body::to_bytes(body, usize::MAX)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to read request body: {}", e)))?;

        // Build the request
        let mut req_builder = self.client.request(method.clone(), target_url);

        // Forward headers, excluding hop-by-hop headers
        for (name, value) in headers.iter() {
            let name_str = name.as_str().to_lowercase();
            // Skip hop-by-hop, problematic, and encoding negotiation headers.
            // accept-encoding is stripped so upstream apps don't compress responses,
            // since the proxy reads the full body into memory and may rewrite HTML content.
            if matches!(
                name_str.as_str(),
                "host"
                    | "connection"
                    | "keep-alive"
                    | "proxy-authenticate"
                    | "proxy-authorization"
                    | "te"
                    | "trailers"
                    | "transfer-encoding"
                    | "upgrade"
                    | "content-length"
                    | "accept-encoding"
            ) {
                continue;
            }
            if let Ok(v) = value.to_str() {
                req_builder = req_builder.header(name.as_str(), v);
            }
        }

        // Add body if not empty
        if !body_bytes.is_empty() {
            req_builder = req_builder.body(body_bytes.to_vec());
        }

        // Send the request
        let response = req_builder.send().await.map_err(|e| {
            if e.is_connect() {
                AppError::ServiceUnavailable(format!("Failed to connect to app: {}", e))
            } else if e.is_timeout() {
                AppError::ServiceUnavailable("Request to app timed out".to_string())
            } else {
                AppError::BadGateway(format!("Proxy error: {}", e))
            }
        })?;

        // Build the response
        let status = response.status();
        let resp_headers = response.headers().clone();
        let resp_body = response
            .bytes()
            .await
            .map_err(|e| AppError::BadGateway(format!("Failed to read response: {}", e)))?;

        let mut builder = Response::builder().status(status);

        // Forward response headers
        for (name, value) in resp_headers.iter() {
            // Skip hop-by-hop headers and content-encoding (we strip accept-encoding
            // from requests, but also guard against upstream compressing anyway)
            if matches!(
                name.as_str(),
                "connection"
                    | "keep-alive"
                    | "proxy-authenticate"
                    | "proxy-authorization"
                    | "te"
                    | "trailers"
                    | "transfer-encoding"
                    | "content-encoding"
            ) {
                continue;
            }
            builder = builder.header(name, value);
        }

        builder
            .body(Body::from(resp_body))
            .map_err(|e| AppError::Internal(format!("Failed to build response: {}", e)))
    }
}

/// Check if the request is a WebSocket upgrade request
pub fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
}

/// Proxy a WebSocket connection
pub async fn proxy_websocket(ws: WebSocketUpgrade, target_url: String) -> Response<Body> {
    ws.on_upgrade(move |socket| handle_websocket(socket, target_url))
}

async fn handle_websocket(client_socket: WebSocket, target_url: String) {
    // Connect to the backend WebSocket
    let ws_url = target_url
        .replace("http://", "ws://")
        .replace("https://", "wss://");

    let backend_socket = match connect_async(&ws_url).await {
        Ok((socket, _)) => socket,
        Err(e) => {
            tracing::error!("Failed to connect to backend WebSocket {}: {}", ws_url, e);
            return;
        }
    };

    let (mut client_sink, mut client_stream) = client_socket.split();
    let (mut backend_sink, mut backend_stream) = backend_socket.split();

    // Forward messages in both directions
    let client_to_backend = async {
        while let Some(msg) = client_stream.next().await {
            match msg {
                Ok(msg) => {
                    let tungstenite_msg = axum_to_tungstenite(msg);
                    if let Some(msg) = tungstenite_msg {
                        if backend_sink.send(msg).await.is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    };

    let backend_to_client = async {
        while let Some(msg) = backend_stream.next().await {
            match msg {
                Ok(msg) => {
                    let axum_msg = tungstenite_to_axum(msg);
                    if let Some(msg) = axum_msg {
                        if client_sink.send(msg).await.is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    };

    // Run both directions concurrently, stop when either ends
    tokio::select! {
        _ = client_to_backend => {}
        _ = backend_to_client => {}
    }
}

/// Convert axum WebSocket message to tungstenite message
fn axum_to_tungstenite(msg: Message) -> Option<tungstenite::Message> {
    match msg {
        Message::Text(t) => Some(tungstenite::Message::Text(t.to_string().into())),
        Message::Binary(b) => Some(tungstenite::Message::Binary(b)),
        Message::Ping(p) => Some(tungstenite::Message::Ping(p)),
        Message::Pong(p) => Some(tungstenite::Message::Pong(p)),
        Message::Close(_) => Some(tungstenite::Message::Close(None)),
    }
}

/// Convert tungstenite message to axum WebSocket message
fn tungstenite_to_axum(msg: tungstenite::Message) -> Option<Message> {
    match msg {
        tungstenite::Message::Text(t) => Some(Message::Text(t.to_string().into())),
        tungstenite::Message::Binary(b) => Some(Message::Binary(b)),
        tungstenite::Message::Ping(p) => Some(Message::Ping(p)),
        tungstenite::Message::Pong(p) => Some(Message::Pong(p)),
        tungstenite::Message::Close(_) => Some(Message::Close(None)),
        tungstenite::Message::Frame(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header as h;

    fn headers_with(key: &'static str, value: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(key, value.parse().expect("valid header value"));
        map
    }

    #[test]
    fn test_is_websocket_upgrade_true() {
        let headers = headers_with("upgrade", "websocket");
        assert!(is_websocket_upgrade(&headers));
    }

    #[test]
    fn test_is_websocket_upgrade_case_insensitive() {
        let headers = headers_with("upgrade", "WebSocket");
        assert!(is_websocket_upgrade(&headers));
    }

    #[test]
    fn test_is_websocket_upgrade_false_no_header() {
        let headers = HeaderMap::new();
        assert!(!is_websocket_upgrade(&headers));
    }

    #[test]
    fn test_is_websocket_upgrade_false_different_value() {
        let headers = headers_with("upgrade", "h2c");
        assert!(!is_websocket_upgrade(&headers));
    }

    #[test]
    fn test_axum_to_tungstenite_text() {
        let msg = Message::Text("hello".to_string().into());
        let result = axum_to_tungstenite(msg);
        assert!(matches!(result, Some(tungstenite::Message::Text(_))));
    }

    #[test]
    fn test_axum_to_tungstenite_binary() {
        let msg = Message::Binary(axum::body::Bytes::from(vec![1_u8, 2, 3]));
        let result = axum_to_tungstenite(msg);
        assert!(matches!(result, Some(tungstenite::Message::Binary(_))));
    }

    #[test]
    fn test_axum_to_tungstenite_close() {
        let msg = Message::Close(None);
        let result = axum_to_tungstenite(msg);
        assert!(matches!(result, Some(tungstenite::Message::Close(None))));
    }

    #[test]
    fn test_tungstenite_to_axum_text() {
        let msg = tungstenite::Message::Text("world".to_string().into());
        let result = tungstenite_to_axum(msg);
        assert!(matches!(result, Some(Message::Text(_))));
    }

    #[test]
    fn test_tungstenite_to_axum_binary() {
        let msg = tungstenite::Message::Binary(axum::body::Bytes::from(vec![4_u8, 5, 6]));
        let result = tungstenite_to_axum(msg);
        assert!(matches!(result, Some(Message::Binary(_))));
    }

    #[test]
    fn test_tungstenite_to_axum_close() {
        let msg = tungstenite::Message::Close(None);
        let result = tungstenite_to_axum(msg);
        assert!(matches!(result, Some(Message::Close(_))));
    }

    #[test]
    fn test_tungstenite_to_axum_frame_returns_none() {
        use tungstenite::protocol::frame::Frame;
        let frame = Frame::ping(vec![]);
        let msg = tungstenite::Message::Frame(frame);
        let result = tungstenite_to_axum(msg);
        assert!(result.is_none(), "Frame messages must be dropped");
    }

    #[test]
    fn test_proxy_service_new() {
        let svc = ProxyService::new();
        let _cloned = svc.clone(); // must be cloneable
    }

    #[test]
    fn test_proxy_service_default() {
        let svc = ProxyService::default();
        let _ = svc;
    }

    #[test]
    fn test_axum_to_tungstenite_ping() {
        let msg = Message::Ping(axum::body::Bytes::from(vec![1_u8, 2]));
        let result = axum_to_tungstenite(msg);
        assert!(matches!(result, Some(tungstenite::Message::Ping(_))));
    }

    #[test]
    fn test_axum_to_tungstenite_pong() {
        let msg = Message::Pong(axum::body::Bytes::from(vec![3_u8, 4]));
        let result = axum_to_tungstenite(msg);
        assert!(matches!(result, Some(tungstenite::Message::Pong(_))));
    }

    #[test]
    fn test_tungstenite_to_axum_ping() {
        let msg = tungstenite::Message::Ping(axum::body::Bytes::from(vec![1_u8]));
        let result = tungstenite_to_axum(msg);
        assert!(matches!(result, Some(Message::Ping(_))));
    }

    #[test]
    fn test_tungstenite_to_axum_pong() {
        let msg = tungstenite::Message::Pong(axum::body::Bytes::from(vec![2_u8]));
        let result = tungstenite_to_axum(msg);
        assert!(matches!(result, Some(Message::Pong(_))));
    }
}
