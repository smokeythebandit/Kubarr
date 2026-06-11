use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ChartsConfig {
    pub dir: PathBuf,
    pub repo: String,
    pub registry: String,
    pub sync_interval: u64,
    pub git_ref: String,
}

impl ChartsConfig {
    pub fn from_env() -> Self {
        Self {
            dir: PathBuf::from(
                env::var("KUBARR_CHARTS_DIR").unwrap_or_else(|_| "/app/charts".to_string()),
            ),
            repo: env::var("KUBARR_CHARTS_REPO")
                .unwrap_or_else(|_| "smokeythebandit/kubarr-charts".to_string()),
            registry: env::var("KUBARR_CHARTS_REGISTRY")
                .unwrap_or_else(|_| "oci://ghcr.io/smokeythebandit/kubarr-charts".to_string()),
            sync_interval: env::var("KUBARR_CHARTS_SYNC_INTERVAL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3600),
            git_ref: env::var("KUBARR_CHARTS_GIT_REF").unwrap_or_else(|_| "main".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charts_config_does_not_panic() {
        let _cfg = ChartsConfig::from_env();
    }

    #[test]
    fn charts_config_default_git_ref() {
        if std::env::var("KUBARR_CHARTS_GIT_REF").is_err() {
            let cfg = ChartsConfig::from_env();
            assert_eq!(cfg.git_ref, "main");
        }
    }

    #[test]
    fn charts_config_default_sync_interval() {
        if std::env::var("KUBARR_CHARTS_SYNC_INTERVAL").is_err() {
            let cfg = ChartsConfig::from_env();
            assert_eq!(cfg.sync_interval, 3600);
        }
    }

    #[test]
    fn charts_config_default_repo() {
        if std::env::var("KUBARR_CHARTS_REPO").is_err() {
            let cfg = ChartsConfig::from_env();
            assert!(!cfg.repo.is_empty());
        }
    }
}
