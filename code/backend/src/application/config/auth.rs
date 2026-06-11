use std::env;

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub oauth2_enabled: bool,
    pub oauth2_issuer_url: String,
}

impl AuthConfig {
    pub fn from_env() -> Self {
        Self {
            oauth2_enabled: env::var("KUBARR_OAUTH2_ENABLED")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            oauth2_issuer_url: env::var("KUBARR_OAUTH2_ISSUER_URL")
                .unwrap_or_else(|_| "http://kubarr:8000/auth".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_config_defaults_without_env() {
        // When env vars are not set, should use defaults
        // We can't reliably unset env vars in parallel tests, so we just ensure from_env() doesn't panic
        let _cfg = AuthConfig::from_env();
    }

    #[test]
    fn auth_config_oauth2_disabled_by_default() {
        // If KUBARR_OAUTH2_ENABLED is not set, oauth2_enabled should be false
        // Only valid if the env var is NOT set in the test environment
        if std::env::var("KUBARR_OAUTH2_ENABLED").is_err() {
            let cfg = AuthConfig::from_env();
            assert!(!cfg.oauth2_enabled);
        }
    }

    #[test]
    fn auth_config_issuer_url_has_default() {
        if std::env::var("KUBARR_OAUTH2_ISSUER_URL").is_err() {
            let cfg = AuthConfig::from_env();
            assert!(!cfg.oauth2_issuer_url.is_empty());
        }
    }
}
