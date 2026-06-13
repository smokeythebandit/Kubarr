//! Chart sync service
//!
//! Discovers charts from GitHub and pulls them from an OCI registry
//! so the catalog always reflects the latest published versions.

use std::collections::VecDeque;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sea_orm::DatabaseConnection;
use serde::Deserialize;

use crate::config::CONFIG;
use crate::state::SharedCatalog;

/// GitHub Contents API entry
#[derive(Debug, Deserialize)]
struct GitHubContent {
    name: String,
    #[serde(rename = "type")]
    content_type: String,
}

/// Shared chart sync service used by both the scheduler and the on-demand endpoint.
pub struct ChartSyncService {
    catalog: SharedCatalog,
    client: reqwest::Client,
}

impl ChartSyncService {
    #[allow(clippy::expect_used)]
    pub fn new(catalog: SharedCatalog) -> Self {
        Self {
            catalog,
            client: reqwest::Client::builder()
                .user_agent("kubarr-backend")
                .timeout(Duration::from_secs(30))
                .build()
                .expect("failed to build reqwest client"),
        }
    }

    /// Discover chart names from the GitHub repo, pull each from OCI, and reload the catalog.
    pub async fn sync(&self) -> anyhow::Result<()> {
        let chart_names = self.discover_charts().await?;

        if chart_names.is_empty() {
            tracing::warn!("Chart sync: no charts discovered from GitHub");
            return Ok(());
        }

        let mut synced = 0u32;
        for name in &chart_names {
            match self.pull_chart(name) {
                Ok(()) => synced += 1,
                Err(e) => tracing::warn!("Chart sync: failed to pull {}: {}", name, e),
            }
        }

        // Reload the catalog from the (now-updated) charts directory
        {
            let mut catalog = self.catalog.write().await;
            catalog.reload();
        }

        tracing::info!("Chart sync completed, {} charts synced", synced);
        Ok(())
    }

    /// Query the GitHub Contents API to discover which chart directories exist.
    async fn discover_charts(&self) -> anyhow::Result<Vec<String>> {
        let mut names = Vec::new();
        let mut dirs = VecDeque::from([String::new()]);

        while let Some(dir) = dirs.pop_front() {
            let url = if dir.is_empty() {
                format!(
                    "https://api.github.com/repos/{}/contents/?ref={}",
                    CONFIG.charts.repo, CONFIG.charts.git_ref,
                )
            } else {
                format!(
                    "https://api.github.com/repos/{}/contents/{}?ref={}",
                    CONFIG.charts.repo, dir, CONFIG.charts.git_ref,
                )
            };

            let resp = self.client.get(&url).send().await?.error_for_status()?;
            let entries: Vec<GitHubContent> = resp.json().await?;
            let has_chart = entries
                .iter()
                .any(|e| e.name == "Chart.yaml" && e.content_type == "file");

            if has_chart {
                if let Some(name) = dir.rsplit('/').next() {
                    names.push(name.to_string());
                }
                continue;
            }

            for entry in entries {
                if entry.content_type == "dir" && !entry.name.starts_with('.') {
                    dirs.push_back(if dir.is_empty() {
                        entry.name
                    } else {
                        format!("{}/{}", dir, entry.name)
                    });
                }
            }
        }

        tracing::debug!("Chart sync: discovered {} charts from GitHub", names.len());
        Ok(names)
    }

    /// Pull a single chart from the OCI registry using `helm pull`.
    fn pull_chart(&self, name: &str) -> anyhow::Result<()> {
        let chart_ref = format!("{}/{}", CONFIG.charts.registry, name);
        let dest = CONFIG.charts.dir.to_str().unwrap_or("/app/charts");

        let output = Command::new("helm")
            .args(["pull", &chart_ref, "--untar", "--destination", dest])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("helm pull failed for {}: {}", name, stderr.trim());
        }

        tracing::debug!("Chart sync: pulled {}", name);
        Ok(())
    }
}

/// Periodic task wrapper that runs chart sync on an interval.
pub struct ChartSyncTask {
    pub service: Arc<ChartSyncService>,
}

#[async_trait]
impl super::scheduler::PeriodicTask for ChartSyncTask {
    fn name(&self) -> &'static str {
        "chart_sync"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(CONFIG.charts.sync_interval)
    }

    async fn run(&self, _db: &DatabaseConnection) -> anyhow::Result<()> {
        self.service.sync().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_content_deser_dir() {
        let json = r#"{"name":"sonarr","type":"dir"}"#;
        let entry: GitHubContent = serde_json::from_str(json).expect("deser");
        assert_eq!(entry.name, "sonarr");
        assert_eq!(entry.content_type, "dir");
    }

    #[test]
    fn github_content_deser_file() {
        let json = r#"{"name":"README.md","type":"file"}"#;
        let entry: GitHubContent = serde_json::from_str(json).expect("deser");
        assert_eq!(entry.name, "README.md");
        assert_eq!(entry.content_type, "file");
    }

    #[test]
    fn github_content_filter_dirs_only() {
        let entries = vec![
            GitHubContent {
                name: "sonarr".to_string(),
                content_type: "dir".to_string(),
            },
            GitHubContent {
                name: "README.md".to_string(),
                content_type: "file".to_string(),
            },
            GitHubContent {
                name: ".github".to_string(),
                content_type: "dir".to_string(),
            },
            GitHubContent {
                name: "radarr".to_string(),
                content_type: "dir".to_string(),
            },
        ];

        // Simulate the filter logic from discover_charts
        let names: Vec<String> = entries
            .into_iter()
            .filter(|e| e.content_type == "dir" && !e.name.starts_with('.'))
            .map(|e| e.name)
            .collect();

        assert_eq!(names, vec!["sonarr", "radarr"]);
    }

    #[test]
    fn chart_sync_service_new() {
        use crate::services::catalog::AppCatalog;
        use std::sync::Arc;
        use tokio::sync::RwLock;
        let catalog = Arc::new(RwLock::new(AppCatalog::new()));
        let _svc = ChartSyncService::new(catalog);
    }

    #[test]
    fn chart_sync_task_name() {
        use crate::services::catalog::AppCatalog;
        use crate::services::scheduler::PeriodicTask;
        use std::sync::Arc;
        use tokio::sync::RwLock;
        let catalog = Arc::new(RwLock::new(AppCatalog::new()));
        let service = Arc::new(ChartSyncService::new(catalog));
        let task = ChartSyncTask { service };
        assert_eq!(task.name(), "chart_sync");
    }

    #[test]
    fn chart_sync_task_interval_is_positive() {
        use crate::services::catalog::AppCatalog;
        use crate::services::scheduler::PeriodicTask;
        use std::sync::Arc;
        use tokio::sync::RwLock;
        let catalog = Arc::new(RwLock::new(AppCatalog::new()));
        let service = Arc::new(ChartSyncService::new(catalog));
        let task = ChartSyncTask { service };
        assert!(task.interval().as_secs() > 0);
    }

    #[test]
    fn github_content_filter_excludes_dot_dirs() {
        let entries = vec![
            GitHubContent {
                name: ".hidden".to_string(),
                content_type: "dir".to_string(),
            },
            GitHubContent {
                name: "visible".to_string(),
                content_type: "dir".to_string(),
            },
        ];

        let names: Vec<String> = entries
            .into_iter()
            .filter(|e| e.content_type == "dir" && !e.name.starts_with('.'))
            .map(|e| e.name)
            .collect();

        assert_eq!(names, vec!["visible"]);
    }
}
