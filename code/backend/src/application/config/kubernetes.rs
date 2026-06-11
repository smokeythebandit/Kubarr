use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct KubernetesConfig {
    pub kubeconfig_path: Option<PathBuf>,
    pub in_cluster: bool,
    pub default_namespace: String,
}

impl KubernetesConfig {
    pub fn from_env() -> Self {
        Self {
            kubeconfig_path: env::var("KUBARR_KUBECONFIG_PATH").ok().map(PathBuf::from),
            in_cluster: env::var("KUBARR_IN_CLUSTER")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            default_namespace: env::var("KUBARR_DEFAULT_NAMESPACE")
                .unwrap_or_else(|_| "media".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kubernetes_config_does_not_panic() {
        let _cfg = KubernetesConfig::from_env();
    }

    #[test]
    fn kubernetes_config_not_in_cluster_by_default() {
        if std::env::var("KUBARR_IN_CLUSTER").is_err() {
            let cfg = KubernetesConfig::from_env();
            assert!(!cfg.in_cluster);
        }
    }

    #[test]
    fn kubernetes_config_default_namespace() {
        if std::env::var("KUBARR_DEFAULT_NAMESPACE").is_err() {
            let cfg = KubernetesConfig::from_env();
            assert_eq!(cfg.default_namespace, "media");
        }
    }

    #[test]
    fn kubernetes_config_no_kubeconfig_by_default() {
        if std::env::var("KUBARR_KUBECONFIG_PATH").is_err() {
            let cfg = KubernetesConfig::from_env();
            assert!(cfg.kubeconfig_path.is_none());
        }
    }
}
