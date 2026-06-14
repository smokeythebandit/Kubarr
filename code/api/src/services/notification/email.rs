use async_trait::async_trait;
use lettre::{
    message::header::ContentType, transport::smtp::authentication::Credentials, AsyncSmtpTransport,
    AsyncTransport, Message, Tokio1Executor,
};
use serde::Deserialize;

use super::{ChannelType, NotificationMessage, NotificationProvider, SendResult};

#[derive(Debug, Deserialize)]
pub struct EmailConfig {
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    pub password: String,
    pub from_address: String,
    pub from_name: Option<String>,
    pub use_tls: Option<bool>,
}

pub struct EmailProvider {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from_address: String,
    from_name: String,
}

impl EmailProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self, String> {
        let email_config: EmailConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("Invalid email config: {}", e))?;

        let creds = Credentials::new(email_config.username, email_config.password);

        let use_tls = email_config.use_tls.unwrap_or(true);

        let transport = if use_tls {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&email_config.smtp_host)
                .map_err(|e| format!("Failed to create SMTP transport: {}", e))?
                .port(email_config.smtp_port)
                .credentials(creds)
                .build()
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&email_config.smtp_host)
                .port(email_config.smtp_port)
                .credentials(creds)
                .build()
        };

        Ok(Self {
            transport,
            from_address: email_config.from_address,
            from_name: email_config
                .from_name
                .unwrap_or_else(|| "Kubarr".to_string()),
        })
    }

    async fn send_email(&self, to: &str, subject: &str, body: &str) -> SendResult {
        let from = format!("{} <{}>", self.from_name, self.from_address);

        let to_mailbox = match to.parse() {
            Ok(mbox) => mbox,
            Err(_) => {
                return SendResult {
                    success: false,
                    error: Some("Invalid recipient email address".to_string()),
                }
            }
        };

        let from_mailbox = match from.parse() {
            Ok(mbox) => mbox,
            Err(_) => match self.from_address.parse() {
                Ok(mbox) => mbox,
                Err(_) => {
                    return SendResult {
                        success: false,
                        error: Some("Invalid from email address".to_string()),
                    }
                }
            },
        };

        let email = match Message::builder()
            .from(from_mailbox)
            .to(to_mailbox)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body.to_string())
        {
            Ok(email) => email,
            Err(e) => {
                return SendResult {
                    success: false,
                    error: Some(format!("Failed to build email: {}", e)),
                }
            }
        };

        match self.transport.send(email).await {
            Ok(_) => SendResult {
                success: true,
                error: None,
            },
            Err(e) => SendResult {
                success: false,
                error: Some(format!("Failed to send email: {}", e)),
            },
        }
    }
}

#[async_trait]
impl NotificationProvider for EmailProvider {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Email
    }

    async fn send(&self, message: &NotificationMessage) -> SendResult {
        let subject = &message.title;
        let body = format!(
            "{}\n\n---\nSeverity: {}\nSent by Kubarr Notification System",
            message.body,
            message.severity.as_str()
        );

        self.send_email(&message.recipient, subject, &body).await
    }

    async fn test(&self, destination: &str) -> SendResult {
        self.send_email(
            destination,
            "Kubarr Test Notification",
            "This is a test notification from Kubarr.\n\nIf you received this email, your email notifications are configured correctly!",
        ).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // EmailConfig deserialization tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_config_deser_all_fields() {
        let json = serde_json::json!({
            "smtp_host": "smtp.example.com",
            "smtp_port": 587,
            "username": "user@example.com",
            "password": "secret",
            "from_address": "noreply@example.com",
            "from_name": "Kubarr Bot",
            "use_tls": true
        });
        let cfg: EmailConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.smtp_host, "smtp.example.com");
        assert_eq!(cfg.smtp_port, 587);
        assert_eq!(cfg.username, "user@example.com");
        assert_eq!(cfg.password, "secret");
        assert_eq!(cfg.from_address, "noreply@example.com");
        assert_eq!(cfg.from_name, Some("Kubarr Bot".to_string()));
        assert_eq!(cfg.use_tls, Some(true));
    }

    #[test]
    fn test_config_deser_optional_fields_absent() {
        let json = serde_json::json!({
            "smtp_host": "smtp.example.com",
            "smtp_port": 25,
            "username": "user",
            "password": "pass",
            "from_address": "bot@example.com"
        });
        let cfg: EmailConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.from_name, None);
        assert_eq!(cfg.use_tls, None);
    }

    #[test]
    fn test_config_deser_missing_smtp_host_fails() {
        let json = serde_json::json!({
            "smtp_port": 587,
            "username": "user",
            "password": "pass",
            "from_address": "bot@example.com"
        });
        let result: Result<EmailConfig, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_deser_missing_smtp_port_fails() {
        let json = serde_json::json!({
            "smtp_host": "smtp.example.com",
            "username": "user",
            "password": "pass",
            "from_address": "bot@example.com"
        });
        let result: Result<EmailConfig, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_deser_missing_username_fails() {
        let json = serde_json::json!({
            "smtp_host": "smtp.example.com",
            "smtp_port": 587,
            "password": "pass",
            "from_address": "bot@example.com"
        });
        let result: Result<EmailConfig, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_deser_missing_password_fails() {
        let json = serde_json::json!({
            "smtp_host": "smtp.example.com",
            "smtp_port": 587,
            "username": "user",
            "from_address": "bot@example.com"
        });
        let result: Result<EmailConfig, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_deser_missing_from_address_fails() {
        let json = serde_json::json!({
            "smtp_host": "smtp.example.com",
            "smtp_port": 587,
            "username": "user",
            "password": "pass"
        });
        let result: Result<EmailConfig, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // EmailProvider::from_config() tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_from_config_success_all_fields_with_tls() {
        let config = serde_json::json!({
            "smtp_host": "smtp.gmail.com",
            "smtp_port": 587,
            "username": "user@gmail.com",
            "password": "apppassword",
            "from_address": "user@gmail.com",
            "from_name": "My App",
            "use_tls": true
        });
        let result = EmailProvider::from_config(&config);
        assert!(result.is_ok());
        let provider = result.unwrap();
        assert_eq!(provider.from_address, "user@gmail.com");
        assert_eq!(provider.from_name, "My App");
    }

    #[test]
    fn test_from_config_success_no_tls() {
        let config = serde_json::json!({
            "smtp_host": "localhost",
            "smtp_port": 25,
            "username": "user",
            "password": "pass",
            "from_address": "noreply@localhost",
            "use_tls": false
        });
        let result = EmailProvider::from_config(&config);
        assert!(result.is_ok());
        let provider = result.unwrap();
        assert_eq!(provider.from_address, "noreply@localhost");
        // from_name should default to "Kubarr" when not specified
        assert_eq!(provider.from_name, "Kubarr");
    }

    #[test]
    fn test_from_config_success_default_from_name() {
        let config = serde_json::json!({
            "smtp_host": "smtp.example.com",
            "smtp_port": 587,
            "username": "user",
            "password": "pass",
            "from_address": "bot@example.com"
        });
        let result = EmailProvider::from_config(&config);
        assert!(result.is_ok());
        let provider = result.unwrap();
        assert_eq!(provider.from_name, "Kubarr");
    }

    #[test]
    fn test_from_config_missing_required_field_returns_err() {
        // Missing from_address — required field
        let config = serde_json::json!({
            "smtp_host": "smtp.example.com",
            "smtp_port": 587,
            "username": "user",
            "password": "pass"
        });
        let result = EmailProvider::from_config(&config);
        assert!(result.is_err());
        let err = result.err().expect("expected Err");
        assert!(err.contains("Invalid email config"));
    }

    #[test]
    fn test_from_config_invalid_json_value_returns_err() {
        // A non-object value triggers deserialization failure
        let config = serde_json::json!("this-is-not-an-object");
        let result = EmailProvider::from_config(&config);
        assert!(result.is_err());
        let err = result.err().expect("expected Err");
        assert!(err.contains("Invalid email config"));
    }

    #[test]
    fn test_from_config_use_tls_defaults_to_true_when_absent() {
        // When use_tls is absent the code defaults to true (starttls_relay path).
        // We just verify it builds successfully without panicking.
        let config = serde_json::json!({
            "smtp_host": "smtp.example.com",
            "smtp_port": 587,
            "username": "user",
            "password": "pass",
            "from_address": "bot@example.com"
        });
        let result = EmailProvider::from_config(&config);
        assert!(result.is_ok());
    }

    // -------------------------------------------------------------------------
    // channel_type, send, test — trait impl coverage
    // -------------------------------------------------------------------------

    fn make_provider() -> EmailProvider {
        EmailProvider::from_config(&serde_json::json!({
            "smtp_host": "smtp.example.com",
            "smtp_port": 587,
            "username": "user",
            "password": "pass",
            "from_address": "bot@example.com",
            "from_name": "Kubarr"
        }))
        .expect("build provider")
    }

    #[test]
    fn channel_type_returns_email() {
        let provider = make_provider();
        assert_eq!(provider.channel_type(), ChannelType::Email);
    }

    #[tokio::test]
    async fn send_email_with_invalid_to_returns_error() {
        let provider = make_provider();
        let result = provider.send_email("not-an-email", "Subject", "Body").await;
        assert!(!result.success, "invalid email should fail");
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains("invalid")
                || result
                    .error
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains("address"),
            "error message should mention invalid address: {:?}",
            result.error
        );
    }

    #[tokio::test]
    async fn send_calls_send_email_with_message_fields() {
        let provider = make_provider();
        let msg = NotificationMessage {
            recipient: "not-an-email".to_string(),
            title: "Test Title".to_string(),
            body: "Test body text".to_string(),
            severity: crate::services::notification::NotificationSeverity::Info,
        };
        // Will fail at SMTP transport since recipient is invalid — covers send() lines
        let result = provider.send(&msg).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_method_calls_send_email_with_test_subject() {
        let provider = make_provider();
        // invalid destination covers test() lines
        let result = provider.test("not-an-email").await;
        assert!(!result.success);
    }

    // -------------------------------------------------------------------------
    // Cover lines 71-110: valid recipient → SMTP transport fails (connection refused)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn send_email_valid_to_smtp_connection_fails() {
        // Use a provider pointing to a server that will refuse connections,
        // but with valid email addresses so we reach the SMTP transport call.
        let provider = EmailProvider::from_config(&serde_json::json!({
            "smtp_host": "127.0.0.1",
            "smtp_port": 19999, // unlikely to be open
            "username": "u",
            "password": "p",
            "from_address": "from@example.com",
            "from_name": "Test",
            "use_tls": false
        }))
        .expect("provider");

        let result = provider
            .send_email("to@example.com", "Subject", "Body text")
            .await;
        // Should fail because SMTP port is not open, but covers the code path
        // through valid-address parsing and email building
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[tokio::test]
    async fn send_with_valid_recipient_reaches_smtp() {
        // Same provider with valid recipient to cover send() → send_email() path
        let provider = EmailProvider::from_config(&serde_json::json!({
            "smtp_host": "127.0.0.1",
            "smtp_port": 19999,
            "username": "u",
            "password": "p",
            "from_address": "from@example.com",
            "from_name": "Test",
            "use_tls": false
        }))
        .expect("provider");

        let msg = NotificationMessage {
            recipient: "to@example.com".to_string(),
            title: "Test".to_string(),
            body: "Body".to_string(),
            severity: crate::services::notification::NotificationSeverity::Warning,
        };
        let result = provider.send(&msg).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_with_valid_destination_reaches_smtp() {
        let provider = EmailProvider::from_config(&serde_json::json!({
            "smtp_host": "127.0.0.1",
            "smtp_port": 19999,
            "username": "u",
            "password": "p",
            "from_address": "from@example.com",
            "use_tls": false
        }))
        .expect("provider");

        let result = provider.test("to@example.com").await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn send_email_invalid_from_address_returns_error() {
        // Create a provider with an invalid from_address to cover the
        // inner fallback path (lines 73-80) where from_address.parse() also fails
        let provider = EmailProvider::from_config(&serde_json::json!({
            "smtp_host": "127.0.0.1",
            "smtp_port": 19999,
            "username": "u",
            "password": "p",
            "from_address": "not-valid-email!!",
            "from_name": "Test",
            "use_tls": false
        }))
        .expect("provider");

        let result = provider
            .send_email("to@example.com", "Subject", "Body")
            .await;
        // Should fail due to invalid from address
        assert!(!result.success);
    }
}
