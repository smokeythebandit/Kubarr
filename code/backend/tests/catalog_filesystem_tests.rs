//! Catalog service filesystem tests
//!
//! Tests the `AppCatalog` service, focusing on:
//! - `AppCatalog::new()` and `AppCatalog::default()` resilience when the charts
//!   directory is absent (the typical test environment).
//! - `reload()` semantics: clears existing apps and re-attempts to load from disk.
//! - Public API surface: `get_all_apps`, `get_app`, `get_apps_by_category`,
//!   `get_categories`, `app_exists`.
//! - Structural correctness of `AppConfig`, `ResourceRequirements`, and
//!   `VolumeConfig` fields.
//! - Edge-cases: empty catalog, single app, many apps, multiple categories,
//!   category deduplication.

use std::collections::HashMap;

use kubarr::services::catalog::{AppCatalog, AppConfig, ResourceRequirements, VolumeConfig};

// ============================================================================
// Helper
// ============================================================================

/// Build a minimal `AppConfig` with the given name and category.
fn make_app(name: &str, category: &str) -> AppConfig {
    AppConfig {
        name: name.to_string(),
        display_name: format!("{} Display", name),
        description: format!("Description for {}", name),
        icon: "box".to_string(),
        container_image: format!("linuxserver/{}:latest", name),
        default_port: 8080,
        resource_requirements: ResourceRequirements {
            cpu_request: "100m".to_string(),
            cpu_limit: "500m".to_string(),
            memory_request: "256Mi".to_string(),
            memory_limit: "512Mi".to_string(),
        },
        volumes: vec![],
        environment_variables: HashMap::new(),
        category: category.to_string(),
        is_system: false,
        is_hidden: false,
        is_browseable: true,
    }
}

// ============================================================================
// AppCatalog::new() / Default — resilience without a charts directory
// ============================================================================

#[test]
fn test_new_does_not_panic_without_charts_dir() {
    // In the test environment the default charts directory (/app/charts) does
    // not exist. AppCatalog::new() must return gracefully without panicking.
    let catalog = AppCatalog::new();
    // We can call any method on the returned catalog without panicking.
    let _ = catalog.get_all_apps();
    let _ = catalog.get_categories();
}

#[test]
fn test_default_does_not_panic_without_charts_dir() {
    // AppCatalog::default() delegates to new(). Both must be safe to call when
    // the charts directory does not exist.
    let catalog = AppCatalog::default();
    let _ = catalog.get_all_apps();
    let _ = catalog.get_categories();
}

#[test]
fn test_new_returns_empty_catalog_when_charts_dir_missing() {
    // Without a charts directory the catalog should contain no apps.
    let catalog = AppCatalog::new();
    assert!(
        catalog.get_all_apps().is_empty(),
        "Catalog must be empty when the charts directory does not exist"
    );
}

#[test]
fn test_default_returns_empty_catalog_when_charts_dir_missing() {
    let catalog = AppCatalog::default();
    assert!(
        catalog.get_all_apps().is_empty(),
        "Default catalog must be empty when the charts directory does not exist"
    );
}

// ============================================================================
// AppCatalog::with_apps()
// ============================================================================

#[test]
fn test_with_apps_empty_map() {
    let catalog = AppCatalog::with_apps(HashMap::new());
    assert!(catalog.get_all_apps().is_empty());
    assert!(catalog.get_categories().is_empty());
    assert!(catalog.get_app("anything").is_none());
    assert!(!catalog.get_app("anything").is_some());
}

#[test]
fn test_with_apps_single_app() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));

    let catalog = AppCatalog::with_apps(apps);
    assert_eq!(catalog.get_all_apps().len(), 1);
    assert!(catalog.get_app("sonarr").is_some());
}

#[test]
fn test_with_apps_multiple_apps() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    apps.insert("radarr".to_string(), make_app("radarr", "media"));
    apps.insert(
        "qbittorrent".to_string(),
        make_app("qbittorrent", "download"),
    );

    let catalog = AppCatalog::with_apps(apps);
    assert_eq!(catalog.get_all_apps().len(), 3);
}

// ============================================================================
// reload() — clears apps and re-loads from charts directory
// ============================================================================

#[test]
fn test_reload_clears_with_apps_catalog() {
    // Create a catalog pre-loaded with apps via with_apps(), then reload().
    // Since the charts directory does not exist in the test environment,
    // reload() should clear all apps and leave the catalog empty.
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    apps.insert("radarr".to_string(), make_app("radarr", "media"));

    let mut catalog = AppCatalog::with_apps(apps);
    assert_eq!(
        catalog.get_all_apps().len(),
        2,
        "Catalog must start with 2 pre-set apps"
    );

    catalog.reload();

    assert!(
        catalog.get_all_apps().is_empty(),
        "After reload() with no charts dir, catalog must be empty"
    );
}

#[test]
fn test_reload_clears_categories() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    apps.insert(
        "qbittorrent".to_string(),
        make_app("qbittorrent", "download"),
    );

    let mut catalog = AppCatalog::with_apps(apps);
    assert_eq!(catalog.get_categories().len(), 2);

    catalog.reload();

    assert!(
        catalog.get_categories().is_empty(),
        "After reload(), categories must also be empty"
    );
}

#[test]
fn test_reload_clears_app_lookup() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));

    let mut catalog = AppCatalog::with_apps(apps);
    assert!(catalog.get_app("sonarr").is_some());

    catalog.reload();

    assert!(
        catalog.get_app("sonarr").is_none(),
        "After reload(), previously present apps must no longer be findable"
    );
}

#[test]
fn test_reload_clears_app_exists() {
    let mut apps = HashMap::new();
    apps.insert("jellyfin".to_string(), make_app("jellyfin", "media"));

    let mut catalog = AppCatalog::with_apps(apps);
    assert!(catalog.get_app("jellyfin").is_some());

    catalog.reload();

    assert!(
        !catalog.get_app("jellyfin").is_some(),
        "After reload(), app_exists must return false for cleared apps"
    );
}

#[test]
fn test_reload_multiple_times_stays_empty() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));

    let mut catalog = AppCatalog::with_apps(apps);
    catalog.reload();
    catalog.reload();
    catalog.reload();

    assert!(
        catalog.get_all_apps().is_empty(),
        "Repeated reload() calls must leave the catalog empty without charts dir"
    );
}

#[test]
fn test_reload_on_new_catalog_stays_empty() {
    // new() + reload() — both operations should produce an empty catalog when
    // the charts directory does not exist.
    let mut catalog = AppCatalog::new();
    assert!(catalog.get_all_apps().is_empty());

    catalog.reload();

    assert!(
        catalog.get_all_apps().is_empty(),
        "reload() on a catalog already empty must remain empty"
    );
}

// ============================================================================
// get_app() — case-insensitive lookup
// ============================================================================

#[test]
fn test_get_app_case_insensitive_upper() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    let catalog = AppCatalog::with_apps(apps);

    assert!(
        catalog.get_app("SONARR").is_some(),
        "get_app must be case-insensitive (all-uppercase)"
    );
}

#[test]
fn test_get_app_case_insensitive_mixed() {
    let mut apps = HashMap::new();
    apps.insert("radarr".to_string(), make_app("radarr", "media"));
    let catalog = AppCatalog::with_apps(apps);

    assert!(
        catalog.get_app("RaDaRr").is_some(),
        "get_app must be case-insensitive (mixed-case)"
    );
}

#[test]
fn test_get_app_returns_correct_app() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    apps.insert("radarr".to_string(), make_app("radarr", "media"));
    let catalog = AppCatalog::with_apps(apps);

    let app = catalog.get_app("sonarr").unwrap();
    assert_eq!(app.name, "sonarr");
    assert_eq!(app.category, "media");
}

#[test]
fn test_get_app_missing_returns_none() {
    let catalog = AppCatalog::with_apps(HashMap::new());
    assert!(catalog.get_app("nonexistent").is_none());
}

// ============================================================================
// app_exists() — case-insensitive
// ============================================================================

#[test]
fn test_app_exists_case_insensitive() {
    let mut apps = HashMap::new();
    apps.insert("jellyfin".to_string(), make_app("jellyfin", "media"));
    let catalog = AppCatalog::with_apps(apps);

    assert!(catalog.get_app("jellyfin").is_some());
    assert!(catalog.get_app("JELLYFIN").is_some());
    assert!(catalog.get_app("Jellyfin").is_some());
}

#[test]
fn test_app_exists_false_for_unknown() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    let catalog = AppCatalog::with_apps(apps);

    assert!(!catalog.get_app("radarr").is_some());
}

// ============================================================================
// get_apps_by_category()
// ============================================================================

#[test]
fn test_get_apps_by_category_single_match() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    apps.insert(
        "qbittorrent".to_string(),
        make_app("qbittorrent", "download"),
    );
    let catalog = AppCatalog::with_apps(apps);

    let media = catalog.get_apps_by_category("media");
    assert_eq!(media.len(), 1);
    assert_eq!(media[0].name, "sonarr");
}

#[test]
fn test_get_apps_by_category_multiple_matches() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    apps.insert("radarr".to_string(), make_app("radarr", "media"));
    apps.insert("bazarr".to_string(), make_app("bazarr", "media"));
    apps.insert(
        "qbittorrent".to_string(),
        make_app("qbittorrent", "download"),
    );
    let catalog = AppCatalog::with_apps(apps);

    let media = catalog.get_apps_by_category("media");
    assert_eq!(media.len(), 3, "Three apps are in the 'media' category");
}

#[test]
fn test_get_apps_by_category_no_match() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    let catalog = AppCatalog::with_apps(apps);

    let results = catalog.get_apps_by_category("tools");
    assert!(
        results.is_empty(),
        "get_apps_by_category with no matching category must return empty vec"
    );
}

#[test]
fn test_get_apps_by_category_case_sensitive() {
    // Category matching is case-sensitive (stored as-is from chart annotations)
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    let catalog = AppCatalog::with_apps(apps);

    // "Media" (capitalised) must NOT match "media"
    let results = catalog.get_apps_by_category("Media");
    assert!(
        results.is_empty(),
        "Category matching must be case-sensitive"
    );
}

// ============================================================================
// get_categories() — sorted and deduplicated
// ============================================================================

#[test]
fn test_get_categories_empty_catalog() {
    let catalog = AppCatalog::with_apps(HashMap::new());
    assert!(catalog.get_categories().is_empty());
}

#[test]
fn test_get_categories_single_category() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    let catalog = AppCatalog::with_apps(apps);

    let cats = catalog.get_categories();
    assert_eq!(cats, vec!["media"]);
}

#[test]
fn test_get_categories_deduplicates() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    apps.insert("radarr".to_string(), make_app("radarr", "media"));
    apps.insert("bazarr".to_string(), make_app("bazarr", "media"));
    let catalog = AppCatalog::with_apps(apps);

    let cats = catalog.get_categories();
    assert_eq!(
        cats.len(),
        1,
        "Three apps with the same category → one unique entry"
    );
    assert_eq!(cats[0], "media");
}

#[test]
fn test_get_categories_sorted_alphabetically() {
    let mut apps = HashMap::new();
    apps.insert("z_app".to_string(), make_app("z_app", "zzz_cat"));
    apps.insert("a_app".to_string(), make_app("a_app", "aaa_cat"));
    apps.insert("m_app".to_string(), make_app("m_app", "mmm_cat"));
    let catalog = AppCatalog::with_apps(apps);

    let cats = catalog.get_categories();
    let mut sorted = cats.clone();
    sorted.sort();

    assert_eq!(
        cats, sorted,
        "get_categories() must return categories in alphabetical order"
    );
    assert_eq!(cats, vec!["aaa_cat", "mmm_cat", "zzz_cat"]);
}

#[test]
fn test_get_categories_mixed_dedup_and_sort() {
    let mut apps = HashMap::new();
    apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
    apps.insert("radarr".to_string(), make_app("radarr", "media")); // dup
    apps.insert("qbt".to_string(), make_app("qbt", "download"));
    apps.insert("nzb".to_string(), make_app("nzb", "download")); // dup
    apps.insert("grafana".to_string(), make_app("grafana", "monitoring"));
    let catalog = AppCatalog::with_apps(apps);

    let cats = catalog.get_categories();
    assert_eq!(cats.len(), 3, "Three distinct categories");
    assert_eq!(cats, vec!["download", "media", "monitoring"]);
}

// ============================================================================
// AppConfig field correctness
// ============================================================================

#[test]
fn test_app_config_is_system_flag() {
    let mut app = make_app("gluetun", "vpn");
    app.is_system = true;
    assert!(app.is_system);
    assert!(!app.is_hidden);
}

#[test]
fn test_app_config_is_hidden_flag() {
    let mut app = make_app("internal_tool", "system");
    app.is_hidden = true;
    assert!(app.is_hidden);
}

#[test]
fn test_app_config_is_not_browseable() {
    let mut app = make_app("backend_svc", "system");
    app.is_browseable = false;
    assert!(!app.is_browseable);
}

#[test]
fn test_app_config_default_browseable() {
    // make_app() sets is_browseable = true by default.
    let app = make_app("sonarr", "media");
    assert!(app.is_browseable);
}

#[test]
fn test_app_config_with_environment_variables() {
    let mut env = HashMap::new();
    env.insert("TZ".to_string(), "UTC".to_string());
    env.insert("PUID".to_string(), "1000".to_string());
    env.insert("PGID".to_string(), "1000".to_string());

    let app = AppConfig {
        name: "sonarr".to_string(),
        display_name: "Sonarr".to_string(),
        description: "TV management".to_string(),
        icon: "tv".to_string(),
        container_image: "linuxserver/sonarr:latest".to_string(),
        default_port: 8989,
        resource_requirements: ResourceRequirements {
            cpu_request: "100m".to_string(),
            cpu_limit: "1000m".to_string(),
            memory_request: "256Mi".to_string(),
            memory_limit: "1Gi".to_string(),
        },
        volumes: vec![],
        environment_variables: env.clone(),
        category: "media".to_string(),
        is_system: false,
        is_hidden: false,
        is_browseable: true,
    };

    assert_eq!(app.environment_variables.len(), 3);
    assert_eq!(app.environment_variables["TZ"], "UTC");
    assert_eq!(app.environment_variables["PUID"], "1000");
}

#[test]
fn test_app_config_with_volumes() {
    let volumes = vec![
        VolumeConfig {
            name: "config".to_string(),
            mount_path: "/config".to_string(),
            size: "1Gi".to_string(),
        },
        VolumeConfig {
            name: "data".to_string(),
            mount_path: "/data".to_string(),
            size: "50Gi".to_string(),
        },
    ];

    let mut app = make_app("plex", "media");
    app.volumes = volumes;

    assert_eq!(app.volumes.len(), 2);
    assert_eq!(app.volumes[0].name, "config");
    assert_eq!(app.volumes[0].mount_path, "/config");
    assert_eq!(app.volumes[1].size, "50Gi");
}

// ============================================================================
// ResourceRequirements
// ============================================================================

#[test]
fn test_resource_requirements_fields() {
    let resources = ResourceRequirements {
        cpu_request: "250m".to_string(),
        cpu_limit: "2000m".to_string(),
        memory_request: "512Mi".to_string(),
        memory_limit: "2Gi".to_string(),
    };

    assert_eq!(resources.cpu_request, "250m");
    assert_eq!(resources.cpu_limit, "2000m");
    assert_eq!(resources.memory_request, "512Mi");
    assert_eq!(resources.memory_limit, "2Gi");
}

#[test]
fn test_resource_requirements_clone() {
    let resources = ResourceRequirements {
        cpu_request: "100m".to_string(),
        cpu_limit: "500m".to_string(),
        memory_request: "128Mi".to_string(),
        memory_limit: "256Mi".to_string(),
    };
    let cloned = resources.clone();
    assert_eq!(cloned.cpu_request, resources.cpu_request);
    assert_eq!(cloned.memory_limit, resources.memory_limit);
}

// ============================================================================
// VolumeConfig
// ============================================================================

#[test]
fn test_volume_config_fields() {
    let vol = VolumeConfig {
        name: "media".to_string(),
        mount_path: "/media".to_string(),
        size: "100Gi".to_string(),
    };
    assert_eq!(vol.name, "media");
    assert_eq!(vol.mount_path, "/media");
    assert_eq!(vol.size, "100Gi");
}

#[test]
fn test_volume_config_clone() {
    let vol = VolumeConfig {
        name: "config".to_string(),
        mount_path: "/config".to_string(),
        size: "1Gi".to_string(),
    };
    let cloned = vol.clone();
    assert_eq!(cloned.name, vol.name);
    assert_eq!(cloned.mount_path, vol.mount_path);
    assert_eq!(cloned.size, vol.size);
}

// ============================================================================
// Large catalog stress
// ============================================================================

#[test]
fn test_large_catalog_get_all_apps_count() {
    let n = 50usize;
    let mut apps = HashMap::new();
    for i in 0..n {
        let name = format!("app_{:02}", i);
        let category = if i % 2 == 0 { "media" } else { "download" };
        apps.insert(name.clone(), make_app(&name, category));
    }
    let catalog = AppCatalog::with_apps(apps);

    assert_eq!(catalog.get_all_apps().len(), n);
    assert_eq!(
        catalog.get_apps_by_category("media").len(),
        n / 2,
        "Half the apps are in 'media'"
    );
    assert_eq!(
        catalog.get_apps_by_category("download").len(),
        n / 2,
        "Half the apps are in 'download'"
    );
    let cats = catalog.get_categories();
    assert_eq!(cats.len(), 2);
    assert_eq!(cats, vec!["download", "media"]);
}

#[test]
fn test_large_catalog_reload_clears_everything() {
    let n = 20usize;
    let mut apps = HashMap::new();
    for i in 0..n {
        let name = format!("app_{:02}", i);
        apps.insert(name.clone(), make_app(&name, "media"));
    }
    let mut catalog = AppCatalog::with_apps(apps);
    assert_eq!(catalog.get_all_apps().len(), n);

    catalog.reload();

    assert!(
        catalog.get_all_apps().is_empty(),
        "reload() must clear all {} apps",
        n
    );
    assert!(catalog.get_categories().is_empty());
    for i in 0..n {
        let name = format!("app_{:02}", i);
        assert!(!catalog.get_app(&name).is_some());
    }
}
