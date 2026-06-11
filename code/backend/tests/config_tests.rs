//! Tests for configuration module

use kubarr::config::{Config, CONFIG};

#[test]
fn test_config_defaults() {
    // Create a config with defaults (env vars not set).
    // Config uses nested sub-configs: server, database, kubernetes, auth, charts.
    let config = Config::from_env();

    assert_eq!(config.server.host, "0.0.0.0");
    assert_eq!(config.server.port, 8000);
    assert_eq!(config.kubernetes.default_namespace, "media");
    assert!(!config.auth.oauth2_enabled);
    assert!(!config.kubernetes.in_cluster);
}

#[test]
fn test_database_url_format() {
    let config = Config::from_env();

    // Database URL should be a postgres URL by default
    assert!(
        config.database.database_url.starts_with("postgres://"),
        "Expected postgres URL, got: {}",
        config.database.database_url
    );
}

#[test]
fn test_version_from_cargo() {
    let config = Config::from_env();
    // Version should be set from Cargo.toml
    assert!(!config.version.is_empty());
    assert!(config.version.contains('.'));
}

#[test]
fn test_config_clone() {
    let config1 = Config::from_env();
    let config2 = config1.clone();

    assert_eq!(config1.server.host, config2.server.host);
    assert_eq!(config1.server.port, config2.server.port);
    assert_eq!(config1.database.database_url, config2.database.database_url);
}

#[test]
fn test_config_debug() {
    let config = Config::from_env();
    let debug_str = format!("{:?}", config);

    // Debug output should contain field names from nested sub-configs
    assert!(debug_str.contains("host"));
    assert!(debug_str.contains("port"));
    assert!(debug_str.contains("database_url"));
}

#[test]
fn test_path_types() {
    let config = Config::from_env();

    // charts.dir should be a PathBuf
    assert!(config.charts.dir.to_str().is_some());
}

#[test]
fn test_lazy_static_config() {
    // Access the global CONFIG using the nested sub-config structure
    let _ = &CONFIG.server.host;
    let _ = &CONFIG.server.port;

    // CONFIG should be initialized with a non-empty host
    assert!(!CONFIG.server.host.is_empty());
}
