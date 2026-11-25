//! Minimal client for Anthropic's Messages API (`POST /v1/messages`).
//!
//! There is no official Rust SDK, so this speaks the HTTP API directly with
//! the shared `reqwest` stack. Phase 1 only checks the key against the
//! Models API ([`MessagesClient::check_model`]); the request and response types are open enough that streaming
//! and tools can be added beside [`MessagesClient::create`] later.
//!
//! The API key is passed per call and only ever placed in the `x-api-key`
//! header. Errors carry the HTTP status and Anthropic's error *type* (a fixed
//! vocabulary), never the provider's message text, so nothing that could echo
//! the key reaches logs or responses.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Anthropic's production API.
pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
/// API version header value.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Beta header value enabling server-side refusal fallbacks.
pub const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// A streamed answer may think for a while; the whole stream gets this long.
const STREAM_TIMEOUT: Duration = Duration::from_secs(600);

/// One message of the conversation sent to the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    /// `user` or `assistant` (or a mid-conversation `system`).
    pub role: String,
    /// A string or an array of content blocks.
    pub content: serde_json::Value,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: serde_json::Value::String(text.into()),
        }
    }
}

/// `output_config` of a request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputConfig {
    /// `low`, `medium` or `high`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// Body of `POST /v1/messages`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<OutputConfig>,
    /// Client tool definitions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    /// `{"type": "auto"}` or `{"type": "none"}`; forced choices are not
    /// supported by the current models.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Server-side refusal fallbacks (`"default"`). When set, the request also
    /// carries the [`FALLBACK_BETA`] header.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallbacks: Option<serde_json::Value>,
}

/// Token counts of one response.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
}

/// The parts of a Messages API response Kubarr reads.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessagesResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub content: Vec<serde_json::Value>,
    #[serde(default)]
    pub usage: Usage,
}

/// Why a call failed. Deliberately carries no provider text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    /// Anthropic answered with a non-2xx status.
    #[error("Anthropic API returned HTTP {status}")]
    Status {
        status: u16,
        /// Anthropic's `error.type`, e.g. `authentication_error`.
        error_type: Option<String>,
    },
    /// The request could not be sent or timed out.
    #[error("could not reach the Anthropic API")]
    Transport,
    /// A 2xx response whose body was not a Messages response.
    #[error("unexpected response from the Anthropic API")]
    Decode,
}

impl ClientError {
    /// A short sentence for the UI. Fixed text per failure class: never the
    /// provider's message, which could quote the request.
    pub fn user_message(&self) -> &'static str {
        match self {
            ClientError::Status { status, error_type } => match (error_type.as_deref(), *status) {
                (Some("authentication_error" | "permission_error"), _) | (_, 401 | 403) => {
                    "Anthropic rejected the API key."
                }
                (Some("not_found_error"), _) | (_, 404) => {
                    "The selected model is not available for this API key."
                }
                (Some("billing_error"), _) | (_, 402) => {
                    "The Anthropic account has a billing problem (check credits)."
                }
                (Some("rate_limit_error"), _) | (_, 429) => {
                    "Anthropic rate-limited the request. Try again shortly."
                }
                (Some("overloaded_error"), _) | (_, 529) | (_, 500..=599) => {
                    "Anthropic is temporarily unavailable. Try again later."
                }
                _ => "Anthropic rejected the test request.",
            },
            ClientError::Transport => "Could not reach the Anthropic API.",
            ClientError::Decode => "Anthropic sent an unexpected response.",
        }
    }
}

/// Talks to the Messages API at a base URL (Anthropic's, or a test server).
#[derive(Clone)]
pub struct MessagesClient {
    http: reqwest::Client,
    base_url: String,
}

impl std::fmt::Debug for MessagesClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessagesClient")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl MessagesClient {
    /// A client for `base_url` (no trailing slash needed).
    pub fn new(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    /// A client for Anthropic's production API.
    pub fn anthropic() -> Self {
        Self::new(ANTHROPIC_BASE_URL)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Check that `api_key` may use `model` with `GET /v1/models/{model}`.
    /// Free: no tokens are spent.
    pub async fn check_model(&self, api_key: &str, model: &str) -> Result<(), ClientError> {
        let response = self
            .http
            .get(format!("{}/v1/models/{model}", self.base_url))
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .send()
            .await
            .map_err(|_| ClientError::Transport)?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let body = response.bytes().await.unwrap_or_default();
        Err(ClientError::Status {
            status: status.as_u16(),
            error_type: error_type(&body),
        })
    }

    fn post(&self, api_key: &str, request: &MessagesRequest) -> reqwest::RequestBuilder {
        let mut builder = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json");
        if request.fallbacks.is_some() {
            builder = builder.header("anthropic-beta", FALLBACK_BETA);
        }
        builder.json(request)
    }

    /// Send one non-streaming request.
    pub async fn create(
        &self,
        api_key: &str,
        request: &MessagesRequest,
    ) -> Result<MessagesResponse, ClientError> {
        let response = self
            .post(api_key, request)
            .send()
            .await
            .map_err(|_| ClientError::Transport)?;
        let status = response.status();
        let body = response.bytes().await.map_err(|_| ClientError::Transport)?;
        if !status.is_success() {
            return Err(ClientError::Status {
                status: status.as_u16(),
                error_type: error_type(&body),
            });
        }
        serde_json::from_slice(&body).map_err(|_| ClientError::Decode)
    }
}

impl MessagesClient {
    /// Send a streaming request (`stream: true` is set here). A non-2xx answer
    /// is an error; otherwise the body is read chunk by chunk from the
    /// returned [`ByteStream`].
    pub async fn stream(
        &self,
        api_key: &str,
        request: &MessagesRequest,
    ) -> Result<ByteStream, ClientError> {
        let mut request = request.clone();
        request.stream = Some(true);
        let response = self
            .post(api_key, &request)
            .timeout(STREAM_TIMEOUT)
            .send()
            .await
            .map_err(|_| ClientError::Transport)?;
        let status = response.status();
        if !status.is_success() {
            let body = response.bytes().await.map_err(|_| ClientError::Transport)?;
            return Err(ClientError::Status {
                status: status.as_u16(),
                error_type: error_type(&body),
            });
        }
        Ok(ByteStream { response })
    }
}

/// The body of a streaming response.
pub struct ByteStream {
    response: reqwest::Response,
}

impl ByteStream {
    /// The next chunk of the body; `None` at the end.
    pub async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ClientError> {
        match self.response.chunk().await {
            Ok(chunk) => Ok(chunk.map(|bytes| bytes.to_vec())),
            Err(_) => Err(ClientError::Transport),
        }
    }
}

/// Anthropic's `error.type` if it is a plain identifier (it always is; the
/// check keeps arbitrary text out of the error).
pub(crate) fn error_type(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let kind = value.get("error")?.get("type")?.as_str()?;
    (kind.len() <= 64 && kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
        .then(|| kind.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_omits_unset_optional_fields() {
        let request = MessagesRequest {
            model: "claude-opus-5-5".into(),
            max_tokens: 1,
            messages: vec![Message::user("ping")],
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({
                "model": "claude-opus-5-5",
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "ping"}]
            })
        );
    }

    #[test]
    fn response_parses_usage_with_cache_fields() {
        let body = serde_json::json!({
            "id": "msg_1", "type": "message", "role": "assistant",
            "model": "claude-opus-5-5", "stop_reason": "max_tokens",
            "content": [],
            "usage": {"input_tokens": 8, "output_tokens": 1,
                      "cache_creation_input_tokens": 3, "cache_read_input_tokens": 5}
        });
        let response: MessagesResponse = serde_json::from_value(body).unwrap();
        assert_eq!(
            response.usage,
            Usage {
                input_tokens: 8,
                output_tokens: 1,
                cache_creation_input_tokens: Some(3),
                cache_read_input_tokens: Some(5),
            }
        );
    }

    #[test]
    fn error_type_accepts_identifiers_only() {
        assert_eq!(
            error_type(br#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#),
            Some("authentication_error".into())
        );
        assert_eq!(
            error_type(br#"{"error":{"type":"sk-ant-api03-leak"}}"#),
            None
        );
        assert_eq!(error_type(b"not json"), None);
    }

    #[test]
    fn user_messages_are_fixed_per_class() {
        let status = |status, kind: Option<&str>| ClientError::Status {
            status,
            error_type: kind.map(str::to_string),
        };
        assert_eq!(
            status(401, Some("authentication_error")).user_message(),
            "Anthropic rejected the API key."
        );
        assert_eq!(
            status(403, Some("permission_error")).user_message(),
            "Anthropic rejected the API key."
        );
        assert_eq!(
            status(404, Some("not_found_error")).user_message(),
            "The selected model is not available for this API key."
        );
        assert_eq!(
            status(529, None).user_message(),
            "Anthropic is temporarily unavailable. Try again later."
        );
        assert_eq!(
            status(400, Some("invalid_request_error")).user_message(),
            "Anthropic rejected the test request."
        );
        assert_eq!(
            ClientError::Transport.user_message(),
            "Could not reach the Anthropic API."
        );
    }
}
