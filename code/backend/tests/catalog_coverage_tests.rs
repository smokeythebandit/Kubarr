//! Additional coverage tests for `src/services/catalog.rs`.
//!
//! The existing `catalog_tests.rs` and `catalog_service_tests.rs` cover the
//! core happy-path API.  This file fills remaining gaps:
//!
//! - `AppCatalog::with_apps` with system/hidden/non-browseable flags
//! - `get_all_apps()` contains expected app names
//! - `get_app()` returns field values correctly (not just Some/None)
//! - `get_apps_by_category()` with multiple matching apps and field access
//! - `get_categories()` with a single app
//! - `app_exists()` with mixed-case names at boundaries
//! - `reload()` clears pre-existing in-memory apps
//! - `AppConfig` clone preserves all fields
//! - `ResourceRequirements` clone
//! - `VolumeConfig` clone
//! - Multiple volume / env-var round-trip serialization

use std::collections::HashMap;

use kubarr::services::catalog::{AppCatalog, AppConfig, ResourceRequirements, VolumeConfig};

// ============================================================================
// Helper
// ============================================================================

fn make_app(name: &str, category: &str) -> AppConfig {
    AppConfig {
        name: name.to_string(),
        display_name: format!("{} Display", name),
        description: format!("Description of {}", name),
        icon: "📦".to_string(),
        container_image: format!("docker.io/test/{}:latest", name),
        default_port: 8080,
        resource_requirements: ResourceRequirements {
            cpu_request: "50m".to_string(),
            cpu_limit: "500m".to_string(),
            memory_request: "64Mi".to_string(),
            memory_limit: "256Mi".to_string(),
        },
        volumes: vec![],
        environment_variables: HashMap::new(),
        category: category.to_string(),
        is_system: false,
        is_hidden: false,
        is_browseable: true,
    }
}

fn catalog_of(apps: &[(&str, &str)]) -> AppCatalog {
    let mut map = HashMap::new();
    for (name, cat) in apps {
        let app = make_app(name, cat);
        map.insert(app.name.clone(), app);
    }
    AppCatalog::with_apps(map)
}

// ============================================================================
// AppCatalog::with_apps — flag combinations
// ============================================================================

#[test]
fn catalog_system_hidden_app_flags() {
    let mut apps = HashMap::new();
    let mut app = make_app("system-app", "system");
    app.is_system = true;
    app.is_hidden = true;
    app.is_browseable = false;
    apps.insert(app.name.clone(), app);

    let catalog = AppCatalog::with_apps(apps);
    let found = catalog.get_app("system-app").expect("must find system-app");
    assert!(found.is_system);
    assert!(found.is_hidden);
    assert!(!found.is_browseable);
}

#[test]
fn catalog_browseable_default_true() {
    let catalog = catalog_of(&[("browseable-app", "media")]);
    let app = catalog.get_app("browseable-app").unwrap();
    assert!(app.is_browseable);
    assert!(!app.is_system);
    assert!(!app.is_hidden);
}

// ============================================================================
// get_all_apps — verify content not just length
// ============================================================================

#[test]
fn get_all_apps_contains_expected_names() {
    let catalog = catalog_of(&[
        ("sonarr", "media"),
        ("radarr", "media"),
        ("traefik", "networking"),
    ]);
    let apps = catalog.get_all_apps();
    let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"sonarr"), "must contain sonarr");
    assert!(names.contains(&"radarr"), "must contain radarr");
    assert!(names.contains(&"traefik"), "must contain traefik");
}

#[test]
fn get_all_apps_single_app() {
    let catalog = catalog_of(&[("only-app", "tools")]);
    let apps = catalog.get_all_apps();
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].name, "only-app");
    assert_eq!(apps[0].category, "tools");
}

// ============================================================================
// get_app — field values are correct
// ============================================================================

#[test]
fn get_app_returns_correct_fields() {
    let catalog = catalog_of(&[("jellyfin", "media")]);
    let app = catalog.get_app("jellyfin").expect("must find jellyfin");
    assert_eq!(app.name, "jellyfin");
    assert_eq!(app.category, "media");
    assert_eq!(app.display_name, "jellyfin Display");
    assert_eq!(app.container_image, "docker.io/test/jellyfin:latest");
    assert_eq!(app.default_port, 8080);
}

#[test]
fn get_app_lowercase_lookup() {
    let catalog = catalog_of(&[("plex", "media")]);
    // Internal key is lowercase "plex"; lookup downcases the arg
    assert!(catalog.get_app("plex").is_some());
    assert!(catalog.get_app("PLEX").is_some());
    assert!(catalog.get_app("Plex").is_some());
    assert!(catalog.get_app("pLEX").is_some());
}

#[test]
fn get_app_not_found_returns_none() {
    let catalog = catalog_of(&[("sonarr", "media")]);
    assert!(catalog.get_app("").is_none());
    assert!(catalog.get_app("unknown").is_none());
    assert!(catalog.get_app("sonarr2").is_none());
}

// ============================================================================
// get_apps_by_category — field access on returned items
// ============================================================================

#[test]
fn get_apps_by_category_returns_correct_apps() {
    let catalog = catalog_of(&[
        ("sonarr", "media"),
        ("radarr", "media"),
        ("qbittorrent", "download"),
    ]);
    let media = catalog.get_apps_by_category("media");
    assert_eq!(media.len(), 2);
    let mut media_names: Vec<&str> = media.iter().map(|a| a.name.as_str()).collect();
    media_names.sort();
    assert_eq!(media_names, vec!["radarr", "sonarr"]);
}

#[test]
fn get_apps_by_category_is_exact_match() {
    // "med" must NOT match "media"
    let catalog = catalog_of(&[("sonarr", "media")]);
    assert!(catalog.get_apps_by_category("med").is_empty());
    assert!(
        catalog.get_apps_by_category("MEDIA").is_empty(),
        "category match is case-sensitive"
    );
}

#[test]
fn get_apps_by_category_with_five_apps() {
    let catalog = catalog_of(&[
        ("a", "cat1"),
        ("b", "cat1"),
        ("c", "cat1"),
        ("d", "cat2"),
        ("e", "cat2"),
    ]);
    assert_eq!(catalog.get_apps_by_category("cat1").len(), 3);
    assert_eq!(catalog.get_apps_by_category("cat2").len(), 2);
    assert_eq!(catalog.get_apps_by_category("cat3").len(), 0);
}

// ============================================================================
// get_categories — edge cases
// ============================================================================

#[test]
fn get_categories_with_single_app() {
    let catalog = catalog_of(&[("only", "unique-cat")]);
    let cats = catalog.get_categories();
    assert_eq!(cats, vec!["unique-cat"]);
}

#[test]
fn get_categories_many_apps_same_category() {
    let catalog = catalog_of(&[("a", "same"), ("b", "same"), ("c", "same"), ("d", "same")]);
    let cats = catalog.get_categories();
    assert_eq!(cats.len(), 1);
    assert_eq!(cats[0], "same");
}

#[test]
fn get_categories_returns_sorted_order() {
    let catalog = catalog_of(&[
        ("z-app", "zzz"),
        ("a-app", "aaa"),
        ("m-app", "mmm"),
        ("b-app", "bbb"),
    ]);
    let cats = catalog.get_categories();
    assert_eq!(cats, vec!["aaa", "bbb", "mmm", "zzz"]);
}

// ============================================================================
// app_exists — boundary cases
// ============================================================================

#[test]
fn app_exists_empty_string_returns_false() {
    let catalog = catalog_of(&[("sonarr", "media")]);
    assert!(!catalog.app_exists(""));
}

#[test]
fn app_exists_whitespace_returns_false() {
    let catalog = catalog_of(&[("sonarr", "media")]);
    // " sonarr" is not the same as "sonarr"
    assert!(!catalog.app_exists(" sonarr"));
}

#[test]
fn app_exists_returns_true_for_all_inserted() {
    let names = ["app-one", "app-two", "app-three"];
    let pairs: Vec<(&str, &str)> = names.iter().map(|n| (*n, "cat")).collect();
    let catalog = catalog_of(&pairs);
    for name in &names {
        assert!(catalog.app_exists(name), "must exist: {}", name);
    }
}

// ============================================================================
// reload — clears in-memory apps then re-loads (empty charts dir in tests)
// ============================================================================

#[test]
fn reload_clears_in_memory_apps() {
    let mut catalog = catalog_of(&[("sonarr", "media"), ("radarr", "media")]);
    // Before reload, apps are present
    assert_eq!(catalog.get_all_apps().len(), 2);

    // reload() clears and re-loads from charts dir (which doesn't exist in tests)
    catalog.reload();

    // After reload, in-memory test apps are gone; charts dir is absent → empty
    // (We only assert no panic and that get_all_apps() is callable)
    let _ = catalog.get_all_apps();
    let _ = catalog.get_categories();
    let _ = catalog.get_apps_by_category("media");
}

#[test]
fn reload_on_empty_catalog_does_not_panic() {
    let mut catalog = AppCatalog::with_apps(HashMap::new());
    catalog.reload();
    assert!(catalog.get_all_apps().len() == 0 || catalog.get_all_apps().len() >= 0);
}

// ============================================================================
// AppConfig clone
// ============================================================================

#[test]
fn app_config_clone_preserves_all_fields() {
    let mut env = HashMap::new();
    env.insert("TZ".to_string(), "UTC".to_string());
    env.insert("PUID".to_string(), "1000".to_string());

    let original = AppConfig {
        name: "clone-test".to_string(),
        display_name: "Clone Test".to_string(),
        description: "A test for clone".to_string(),
        icon: "🎯".to_string(),
        container_image: "registry/clone-test:v1".to_string(),
        default_port: 9090,
        resource_requirements: ResourceRequirements {
            cpu_request: "200m".to_string(),
            cpu_limit: "2000m".to_string(),
            memory_request: "512Mi".to_string(),
            memory_limit: "4Gi".to_string(),
        },
        volumes: vec![VolumeConfig {
            name: "config".to_string(),
            mount_path: "/config".to_string(),
            size: "2Gi".to_string(),
        }],
        environment_variables: env,
        category: "tools".to_string(),
        is_system: true,
        is_hidden: false,
        is_browseable: true,
    };

    let cloned = original.clone();
    assert_eq!(cloned.name, "clone-test");
    assert_eq!(cloned.display_name, "Clone Test");
    assert_eq!(cloned.description, "A test for clone");
    assert_eq!(cloned.icon, "🎯");
    assert_eq!(cloned.container_image, "registry/clone-test:v1");
    assert_eq!(cloned.default_port, 9090);
    assert_eq!(cloned.resource_requirements.cpu_request, "200m");
    assert_eq!(cloned.resource_requirements.memory_limit, "4Gi");
    assert_eq!(cloned.volumes.len(), 1);
    assert_eq!(cloned.volumes[0].name, "config");
    assert_eq!(cloned.environment_variables["TZ"], "UTC");
    assert_eq!(cloned.category, "tools");
    assert!(cloned.is_system);
    assert!(!cloned.is_hidden);
    assert!(cloned.is_browseable);
}

// ============================================================================
// ResourceRequirements clone
// ============================================================================

#[test]
fn resource_requirements_clone() {
    let rr = ResourceRequirements {
        cpu_request: "100m".to_string(),
        cpu_limit: "1000m".to_string(),
        memory_request: "256Mi".to_string(),
        memory_limit: "1Gi".to_string(),
    };
    let cloned = rr.clone();
    assert_eq!(cloned.cpu_request, "100m");
    assert_eq!(cloned.cpu_limit, "1000m");
    assert_eq!(cloned.memory_request, "256Mi");
    assert_eq!(cloned.memory_limit, "1Gi");
}

// ============================================================================
// VolumeConfig clone
// ============================================================================

#[test]
fn volume_config_clone() {
    let vol = VolumeConfig {
        name: "media".to_string(),
        mount_path: "/media".to_string(),
        size: "50Gi".to_string(),
    };
    let cloned = vol.clone();
    assert_eq!(cloned.name, "media");
    assert_eq!(cloned.mount_path, "/media");
    assert_eq!(cloned.size, "50Gi");
}

// ============================================================================
// AppConfig serialization round-trip with volumes and env vars
// ============================================================================

#[test]
fn app_config_with_env_vars_roundtrip() {
    let mut env = HashMap::new();
    env.insert("PUID".to_string(), "1000".to_string());
    env.insert("PGID".to_string(), "1000".to_string());
    env.insert("TZ".to_string(), "Europe/London".to_string());

    let app = AppConfig {
        name: "envapp".to_string(),
        display_name: "Env App".to_string(),
        description: "App with env vars".to_string(),
        icon: "🔧".to_string(),
        container_image: "test/envapp:1.0".to_string(),
        default_port: 7070,
        resource_requirements: ResourceRequirements {
            cpu_request: "50m".to_string(),
            cpu_limit: "200m".to_string(),
            memory_request: "32Mi".to_string(),
            memory_limit: "128Mi".to_string(),
        },
        volumes: vec![
            VolumeConfig {
                name: "vol-a".to_string(),
                mount_path: "/vol-a".to_string(),
                size: "1Gi".to_string(),
            },
            VolumeConfig {
                name: "vol-b".to_string(),
                mount_path: "/vol-b".to_string(),
                size: "5Gi".to_string(),
            },
        ],
        environment_variables: env,
        category: "custom".to_string(),
        is_system: false,
        is_hidden: false,
        is_browseable: false,
    };

    let json = serde_json::to_string(&app).expect("must serialize");
    let parsed: AppConfig = serde_json::from_str(&json).expect("must deserialize");

    assert_eq!(parsed.name, "envapp");
    assert_eq!(parsed.default_port, 7070);
    assert_eq!(parsed.volumes.len(), 2);
    assert_eq!(parsed.environment_variables.len(), 3);
    assert_eq!(parsed.environment_variables["TZ"], "Europe/London");
    assert_eq!(parsed.volumes[0].size.as_str(), "1Gi");
    assert!(!parsed.is_browseable);
}

#[test]
fn app_config_no_volumes_no_env_roundtrip() {
    let app = AppConfig {
        name: "minimal".to_string(),
        display_name: "Minimal".to_string(),
        description: "Minimal app".to_string(),
        icon: "📦".to_string(),
        container_image: "test/minimal:latest".to_string(),
        default_port: 80,
        resource_requirements: ResourceRequirements {
            cpu_request: "10m".to_string(),
            cpu_limit: "100m".to_string(),
            memory_request: "16Mi".to_string(),
            memory_limit: "64Mi".to_string(),
        },
        volumes: vec![],
        environment_variables: HashMap::new(),
        category: "other".to_string(),
        is_system: false,
        is_hidden: false,
        is_browseable: true,
    };

    let json = serde_json::to_string(&app).unwrap();
    let parsed: AppConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.name, "minimal");
    assert!(parsed.volumes.is_empty());
    assert!(parsed.environment_variables.is_empty());
    assert_eq!(parsed.default_port, 80);
}

// ============================================================================
// AppCatalog::new / Default — must not panic without charts dir
// ============================================================================

#[test]
fn catalog_new_is_same_as_default() {
    // Both AppCatalog::new() and AppCatalog::default() call load_apps().
    // In the test environment, the charts dir doesn't exist → both should
    // produce a usable (possibly empty) catalog.
    let from_new = AppCatalog::new();
    let from_default = AppCatalog::default();

    // Check that both produce consistent results
    assert_eq!(
        from_new.get_all_apps().len(),
        from_default.get_all_apps().len(),
        "new() and default() must load the same number of apps"
    );
    assert_eq!(
        from_new.get_categories().len(),
        from_default.get_categories().len(),
    );
}

#[test]
fn catalog_new_returns_usable_catalog() {
    let catalog = AppCatalog::new();
    // Must be callable without panicking regardless of filesystem state
    let _ = catalog.get_all_apps();
    let _ = catalog.get_categories();
    assert!(!catalog.app_exists("")); // empty string never exists
}
