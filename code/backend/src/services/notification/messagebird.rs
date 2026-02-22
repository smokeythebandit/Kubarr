#![allow(dead_code)]

use async_trait::async_trait;
use serde::Deserialize;

use super::{ChannelType, NotificationMessage, NotificationProvider, SendResult};

#[derive(Debug, Deserialize)]
pub struct MessageBirdConfig {
    pub api_key: String,
    pub originator: Option<String>,
}

pub struct MessageBirdProvider {
    api_key: String,
    originator: String,
    client: reqwest::Client,
}

impl MessageBirdProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self, String> {
        let mb_config: MessageBirdConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("Invalid MessageBird config: {}", e))?;

        Ok(Self {
            api_key: mb_config.api_key,
            originator: mb_config.originator.unwrap_or_else(|| "Kubarr".to_string()),
            client: reqwest::Client::new(),
        })
    }

    async fn send_sms(&self, recipient: &str, body: &str) -> SendResult {
        let url = "https://rest.messagebird.com/messages";

        let payload = serde_json::json!({
            "originator": self.originator,
            "recipients": [recipient],
            "body": body
        });

        match self
            .client
            .post(url)
            .header("Authorization", format!("AccessKey {}", self.api_key))
            .json(&payload)
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    SendResult {
                        success: true,
                        error: None,
                    }
                } else {
                    let error_text = response.text().await.unwrap_or_default();
                    SendResult {
                        success: false,
                        error: Some(format!("MessageBird API error: {}", error_text)),
                    }
                }
            }
            Err(e) => SendResult {
                success: false,
                error: Some(format!("Failed to send SMS: {}", e)),
            },
        }
    }
}

#[async_trait]
impl NotificationProvider for MessageBirdProvider {
    fn channel_type(&self) -> ChannelType {
        ChannelType::MessageBird
    }

    async fn send(&self, message: &NotificationMessage) -> SendResult {
        // SMS has character limits, so keep it concise
        let severity_prefix = match message.severity {
            super::NotificationSeverity::Info => "[INFO]",
            super::NotificationSeverity::Warning => "[WARN]",
            super::NotificationSeverity::Critical => "[ALERT]",
        };

        let text = format!(
            "{} {}: {}",
            severity_prefix,
            message.title,
            truncate(&message.body, 120)
        );

        self.send_sms(&message.recipient, &text).await
    }

    async fn test(&self, destination: &str) -> SendResult {
        self.send_sms(
            destination,
            "Kubarr Test: SMS notifications are configured correctly!",
        )
        .await
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // MessageBirdConfig deserialization tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_config_deser_with_originator() {
        let json = serde_json::json!({
            "api_key": "test-key-123",
            "originator": "MySender"
        });
        let cfg: MessageBirdConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.api_key, "test-key-123");
        assert_eq!(cfg.originator, Some("MySender".to_string()));
    }

    #[test]
    fn test_config_deser_without_originator() {
        let json = serde_json::json!({
            "api_key": "test-key-456"
        });
        let cfg: MessageBirdConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.api_key, "test-key-456");
        assert_eq!(cfg.originator, None);
    }

    #[test]
    fn test_config_deser_missing_api_key_fails() {
        let json = serde_json::json!({
            "originator": "MySender"
        });
        let result: Result<MessageBirdConfig, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_deser_empty_object_fails() {
        let json = serde_json::json!({});
        let result: Result<MessageBirdConfig, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // truncate() tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_truncate_shorter_than_max() {
        let s = "hello";
        let result = truncate(s, 120);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_exactly_max_len() {
        // String of exactly 10 chars with max_len=10 should NOT be truncated
        let s = "1234567890";
        let result = truncate(s, 10);
        assert_eq!(result, "1234567890");
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn test_truncate_longer_than_max() {
        // 15 chars, max_len=10 → first 7 chars + "..."
        let s = "abcdefghijklmno";
        let result = truncate(s, 10);
        assert_eq!(result, "abcdefg...");
        assert_eq!(result.len(), 10);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_empty_string() {
        let result = truncate("", 10);
        assert_eq!(result, "");
    }

    #[test]
    fn test_truncate_just_over_max() {
        // 11 chars, max_len=10 → first 7 + "..."
        let s = "12345678901";
        let result = truncate(s, 10);
        assert_eq!(result, "1234567...");
        assert_eq!(result.len(), 10);
    }

    // -------------------------------------------------------------------------
    // from_config() tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_from_config_success_with_originator() {
        let config = serde_json::json!({
            "api_key": "live-key-abc",
            "originator": "CustomSender"
        });
        let provider = MessageBirdProvider::from_config(&config);
        assert!(provider.is_ok());
        let p = provider.unwrap();
        assert_eq!(p.api_key, "live-key-abc");
        assert_eq!(p.originator, "CustomSender");
    }

    #[test]
    fn test_from_config_success_without_originator_defaults_to_kubarr() {
        let config = serde_json::json!({
            "api_key": "live-key-xyz"
        });
        let provider = MessageBirdProvider::from_config(&config);
        assert!(provider.is_ok());
        let p = provider.unwrap();
        assert_eq!(p.api_key, "live-key-xyz");
        assert_eq!(p.originator, "Kubarr");
    }

    #[test]
    fn test_from_config_invalid_json_returns_err() {
        // A non-object value (e.g. a string) will fail deserialization
        let config = serde_json::json!("not-an-object");
        let result = MessageBirdProvider::from_config(&config);
        assert!(result.is_err());
        let err = result.err().expect("expected Err");
        assert!(err.contains("Invalid MessageBird config"));
    }

    #[test]
    fn test_from_config_missing_api_key_returns_err() {
        let config = serde_json::json!({
            "originator": "Sender"
        });
        let result = MessageBirdProvider::from_config(&config);
        assert!(result.is_err());
        let err = result.err().expect("expected Err");
        assert!(err.contains("Invalid MessageBird config"));
    }
}
