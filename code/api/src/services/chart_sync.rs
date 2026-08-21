//! Chart sync service
//!
//! Discovers charts from GitHub and pulls them from an OCI registry
//! so the catalog always reflects the latest published versions.

use std::collections::VecDeque;
use std::fs;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use serde::Deserialize;

use crate::config::CONFIG;
use crate::models::app_state;
use crate::state::SharedCatalog;

/// GitHub Contents API entry
#[derive(Debug, Deserialize)]
struct GitHubContent {
    name: String,
    #[serde(rename = "type")]
    content_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoveredChart {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct ChartMetadata {
    version: String,
}

/// Shared chart sync service used by both the scheduler and the on-demand endpoint.
pub struct ChartSyncService {
    catalog: SharedCatalog,
    client: reqwest::Client,
    last_synced: tokio::sync::RwLock<Option<chrono::DateTime<chrono::Utc>>>,
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
            last_synced: tokio::sync::RwLock::new(None),
        }
    }

    /// When the last successful sync finished, if any.
    pub async fn last_synced(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.last_synced.read().await
    }

    pub async fn refresh_available_chart_versions(
        &self,
        db: &DatabaseConnection,
    ) -> anyhow::Result<()> {
        let catalog = self.catalog.read().await;
        let now = chrono::Utc::now();

        for state in app_state::Entity::find().all(db).await? {
            let available = catalog.chart_version(&state.app_name);
            let update_available = matches!(
                (&state.installed_chart_version, &available),
                (Some(installed), Some(available)) if installed != available
            );
            let mut active: app_state::ActiveModel = state.into();
            active.available_chart_version = Set(available);
            active.update_available = Set(update_available);
            active.last_checked_at = Set(Some(now));
            active.updated_at = Set(now);
            active.update(db).await?;
        }

        Ok(())
    }

    /// Discover chart names from the GitHub repo, pull each from OCI, and reload the catalog.
    pub async fn sync(self: &Arc<Self>) -> anyhow::Result<()> {
        let charts = match self.discover_charts().await {
            Ok(charts) => charts,
            Err(e) => {
                tracing::warn!(
                    "Chart sync: GitHub Contents API discovery failed, trying archive fallback: {}",
                    e
                );
                self.discover_charts_from_archive().await?
            }
        };

        if charts.is_empty() {
            tracing::warn!("Chart sync: no charts discovered from GitHub");
            return Ok(());
        }

        let mut synced = 0u32;
        for chart in &charts {
            // Helm/tar/filesystem work is blocking; keep it off the async
            // workers without re-entering the runtime (a nested `block_on`
            // here panics the timer driver if the runtime shuts down while a
            // sync is in flight, taking the whole process down with it).
            let service = self.clone();
            let chart = chart.clone();
            let chart_name = chart.name.clone();
            let chart_version = chart.version.clone();
            match tokio::task::spawn_blocking(move || service.pull_chart(&chart)).await {
                Ok(Ok(())) => synced += 1,
                Ok(Err(e)) => tracing::warn!(
                    "Chart sync: failed to pull {} {}: {}",
                    chart_name,
                    chart_version,
                    e
                ),
                Err(e) => {
                    tracing::warn!("Chart sync: pull task for {} panicked: {}", chart_name, e)
                }
            }
        }

        // Reload the catalog from the (now-updated) charts directory
        {
            let mut catalog = self.catalog.write().await;
            catalog.reload();
        }

        *self.last_synced.write().await = Some(chrono::Utc::now());
        tracing::info!("Chart sync completed, {} charts synced", synced);
        Ok(())
    }

    /// Run a full chart sync. Network discovery stays on the async runtime;
    /// the blocking Helm/tar work inside `sync` is dispatched to the blocking
    /// pool per chart, so async workers and HTTP probes are never starved.
    pub async fn sync_on_blocking_thread(self: Arc<Self>) -> anyhow::Result<()> {
        self.sync().await
    }

    /// Query the GitHub Contents API to discover which chart directories exist.
    async fn discover_charts(&self) -> anyhow::Result<Vec<DiscoveredChart>> {
        let mut charts = Vec::new();
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
                    let url = format!(
                        "https://raw.githubusercontent.com/{}/{}/{}/Chart.yaml",
                        CONFIG.charts.repo, CONFIG.charts.git_ref, dir,
                    );
                    let content = self
                        .client
                        .get(url)
                        .send()
                        .await?
                        .error_for_status()?
                        .text()
                        .await?;
                    charts.push(DiscoveredChart {
                        name: name.to_string(),
                        version: chart_version_from_yaml(&content)?,
                    });
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

        tracing::debug!("Chart sync: discovered {} charts from GitHub", charts.len());
        Ok(charts)
    }

    /// Download the source archive and discover charts without using the GitHub API.
    async fn discover_charts_from_archive(&self) -> anyhow::Result<Vec<DiscoveredChart>> {
        let url = format!(
            "https://github.com/{}/archive/{}.tar.gz",
            CONFIG.charts.repo, CONFIG.charts.git_ref,
        );
        let archive_path = std::env::temp_dir().join(format!(
            "kubarr-charts-{}-{}.tar.gz",
            std::process::id(),
            CONFIG.charts.git_ref.replace('/', "-")
        ));

        let bytes = self
            .client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        fs::write(&archive_path, bytes)?;

        let result = (|| {
            let output = Command::new("tar")
                .arg("-tzf")
                .arg(&archive_path)
                .output()?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("failed to list chart archive: {}", stderr.trim());
            }

            let mut charts = Vec::new();
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let path = line.trim_end_matches('/');
                if !path.ends_with("/Chart.yaml") {
                    continue;
                }

                let mut parts = path.split('/').collect::<Vec<_>>();
                if parts.len() < 3 {
                    continue;
                }

                parts.pop();
                if let Some(name) = parts.last() {
                    let name = (*name).to_string();
                    if charts
                        .iter()
                        .any(|chart: &DiscoveredChart| chart.name == name)
                    {
                        continue;
                    }

                    let chart_output = Command::new("tar")
                        .args(["-xOzf"])
                        .arg(&archive_path)
                        .arg(path)
                        .output()?;
                    if !chart_output.status.success() {
                        let stderr = String::from_utf8_lossy(&chart_output.stderr);
                        anyhow::bail!("failed to read {}: {}", path, stderr.trim());
                    }
                    let content = String::from_utf8(chart_output.stdout)?;
                    charts.push(DiscoveredChart {
                        name,
                        version: chart_version_from_yaml(&content)?,
                    });
                }
            }

            Ok::<_, anyhow::Error>(charts)
        })();
        let _ = fs::remove_file(&archive_path);
        let charts = result?;

        tracing::debug!(
            "Chart sync: discovered {} charts from GitHub archive",
            charts.len()
        );
        Ok(charts)
    }

    /// Pull a single chart from the OCI registry using `helm pull`.
    fn pull_chart(&self, chart: &DiscoveredChart) -> anyhow::Result<()> {
        let chart_ref = format!("{}/{}", CONFIG.charts.registry, chart.name);
        let dest = CONFIG.charts.dir.to_str().unwrap_or("/app/charts");
        std::fs::create_dir_all(dest)?;

        // `helm pull --untar` refuses to overwrite an existing chart dir, so
        // untar into a scratch dir and swap it into place; without this every
        // sync after the first fails for already-synced charts.
        let staging = std::path::Path::new(dest).join(format!(".pull-{}", chart.name));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;

        let output = Command::new("helm")
            .args([
                "pull",
                &chart_ref,
                "--version",
                &chart.version,
                "--untar",
                "--destination",
                &staging.to_string_lossy(),
            ])
            .output();
        let result = (|| {
            let output = output?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!(
                    "helm pull failed for {} {}: {}",
                    chart.name,
                    chart.version,
                    stderr.trim()
                );
            }
            let final_dir = std::path::Path::new(dest).join(&chart.name);
            let _ = std::fs::remove_dir_all(&final_dir);
            std::fs::rename(staging.join(&chart.name), &final_dir)?;
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&staging);
        result?;

        tracing::debug!("Chart sync: pulled {} {}", chart.name, chart.version);
        Ok(())
    }
}

fn chart_version_from_yaml(content: &str) -> anyhow::Result<String> {
    let metadata: ChartMetadata = serde_yaml::from_str(content)?;
    Ok(metadata.version)
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
        self.service.clone().sync_on_blocking_thread().await?;
        self.service.refresh_available_chart_versions(_db).await
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

    #[test]
    fn chart_version_preserves_build_metadata() {
        let version = chart_version_from_yaml("name: openresty\nversion: 1.29.2+5.1\n")
            .expect("chart metadata");
        assert_eq!(version, "1.29.2+5.1");
    }
}
