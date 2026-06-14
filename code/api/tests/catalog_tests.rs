use std::collections::HashMap;

use kubarr::services::catalog::{AppCatalog, AppConfig, ResourceRequirements, VolumeConfig};

#[test]
fn test_app_config_serialization() {
    let config = AppConfig {
        name: "testapp".to_string(),
        display_name: "Test App".to_string(),
        description: "A test application".to_string(),
        icon: "📦".to_string(),
        container_image: "linuxserver/testapp:latest".to_string(),
        default_port: 8080,
        resource_requirements: ResourceRequirements {
            cpu_request: "100m".to_string(),
            cpu_limit: "1000m".to_string(),
            memory_request: "256Mi".to_string(),
            memory_limit: "1Gi".to_string(),
        },
        volumes: vec![VolumeConfig {
            name: "config".to_string(),
            mount_path: "/config".to_string(),
            size: "1Gi".to_string(),
        }],
        environment_variables: HashMap::new(),
        category: "media".to_string(),
        is_system: false,
        is_hidden: false,
        is_browseable: true,
    };

    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"name\":\"testapp\""));
    assert!(json.contains("\"display_name\":\"Test App\""));
    assert!(json.contains("\"category\":\"media\""));

    // Deserialize back
    let parsed: AppConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.name, config.name);
    assert_eq!(parsed.default_port, config.default_port);
}

#[test]
fn test_resource_requirements_serialization() {
    let resources = ResourceRequirements {
        cpu_request: "100m".to_string(),
        cpu_limit: "500m".to_string(),
        memory_request: "128Mi".to_string(),
        memory_limit: "512Mi".to_string(),
    };

    let json = serde_json::to_string(&resources).unwrap();
    assert!(json.contains("\"cpu_request\":\"100m\""));
    assert!(json.contains("\"memory_limit\":\"512Mi\""));
}

#[test]
fn test_volume_config_serialization() {
    let volume = VolumeConfig {
        name: "data".to_string(),
        mount_path: "/data".to_string(),
        size: "10Gi".to_string(),
    };

    let json = serde_json::to_string(&volume).unwrap();
    assert!(json.contains("\"name\":\"data\""));
    assert!(json.contains("\"mount_path\":\"/data\""));
    assert!(json.contains("\"size\":\"10Gi\""));
}

#[test]
fn test_app_catalog_empty() {
    let catalog = AppCatalog::with_apps(HashMap::new());

    assert!(catalog.get_all_apps().is_empty());
    assert!(catalog.get_app("nonexistent").is_none());
    assert!(catalog.get_categories().is_empty());
}

#[test]
fn test_app_catalog_get_app() {
    let mut apps = HashMap::new();
    apps.insert(
        "testapp".to_string(),
        AppConfig {
            name: "testapp".to_string(),
            display_name: "Test App".to_string(),
            description: "Test".to_string(),
            icon: "📦".to_string(),
            container_image: "test:latest".to_string(),
            default_port: 8080,
            resource_requirements: ResourceRequirements {
                cpu_request: "100m".to_string(),
                cpu_limit: "1000m".to_string(),
                memory_request: "256Mi".to_string(),
                memory_limit: "1Gi".to_string(),
            },
            volumes: vec![],
            environment_variables: HashMap::new(),
            category: "media".to_string(),
            is_system: false,
            is_hidden: false,
            is_browseable: true,
        },
    );

    let catalog = AppCatalog::with_apps(apps);

    assert!(catalog.get_app("testapp").is_some());
    assert!(catalog.get_app("TESTAPP").is_some()); // Case insensitive
    assert!(catalog.get_app("nonexistent").is_none());
}

#[test]
fn test_app_catalog_get_apps_by_category() {
    let mut apps = HashMap::new();

    apps.insert(
        "sonarr".to_string(),
        AppConfig {
            name: "sonarr".to_string(),
            display_name: "Sonarr".to_string(),
            description: "TV series manager".to_string(),
            icon: "📺".to_string(),
            container_image: "linuxserver/sonarr:latest".to_string(),
            default_port: 8989,
            resource_requirements: ResourceRequirements {
                cpu_request: "100m".to_string(),
                cpu_limit: "1000m".to_string(),
                memory_request: "256Mi".to_string(),
                memory_limit: "1Gi".to_string(),
            },
            volumes: vec![],
            environment_variables: HashMap::new(),
            category: "media".to_string(),
            is_system: false,
            is_hidden: false,
            is_browseable: true,
        },
    );

    apps.insert(
        "qbittorrent".to_string(),
        AppConfig {
            name: "qbittorrent".to_string(),
            display_name: "qBittorrent".to_string(),
            description: "BitTorrent client".to_string(),
            icon: "⬇️".to_string(),
            container_image: "linuxserver/qbittorrent:latest".to_string(),
            default_port: 8080,
            resource_requirements: ResourceRequirements {
                cpu_request: "100m".to_string(),
                cpu_limit: "1000m".to_string(),
                memory_request: "256Mi".to_string(),
                memory_limit: "1Gi".to_string(),
            },
            volumes: vec![],
            environment_variables: HashMap::new(),
            category: "download".to_string(),
            is_system: false,
            is_hidden: false,
            is_browseable: true,
        },
    );

    let catalog = AppCatalog::with_apps(apps);

    let media_apps = catalog.get_apps_by_category("media");
    assert_eq!(media_apps.len(), 1);
    assert_eq!(media_apps[0].name, "sonarr");

    let download_apps = catalog.get_apps_by_category("download");
    assert_eq!(download_apps.len(), 1);
    assert_eq!(download_apps[0].name, "qbittorrent");

    let other_apps = catalog.get_apps_by_category("other");
    assert!(other_apps.is_empty());
}

fn make_app(name: &str, category: &str) -> AppConfig {
    AppConfig {
        name: name.to_string(),
        display_name: name.to_string(),
        description: format!("Test app {name}"),
        icon: "📦".to_string(),
        container_image: format!("test/{name}:latest"),
        default_port: 8080,
        resource_requirements: ResourceRequirements {
            cpu_request: "100m".to_string(),
            cpu_limit: "1000m".to_string(),
            memory_request: "256Mi".to_string(),
            memory_limit: "1Gi".to_string(),
        },
        volumes: vec![],
        environment_variables: HashMap::new(),
        category: category.to_string(),
        is_system: false,
        is_hidden: false,
        is_browseable: true,
    }
}

#[test]
fn test_app_exists() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    let catalog = AppCatalog::with_apps(apps);

    assert!(
        catalog.get_app("sonarr").is_some(),
        "app_exists must return true for known apps"
    );
    assert!(
        catalog.get_app("SONARR").is_some(),
        "app_exists must be case-insensitive"
    );
    assert!(
        !catalog.get_app("radarr").is_some(),
        "app_exists must return false for unknown apps"
    );
}

#[test]
fn test_get_all_apps_count() {
    let mut apps = HashMap::new();
    apps.insert("app1".to_string(), make_app("app1", "media"));
    apps.insert("app2".to_string(), make_app("app2", "download"));
    apps.insert("app3".to_string(), make_app("app3", "media"));
    let catalog = AppCatalog::with_apps(apps);

    assert_eq!(catalog.get_all_apps().len(), 3);
}

#[test]
fn test_get_categories_sorted() {
    let mut apps = HashMap::new();
    apps.insert("z".to_string(), make_app("z", "zzz"));
    apps.insert("a".to_string(), make_app("a", "aaa"));
    apps.insert("m".to_string(), make_app("m", "mmm"));
    let catalog = AppCatalog::with_apps(apps);

    let cats = catalog.get_categories();
    let mut sorted = cats.clone();
    sorted.sort();
    assert_eq!(cats, sorted, "get_categories must return sorted categories");
    assert_eq!(cats.len(), 3);
}

#[test]
fn test_get_categories_deduplicates() {
    let mut apps = HashMap::new();
    apps.insert("a".to_string(), make_app("a", "media"));
    apps.insert("b".to_string(), make_app("b", "media"));
    apps.insert("c".to_string(), make_app("c", "download"));
    let catalog = AppCatalog::with_apps(apps);

    let cats = catalog.get_categories();
    assert_eq!(cats.len(), 2, "Categories must be deduplicated");
}

#[test]
fn test_catalog_new_no_charts_dir() {
    // AppCatalog::new() should not panic even if charts dir is missing
    // (it logs a warning and returns empty catalog)
    let catalog = AppCatalog::new();
    // In test environment charts dir (/app/charts) doesn't exist → empty catalog
    // We just verify it doesn't panic and returns a usable catalog
    let _ = catalog.get_all_apps();
    let _ = catalog.get_categories();
}

#[test]
fn test_app_with_volumes_and_env() {
    let app = AppConfig {
        name: "fullapp".to_string(),
        display_name: "Full App".to_string(),
        description: "App with all fields".to_string(),
        icon: "🎯".to_string(),
        container_image: "fullapp:latest".to_string(),
        default_port: 9090,
        resource_requirements: ResourceRequirements {
            cpu_request: "250m".to_string(),
            cpu_limit: "2000m".to_string(),
            memory_request: "512Mi".to_string(),
            memory_limit: "2Gi".to_string(),
        },
        volumes: vec![
            VolumeConfig {
                name: "config".to_string(),
                mount_path: "/config".to_string(),
                size: "1Gi".to_string(),
            },
            VolumeConfig {
                name: "data".to_string(),
                mount_path: "/data".to_string(),
                size: "10Gi".to_string(),
            },
        ],
        environment_variables: {
            let mut env = HashMap::new();
            env.insert("TZ".to_string(), "UTC".to_string());
            env.insert("PUID".to_string(), "1000".to_string());
            env
        },
        category: "tools".to_string(),
        is_system: true,
        is_hidden: true,
        is_browseable: false,
    };

    assert_eq!(app.volumes.len(), 2);
    assert_eq!(app.environment_variables.len(), 2);
    assert!(app.is_system);
    assert!(app.is_hidden);
    assert!(!app.is_browseable);

    let json = serde_json::to_string(&app).unwrap();
    let parsed: AppConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.name, "fullapp");
    assert_eq!(parsed.volumes.len(), 2);
    assert_eq!(parsed.environment_variables["TZ"], "UTC");
}
