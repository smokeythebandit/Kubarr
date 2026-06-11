use std::env;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        Self {
            host: env::var("KUBARR_API_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("KUBARR_API_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8000),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_config_does_not_panic() {
        let _cfg = ServerConfig::from_env();
    }

    #[test]
    fn server_config_default_host() {
        if std::env::var("KUBARR_API_HOST").is_err() {
            let cfg = ServerConfig::from_env();
            assert_eq!(cfg.host, "0.0.0.0");
        }
    }

    #[test]
    fn server_config_default_port() {
        if std::env::var("KUBARR_API_PORT").is_err() {
            let cfg = ServerConfig::from_env();
            assert_eq!(cfg.port, 8000);
        }
    }
}
