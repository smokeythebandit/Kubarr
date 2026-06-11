use std::env;

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub database_url: String,
}

impl DatabaseConfig {
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("KUBARR_DATABASE_URL")
                .or_else(|_| env::var("DATABASE_URL"))
                .unwrap_or_else(|_| "postgres://kubarr:kubarr@localhost:5432/kubarr".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_config_does_not_panic() {
        let _cfg = DatabaseConfig::from_env();
    }

    #[test]
    fn database_config_default_url() {
        if std::env::var("KUBARR_DATABASE_URL").is_err() && std::env::var("DATABASE_URL").is_err() {
            let cfg = DatabaseConfig::from_env();
            assert!(cfg.database_url.contains("postgres://"));
        }
    }
}
