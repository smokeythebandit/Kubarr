//! Tests for notification provider construction and utility functions.
//!
//! Covers:
//! - `ChannelType::as_str()`, `parse()`, `all()`, `Display`
//! - `NotificationSeverity::as_str()`, `parse()`, `Display`
//! - `EmailProvider::from_config()` — success and error paths
//! - `TelegramProvider::from_config()` — success and error paths
//! - `MessageBirdProvider::from_config()` — success path (with and without originator) and error

use kubarr::services::notification::{
    ChannelType, EmailProvider, MessageBirdProvider, NotificationSeverity, TelegramProvider,
};

// ============================================================================
// ChannelType
// ============================================================================

#[test]
fn test_channel_type_as_str_email() {
    assert_eq!(ChannelType::Email.as_str(), "email");
}

#[test]
fn test_channel_type_as_str_telegram() {
    assert_eq!(ChannelType::Telegram.as_str(), "telegram");
}

#[test]
fn test_channel_type_as_str_messagebird() {
    assert_eq!(ChannelType::MessageBird.as_str(), "messagebird");
}

#[test]
fn test_channel_type_parse_email() {
    assert_eq!(ChannelType::parse("email"), Some(ChannelType::Email));
}

#[test]
fn test_channel_type_parse_telegram() {
    assert_eq!(ChannelType::parse("telegram"), Some(ChannelType::Telegram));
}

#[test]
fn test_channel_type_parse_messagebird() {
    assert_eq!(
        ChannelType::parse("messagebird"),
        Some(ChannelType::MessageBird)
    );
}

#[test]
fn test_channel_type_parse_uppercase() {
    assert_eq!(ChannelType::parse("EMAIL"), Some(ChannelType::Email));
    assert_eq!(ChannelType::parse("TELEGRAM"), Some(ChannelType::Telegram));
}

#[test]
fn test_channel_type_parse_unknown_returns_none() {
    assert_eq!(ChannelType::parse("signal"), None);
    assert_eq!(ChannelType::parse(""), None);
    assert_eq!(ChannelType::parse("webhook"), None);
}

#[test]
fn test_channel_type_all_returns_all_three() {
    let all = ChannelType::all();
    assert_eq!(all.len(), 3);
    assert!(all.contains(&ChannelType::Email));
    assert!(all.contains(&ChannelType::Telegram));
    assert!(all.contains(&ChannelType::MessageBird));
}

#[test]
fn test_channel_type_display_email() {
    let s = format!("{}", ChannelType::Email);
    assert_eq!(s, "email");
}

#[test]
fn test_channel_type_display_telegram() {
    let s = format!("{}", ChannelType::Telegram);
    assert_eq!(s, "telegram");
}

#[test]
fn test_channel_type_display_messagebird() {
    let s = format!("{}", ChannelType::MessageBird);
    assert_eq!(s, "messagebird");
}

// ============================================================================
// NotificationSeverity
// ============================================================================

#[test]
fn test_severity_as_str_info() {
    assert_eq!(NotificationSeverity::Info.as_str(), "info");
}

#[test]
fn test_severity_as_str_warning() {
    assert_eq!(NotificationSeverity::Warning.as_str(), "warning");
}

#[test]
fn test_severity_as_str_critical() {
    assert_eq!(NotificationSeverity::Critical.as_str(), "critical");
}

#[test]
fn test_severity_parse_warning() {
    assert_eq!(
        NotificationSeverity::parse("warning"),
        NotificationSeverity::Warning
    );
}

#[test]
fn test_severity_parse_critical() {
    assert_eq!(
        NotificationSeverity::parse("critical"),
        NotificationSeverity::Critical
    );
}

#[test]
fn test_severity_parse_defaults_to_info() {
    assert_eq!(
        NotificationSeverity::parse("info"),
        NotificationSeverity::Info
    );
    assert_eq!(
        NotificationSeverity::parse("unknown"),
        NotificationSeverity::Info
    );
    assert_eq!(NotificationSeverity::parse(""), NotificationSeverity::Info);
}

#[test]
fn test_severity_parse_case_insensitive() {
    assert_eq!(
        NotificationSeverity::parse("WARNING"),
        NotificationSeverity::Warning
    );
    assert_eq!(
        NotificationSeverity::parse("CRITICAL"),
        NotificationSeverity::Critical
    );
}

#[test]
fn test_severity_display_info() {
    let s = format!("{}", NotificationSeverity::Info);
    assert_eq!(s, "info");
}

#[test]
fn test_severity_display_warning() {
    let s = format!("{}", NotificationSeverity::Warning);
    assert_eq!(s, "warning");
}

#[test]
fn test_severity_display_critical() {
    let s = format!("{}", NotificationSeverity::Critical);
    assert_eq!(s, "critical");
}

// ============================================================================
// EmailProvider::from_config
// ============================================================================

#[test]
fn test_email_provider_from_config_success() {
    let config = serde_json::json!({
        "smtp_host": "smtp.example.com",
        "smtp_port": 587,
        "username": "user@example.com",
        "password": "password123",
        "from_address": "noreply@example.com",
        "from_name": "Kubarr",
        "use_tls": true
    });

    let result = EmailProvider::from_config(&config);
    assert!(
        result.is_ok(),
        "EmailProvider::from_config with valid config must succeed: {:?}",
        result.err()
    );
}

#[test]
fn test_email_provider_from_config_no_tls() {
    let config = serde_json::json!({
        "smtp_host": "smtp.example.com",
        "smtp_port": 25,
        "username": "user",
        "password": "pass",
        "from_address": "test@example.com",
        "use_tls": false
    });

    let result = EmailProvider::from_config(&config);
    assert!(
        result.is_ok(),
        "EmailProvider::from_config with use_tls=false must succeed"
    );
}

#[test]
fn test_email_provider_from_config_default_from_name() {
    let config = serde_json::json!({
        "smtp_host": "smtp.example.com",
        "smtp_port": 587,
        "username": "u",
        "password": "p",
        "from_address": "noreply@example.com"
        // no from_name — should default to "Kubarr"
    });

    let result = EmailProvider::from_config(&config);
    assert!(result.is_ok(), "missing from_name must use default");
}

#[test]
fn test_email_provider_from_config_invalid_json_returns_error() {
    let config = serde_json::json!({
        "wrong_field": "value"
        // missing required fields
    });

    let result = EmailProvider::from_config(&config);
    assert!(
        result.is_err(),
        "EmailProvider::from_config with missing required fields must fail"
    );
    let err = result.err().unwrap();
    assert!(
        err.contains("Invalid email config"),
        "Error must mention 'Invalid email config', got: {}",
        err
    );
}

// ============================================================================
// TelegramProvider::from_config
// ============================================================================

#[test]
fn test_telegram_provider_from_config_success() {
    let config = serde_json::json!({
        "bot_token": "123456:AABBCCDDEEFF"
    });

    let result = TelegramProvider::from_config(&config);
    assert!(
        result.is_ok(),
        "TelegramProvider::from_config with valid config must succeed"
    );
}

#[test]
fn test_telegram_provider_from_config_missing_token_returns_error() {
    let config = serde_json::json!({
        "wrong_key": "value"
    });

    let result = TelegramProvider::from_config(&config);
    assert!(
        result.is_err(),
        "TelegramProvider::from_config without bot_token must fail"
    );
    let err = result.err().unwrap();
    assert!(
        err.contains("Invalid Telegram config"),
        "Error must mention 'Invalid Telegram config', got: {}",
        err
    );
}

// ============================================================================
// MessageBirdProvider::from_config
// ============================================================================

#[test]
fn test_messagebird_provider_from_config_success_with_originator() {
    let config = serde_json::json!({
        "api_key": "fake",
        "originator": "Kubarr"
    });

    let result = MessageBirdProvider::from_config(&config);
    assert!(
        result.is_ok(),
        "MessageBirdProvider::from_config with originator must succeed"
    );
}

#[test]
fn test_messagebird_provider_from_config_success_without_originator() {
    let config = serde_json::json!({
        "api_key": "fake"
        // no originator — should default to "Kubarr"
    });

    let result = MessageBirdProvider::from_config(&config);
    assert!(
        result.is_ok(),
        "MessageBirdProvider::from_config without originator must succeed with default"
    );
}

#[test]
fn test_messagebird_provider_from_config_missing_api_key_returns_error() {
    let config = serde_json::json!({
        "originator": "Kubarr"
        // missing api_key
    });

    let result = MessageBirdProvider::from_config(&config);
    assert!(
        result.is_err(),
        "MessageBirdProvider::from_config without api_key must fail"
    );
    let err = result.err().unwrap();
    assert!(
        err.contains("Invalid MessageBird config"),
        "Error must mention 'Invalid MessageBird config', got: {}",
        err
    );
}
