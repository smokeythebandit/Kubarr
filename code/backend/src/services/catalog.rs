use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::CONFIG;
use crate::error::Result;

/// App configuration from Helm chart
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AppConfig {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub icon: String,
    pub container_image: String,
    pub default_port: i32,
    pub resource_requirements: ResourceRequirements,
    pub volumes: Vec<VolumeConfig>,
    pub environment_variables: HashMap<String, String>,
    pub category: String,
    pub is_system: bool,
    pub is_hidden: bool,
    pub is_browseable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ResourceRequirements {
    pub cpu_request: String,
    pub cpu_limit: String,
    pub memory_request: String,
    pub memory_limit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct VolumeConfig {
    pub name: String,
    pub mount_path: String,
    pub size: String,
}

/// App catalog - registry of all available applications
pub struct AppCatalog {
    apps: HashMap<String, AppConfig>,
}

impl AppCatalog {
    /// Create a new app catalog from charts directory
    pub fn new() -> Self {
        let mut catalog = Self {
            apps: HashMap::new(),
        };
        catalog.load_apps();
        catalog
    }

    /// Create an app catalog with the given apps (for testing)
    pub fn with_apps(apps: HashMap<String, AppConfig>) -> Self {
        Self { apps }
    }

    /// Load all app definitions from Helm charts
    fn load_apps(&mut self) {
        let charts_dir = &CONFIG.charts.dir;

        if !charts_dir.exists() {
            tracing::warn!("Charts directory does not exist: {}", charts_dir.display());
            return;
        }

        if let Ok(entries) = std::fs::read_dir(charts_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(chart_name) = path.file_name().and_then(|n| n.to_str()) {
                        match self.parse_chart(chart_name, &path) {
                            Ok(Some(app)) => {
                                self.apps.insert(app.name.clone(), app);
                            }
                            Ok(None) => {
                                // Chart doesn't have kubarr annotations, skip
                            }
                            Err(e) => {
                                tracing::warn!("Failed to load chart {}: {}", chart_name, e);
                            }
                        }
                    }
                }
            }
        }

        tracing::info!("Loaded {} apps from catalog", self.apps.len());
    }

    /// Parse a Helm chart into an AppConfig
    fn parse_chart(&self, chart_name: &str, chart_dir: &Path) -> Result<Option<AppConfig>> {
        let chart_yaml = chart_dir.join("Chart.yaml");
        let values_yaml = chart_dir.join("values.yaml");

        if !chart_yaml.exists() {
            return Ok(None);
        }

        // Parse Chart.yaml
        let chart_content = std::fs::read_to_string(&chart_yaml)?;
        let chart: serde_yaml::Value = serde_yaml::from_str(&chart_content)?;

        // Get kubarr annotations
        let annotations = chart
            .get("annotations")
            .and_then(|a| a.as_mapping())
            .cloned()
            .unwrap_or_default();

        // Skip charts without kubarr category annotation
        let category =
            match annotations.get(serde_yaml::Value::String("kubarr.io/category".to_string())) {
                Some(c) => c.as_str().unwrap_or("other").to_string(),
                None => return Ok(None),
            };

        // Parse values.yaml
        let values: serde_yaml::Value = if values_yaml.exists() {
            let values_content = std::fs::read_to_string(&values_yaml)?;
            serde_yaml::from_str(&values_content).unwrap_or(serde_yaml::Value::Null)
        } else {
            serde_yaml::Value::Null
        };

        // Get app-specific config
        let app_values = values
            .get(chart_name)
            .cloned()
            .unwrap_or(serde_yaml::Value::Null);

        // Extract image info
        let image_config = app_values
            .get("image")
            .cloned()
            .unwrap_or(serde_yaml::Value::Null);
        let default_image_repo = format!("linuxserver/{}", chart_name);
        let image_repo = image_config
            .get("repository")
            .and_then(|r| r.as_str())
            .unwrap_or(&default_image_repo);
        let image_tag = image_config
            .get("tag")
            .and_then(|t| t.as_str())
            .unwrap_or("latest");
        let container_image = format!("{}:{}", image_repo, image_tag);

        // Extract port
        let service_config = app_values
            .get("service")
            .cloned()
            .unwrap_or(serde_yaml::Value::Null);
        let default_port = service_config
            .get("port")
            .and_then(|p| p.as_i64())
            .unwrap_or(8080) as i32;

        // Extract resources
        let resources_config = app_values
            .get("resources")
            .cloned()
            .unwrap_or(serde_yaml::Value::Null);
        let requests = resources_config
            .get("requests")
            .cloned()
            .unwrap_or(serde_yaml::Value::Null);
        let limits = resources_config
            .get("limits")
            .cloned()
            .unwrap_or(serde_yaml::Value::Null);

        let resource_requirements = ResourceRequirements {
            cpu_request: requests
                .get("cpu")
                .and_then(|c| c.as_str())
                .unwrap_or("100m")
                .to_string(),
            cpu_limit: limits
                .get("cpu")
                .and_then(|c| c.as_str())
                .unwrap_or("1000m")
                .to_string(),
            memory_request: requests
                .get("memory")
                .and_then(|m| m.as_str())
                .unwrap_or("256Mi")
                .to_string(),
            memory_limit: limits
                .get("memory")
                .and_then(|m| m.as_str())
                .unwrap_or("1Gi")
                .to_string(),
        };

        // Extract volumes
        let mut volumes = Vec::new();
        if let Some(persistence) = values.get("persistence").and_then(|p| p.as_mapping()) {
            for (vol_name, vol_config) in persistence {
                if let (Some(name), Some(config)) = (vol_name.as_str(), vol_config.as_mapping()) {
                    let enabled = config
                        .get(serde_yaml::Value::String("enabled".to_string()))
                        .and_then(|e| e.as_bool())
                        .unwrap_or(true);

                    if enabled {
                        volumes.push(VolumeConfig {
                            name: name.to_string(),
                            mount_path: config
                                .get(serde_yaml::Value::String("mountPath".to_string()))
                                .and_then(|m| m.as_str())
                                .unwrap_or(&format!("/{}", name))
                                .to_string(),
                            size: config
                                .get(serde_yaml::Value::String("size".to_string()))
                                .and_then(|s| s.as_str())
                                .unwrap_or("1Gi")
                                .to_string(),
                        });
                    }
                }
            }
        }

        // Extract environment variables
        let env_vars: HashMap<String, String> = app_values
            .get("env")
            .and_then(|e| e.as_mapping())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        // Get display name and other annotations
        let display_name = annotations
            .get(serde_yaml::Value::String(
                "kubarr.io/display-name".to_string(),
            ))
            .and_then(|d| d.as_str())
            .unwrap_or(chart_name)
            .to_string();

        let icon = annotations
            .get(serde_yaml::Value::String("kubarr.io/icon".to_string()))
            .and_then(|i| i.as_str())
            .unwrap_or("📦")
            .to_string();

        let is_system = annotations
            .get(serde_yaml::Value::String("kubarr.io/system".to_string()))
            .and_then(|s| s.as_str())
            .map(|s| s.to_lowercase() == "true")
            .unwrap_or(false);

        let is_hidden = annotations
            .get(serde_yaml::Value::String("kubarr.io/hidden".to_string()))
            .and_then(|h| h.as_str())
            .map(|h| h.to_lowercase() == "true")
            .unwrap_or(false);

        // Apps are browseable by default unless explicitly set to false
        let is_browseable = annotations
            .get(serde_yaml::Value::String(
                "kubarr.io/browseable".to_string(),
            ))
            .and_then(|b| b.as_str())
            .map(|b| b.to_lowercase() != "false")
            .unwrap_or(true);

        let description = chart
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_string();

        Ok(Some(AppConfig {
            name: chart_name.to_string(),
            display_name,
            description,
            icon,
            container_image,
            default_port,
            resource_requirements,
            volumes,
            environment_variables: env_vars,
            category,
            is_system,
            is_hidden,
            is_browseable,
        }))
    }

    /// Get all available apps
    pub fn get_all_apps(&self) -> Vec<&AppConfig> {
        self.apps.values().collect()
    }

    /// Get a specific app by name
    pub fn get_app(&self, app_name: &str) -> Option<&AppConfig> {
        self.apps.get(&app_name.to_lowercase())
    }

    /// Get all apps in a specific category
    pub fn get_apps_by_category(&self, category: &str) -> Vec<&AppConfig> {
        self.apps
            .values()
            .filter(|app| app.category == category)
            .collect()
    }

    /// Check if an app exists in the catalog
    #[allow(dead_code)]
    pub fn app_exists(&self, app_name: &str) -> bool {
        self.apps.contains_key(&app_name.to_lowercase())
    }

    /// Get all unique categories
    pub fn get_categories(&self) -> Vec<String> {
        let mut categories: Vec<_> = self
            .apps
            .values()
            .map(|app| app.category.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        categories.sort();
        categories
    }

    /// Reload apps from charts directory
    pub fn reload(&mut self) {
        self.apps.clear();
        self.load_apps();
    }
}

impl Default for AppCatalog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_app(name: &str, category: &str) -> AppConfig {
        AppConfig {
            name: name.to_string(),
            display_name: format!("{} Display", name),
            description: format!("{} description", name),
            icon: "📦".to_string(),
            container_image: format!("linuxserver/{}:latest", name),
            default_port: 8080,
            resource_requirements: ResourceRequirements {
                cpu_request: "100m".to_string(),
                cpu_limit: "500m".to_string(),
                memory_request: "128Mi".to_string(),
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

    fn make_catalog() -> AppCatalog {
        let mut apps = HashMap::new();
        apps.insert("sonarr".to_string(), make_app("sonarr", "media"));
        apps.insert("radarr".to_string(), make_app("radarr", "media"));
        apps.insert("grafana".to_string(), make_app("grafana", "monitoring"));
        AppCatalog::with_apps(apps)
    }

    #[test]
    fn get_all_apps_returns_all() {
        let catalog = make_catalog();
        assert_eq!(catalog.get_all_apps().len(), 3);
    }

    #[test]
    fn get_app_found() {
        let catalog = make_catalog();
        let app = catalog.get_app("sonarr");
        assert!(app.is_some());
        assert_eq!(app.unwrap().name, "sonarr");
    }

    #[test]
    fn get_app_not_found() {
        let catalog = make_catalog();
        assert!(catalog.get_app("nonexistent").is_none());
    }

    #[test]
    fn get_apps_by_category_media() {
        let catalog = make_catalog();
        let media = catalog.get_apps_by_category("media");
        assert_eq!(media.len(), 2);
    }

    #[test]
    fn get_apps_by_category_monitoring() {
        let catalog = make_catalog();
        let monitoring = catalog.get_apps_by_category("monitoring");
        assert_eq!(monitoring.len(), 1);
        assert_eq!(monitoring[0].name, "grafana");
    }

    #[test]
    fn get_apps_by_category_missing() {
        let catalog = make_catalog();
        assert!(catalog.get_apps_by_category("nonexistent").is_empty());
    }

    #[test]
    fn app_exists_true() {
        let catalog = make_catalog();
        assert!(catalog.app_exists("sonarr"));
    }

    #[test]
    fn app_exists_false() {
        let catalog = make_catalog();
        assert!(!catalog.app_exists("plex"));
    }

    #[test]
    fn get_categories_sorted() {
        let catalog = make_catalog();
        let cats = catalog.get_categories();
        // Should be sorted and contain "media" and "monitoring"
        assert!(cats.contains(&"media".to_string()));
        assert!(cats.contains(&"monitoring".to_string()));
        assert_eq!(cats.len(), 2);
        // Verify sorted
        let mut sorted = cats.clone();
        sorted.sort();
        assert_eq!(cats, sorted);
    }

    #[test]
    fn app_config_serde_roundtrip() {
        let app = make_app("sonarr", "media");
        let json = serde_json::to_string(&app).expect("ser");
        let deserialized: AppConfig = serde_json::from_str(&json).expect("deser");
        assert_eq!(deserialized.name, "sonarr");
        assert_eq!(deserialized.category, "media");
        assert_eq!(deserialized.resource_requirements.cpu_request, "100m");
    }

    #[test]
    fn resource_requirements_serde() {
        let rr = ResourceRequirements {
            cpu_request: "250m".to_string(),
            cpu_limit: "2000m".to_string(),
            memory_request: "512Mi".to_string(),
            memory_limit: "2Gi".to_string(),
        };
        let json = serde_json::to_string(&rr).expect("ser");
        assert!(json.contains("\"cpu_request\":\"250m\""));
        let deser: ResourceRequirements = serde_json::from_str(&json).expect("deser");
        assert_eq!(deser.memory_limit, "2Gi");
    }

    #[test]
    fn volume_config_serde() {
        let vol = VolumeConfig {
            name: "config".to_string(),
            mount_path: "/config".to_string(),
            size: "5Gi".to_string(),
        };
        let json = serde_json::to_string(&vol).expect("ser");
        assert!(json.contains("\"mount_path\":\"/config\""));
        let deser: VolumeConfig = serde_json::from_str(&json).expect("deser");
        assert_eq!(deser.size, "5Gi");
    }

    #[test]
    fn app_config_with_volumes_and_env() {
        let mut app = make_app("jellyfin", "media");
        app.volumes.push(VolumeConfig {
            name: "config".to_string(),
            mount_path: "/config".to_string(),
            size: "1Gi".to_string(),
        });
        app.environment_variables
            .insert("PUID".to_string(), "1000".to_string());
        app.is_system = true;
        app.is_hidden = true;
        app.is_browseable = false;
        app.default_port = 8096;

        let json = serde_json::to_string(&app).expect("ser");
        assert!(json.contains("\"is_system\":true"));
        assert!(json.contains("\"is_hidden\":true"));
        assert!(json.contains("\"is_browseable\":false"));
        assert!(json.contains("\"default_port\":8096"));
    }
}
