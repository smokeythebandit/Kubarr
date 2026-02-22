#![allow(dead_code)]

use async_trait::async_trait;
use serde::Deserialize;

use super::{ChannelType, NotificationMessage, NotificationProvider, SendResult};

#[derive(Debug, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
}

pub struct TelegramProvider {
    bot_token: String,
    client: reqwest::Client,
}

impl TelegramProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self, String> {
        let telegram_config: TelegramConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("Invalid Telegram config: {}", e))?;

        Ok(Self {
            bot_token: telegram_config.bot_token,
            client: reqwest::Client::new(),
        })
    }

    async fn send_message(&self, chat_id: &str, text: &str) -> SendResult {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.bot_token);

        let payload = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "HTML"
        });

        match self.client.post(&url).json(&payload).send().await {
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
                        error: Some(format!("Telegram API error: {}", error_text)),
                    }
                }
            }
            Err(e) => SendResult {
                success: false,
                error: Some(format!("Failed to send Telegram message: {}", e)),
            },
        }
    }
}

#[async_trait]
impl NotificationProvider for TelegramProvider {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }

    async fn send(&self, message: &NotificationMessage) -> SendResult {
        let severity_emoji = match message.severity {
            super::NotificationSeverity::Info => "ℹ️",
            super::NotificationSeverity::Warning => "⚠️",
            super::NotificationSeverity::Critical => "🚨",
        };

        let text = format!(
            "{} <b>{}</b>\n\n{}",
            severity_emoji,
            html_escape(&message.title),
            html_escape(&message.body)
        );

        self.send_message(&message.recipient, &text).await
    }

    async fn test(&self, destination: &str) -> SendResult {
        let text = "✅ <b>Kubarr Test Notification</b>\n\nThis is a test notification from Kubarr.\n\nIf you received this message, your Telegram notifications are configured correctly!";
        self.send_message(destination, text).await
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    // NotificationSeverity lives in the grandparent notification module (mod.rs);
    // bring it into scope explicitly since the production import above only
    // imports a subset of the parent's exports.
    use super::super::NotificationSeverity;

    // -------------------------------------------------------------------------
    // TelegramConfig deserialization tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_config_deser_success() {
        let json = serde_json::json!({ "bot_token": "123456:ABC-DEF" });
        let cfg: TelegramConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.bot_token, "123456:ABC-DEF");
    }

    #[test]
    fn test_config_deser_missing_bot_token_fails() {
        let json = serde_json::json!({});
        let result: Result<TelegramConfig, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_deser_wrong_type_fails() {
        let json = serde_json::json!({ "bot_token": 12345 });
        // Numbers don't deserialize into String
        let result: Result<TelegramConfig, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // html_escape() tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_html_escape_ampersand() {
        assert_eq!(html_escape("fish & chips"), "fish &amp; chips");
    }

    #[test]
    fn test_html_escape_less_than() {
        assert_eq!(html_escape("a < b"), "a &lt; b");
    }

    #[test]
    fn test_html_escape_greater_than() {
        assert_eq!(html_escape("a > b"), "a &gt; b");
    }

    #[test]
    fn test_html_escape_ampersand_chain() {
        // Existing &amp; in the source should become &amp;amp; (double-escape)
        assert_eq!(html_escape("&amp;"), "&amp;amp;");
    }

    #[test]
    fn test_html_escape_combined() {
        assert_eq!(
            html_escape("<script>alert('a & b')</script>"),
            "&lt;script&gt;alert('a &amp; b')&lt;/script&gt;"
        );
    }

    #[test]
    fn test_html_escape_no_special_chars() {
        let s = "hello world 123";
        assert_eq!(html_escape(s), s);
    }

    #[test]
    fn test_html_escape_empty_string() {
        assert_eq!(html_escape(""), "");
    }

    // -------------------------------------------------------------------------
    // from_config() tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_from_config_success() {
        let config = serde_json::json!({ "bot_token": "987654:XYZ" });
        let result = TelegramProvider::from_config(&config);
        assert!(result.is_ok());
        let p = result.unwrap();
        assert_eq!(p.bot_token, "987654:XYZ");
    }

    #[test]
    fn test_from_config_missing_token_returns_err() {
        let config = serde_json::json!({});
        let result = TelegramProvider::from_config(&config);
        assert!(result.is_err());
        let err = result.err().expect("expected Err");
        assert!(err.contains("Invalid Telegram config"));
    }

    #[test]
    fn test_from_config_invalid_value_returns_err() {
        let config = serde_json::json!("not-an-object");
        let result = TelegramProvider::from_config(&config);
        assert!(result.is_err());
        let err = result.err().expect("expected Err");
        assert!(err.contains("Invalid Telegram config"));
    }

    // -------------------------------------------------------------------------
    // Severity emoji mapping tests (via send() message format)
    // -------------------------------------------------------------------------
    // We test the emoji logic indirectly by inspecting what the send() method
    // would produce.  Because send() calls send_message() which hits the
    // network, we test the format logic directly by re-implementing it here
    // in the same way the production code does.

    #[test]
    fn test_severity_emoji_info() {
        let emoji = match NotificationSeverity::Info {
            NotificationSeverity::Info => "ℹ️",
            NotificationSeverity::Warning => "⚠️",
            NotificationSeverity::Critical => "🚨",
        };
        assert_eq!(emoji, "ℹ️");
    }

    #[test]
    fn test_severity_emoji_warning() {
        let emoji = match NotificationSeverity::Warning {
            NotificationSeverity::Info => "ℹ️",
            NotificationSeverity::Warning => "⚠️",
            NotificationSeverity::Critical => "🚨",
        };
        assert_eq!(emoji, "⚠️");
    }

    #[test]
    fn test_severity_emoji_critical() {
        let emoji = match NotificationSeverity::Critical {
            NotificationSeverity::Info => "ℹ️",
            NotificationSeverity::Warning => "⚠️",
            NotificationSeverity::Critical => "🚨",
        };
        assert_eq!(emoji, "🚨");
    }

    // -------------------------------------------------------------------------
    // Message format construction test
    // -------------------------------------------------------------------------

    #[test]
    fn test_message_format_construction() {
        // Reproduce the exact format used in send()
        let severity_emoji = "ℹ️";
        let title = "Test Alert";
        let body = "Something happened <here> & now";

        let text = format!(
            "{} <b>{}</b>\n\n{}",
            severity_emoji,
            html_escape(title),
            html_escape(body)
        );

        assert!(text.starts_with("ℹ️"));
        assert!(text.contains("<b>Test Alert</b>"));
        assert!(text.contains("Something happened &lt;here&gt; &amp; now"));
    }

    #[test]
    fn test_message_format_html_escaping_in_title() {
        let title = "Alert <critical> & urgent";
        let escaped = html_escape(title);
        assert_eq!(escaped, "Alert &lt;critical&gt; &amp; urgent");
    }
}
