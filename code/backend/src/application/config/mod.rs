pub mod auth;
pub mod charts;
pub mod database;
pub mod kubernetes;
pub mod server;

use once_cell::sync::Lazy;
use std::env;

/// Application configuration loaded from environment variables
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub server: server::ServerConfig,
    pub database: database::DatabaseConfig,
    pub kubernetes: kubernetes::KubernetesConfig,
    pub auth: auth::AuthConfig,
    pub charts: charts::ChartsConfig,

    // Build info
    pub commit_hash: String,
    pub build_time: String,
    pub version: String,
    pub channel: String,

    // Logging
    pub log_level: String,

    // Frontend proxy
    pub frontend_url: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            server: server::ServerConfig::from_env(),
            database: database::DatabaseConfig::from_env(),
            kubernetes: kubernetes::KubernetesConfig::from_env(),
            auth: auth::AuthConfig::from_env(),
            charts: charts::ChartsConfig::from_env(),

            // Build info
            commit_hash: env::var("COMMIT_HASH").unwrap_or_else(|_| "unknown".to_string()),
            build_time: env::var("BUILD_TIME").unwrap_or_else(|_| "unknown".to_string()),
            version: env!("CARGO_PKG_VERSION").to_string(),
            channel: env::var("CHANNEL").unwrap_or_else(|_| "dev".to_string()),

            // Logging
            log_level: env::var("KUBARR_LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),

            // Frontend proxy
            frontend_url: env::var("KUBARR_FRONTEND_URL").unwrap_or_else(|_| {
                if env::var("KUBARR_IN_CLUSTER")
                    .map(|v| v.to_lowercase() == "true")
                    .unwrap_or(false)
                {
                    "http://kubarr-frontend.kubarr.svc.cluster.local:80".to_string()
                } else {
                    "http://localhost:3000".to_string()
                }
            }),
        }
    }
}

pub static CONFIG: Lazy<Config> = Lazy::new(Config::from_env);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_from_env_does_not_panic() {
        let _cfg = Config::from_env();
    }

    #[test]
    fn config_version_is_not_empty() {
        let cfg = Config::from_env();
        assert!(!cfg.version.is_empty());
    }

    #[test]
    fn config_default_channel_is_dev() {
        if std::env::var("CHANNEL").is_err() {
            let cfg = Config::from_env();
            assert_eq!(cfg.channel, "dev");
        }
    }

    #[test]
    fn config_default_log_level_is_info() {
        if std::env::var("KUBARR_LOG_LEVEL").is_err() {
            let cfg = Config::from_env();
            assert_eq!(cfg.log_level, "info");
        }
    }

    #[test]
    fn config_frontend_url_not_empty() {
        let cfg = Config::from_env();
        assert!(!cfg.frontend_url.is_empty());
    }
}
