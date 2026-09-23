//! Chart sync service
//!
//! Discovers charts from GitHub and pulls them from an OCI registry
//! so the catalog always reflects the latest published versions.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
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
    name: String,
    version: String,
}

/// Shared chart sync service used by both the scheduler and the on-demand endpoint.
pub struct ChartSyncService {
    catalog: SharedCatalog,
    client: reqwest::Client,
    sync_lock: tokio::sync::Mutex<()>,
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
            sync_lock: tokio::sync::Mutex::new(()),
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

        for initial_state in app_state::Entity::find().all(db).await? {
            let observed_available = catalog.chart_version(&initial_state.app_name);
            let mut state = initial_state;
            loop {
                let available = observed_available
                    .clone()
                    .or_else(|| state.available_chart_version.clone());
                let update_available = matches!(
                    (&state.installed_chart_version, &available),
                    (Some(installed), Some(available)) if installed != available
                );
                let installed_before = state.installed_chart_version.clone();
                let available_before = state.available_chart_version.clone();
                let app_name = state.app_name.clone();
                let mut active: app_state::ActiveModel = state.into();
                active.available_chart_version = Set(available);
                active.update_available = Set(update_available);
                active.last_checked_at = Set(Some(now));
                active.updated_at = Set(now);
                let mut update = app_state::Entity::update_many()
                    .set(active)
                    .filter(app_state::Column::AppName.eq(&app_name));
                update = update.filter(match installed_before.as_deref() {
                    Some(version) => app_state::Column::InstalledChartVersion.eq(version),
                    None => app_state::Column::InstalledChartVersion.is_null(),
                });
                update = update.filter(match available_before.as_deref() {
                    Some(version) => app_state::Column::AvailableChartVersion.eq(version),
                    None => app_state::Column::AvailableChartVersion.is_null(),
                });
                if update.exec(db).await?.rows_affected != 0 {
                    break;
                }
                let Some(current) = app_state::Entity::find_by_id(app_name).one(db).await? else {
                    break;
                };
                state = current;
            }
        }

        Ok(())
    }

    /// Discover chart names from the GitHub repo, pull each from OCI, and reload the catalog.
    pub async fn sync(self: &Arc<Self>) -> anyhow::Result<()> {
        self.run_serialized(self.sync_transaction()).await
    }

    /// Sync the process catalog and persist its versions as the shared authoritative targets.
    pub async fn sync_and_refresh(self: &Arc<Self>, db: &DatabaseConnection) -> anyhow::Result<()> {
        self.run_serialized(async {
            self.sync_transaction().await?;
            self.refresh_available_chart_versions(db).await
        })
        .await
    }

    async fn run_serialized<F>(&self, transaction: F) -> anyhow::Result<()>
    where
        F: std::future::Future<Output = anyhow::Result<()>>,
    {
        let _guard = self.sync_lock.lock().await;
        transaction.await
    }

    async fn sync_transaction(self: &Arc<Self>) -> anyhow::Result<()> {
        let charts = if let Some(source_dir) = CONFIG.charts.source_dir.as_deref() {
            discover_charts_from_local_source(source_dir)?
        } else {
            match self.discover_charts().await {
                Ok(charts) => charts,
                Err(e) => {
                    tracing::warn!(
                        "Chart sync: GitHub Contents API discovery failed, trying archive fallback: {}",
                        e
                    );
                    self.discover_charts_from_archive().await?
                }
            }
        };

        if charts.is_empty() {
            tracing::warn!("Chart sync: no charts discovered from GitHub");
            return Ok(());
        }

        self.sync_discovered_with(charts, |chart| {
            // Helm/tar/filesystem work is blocking; keep it off the async
            // workers without re-entering the runtime (a nested `block_on`
            // here panics the timer driver if the runtime shuts down while a
            // sync is in flight, taking the whole process down with it).
            let service = self.clone();
            async move {
                tokio::task::spawn_blocking(move || service.pull_chart(&chart))
                    .await
                    .map_err(|error| anyhow::anyhow!("pull task panicked: {error}"))?
            }
        })
        .await
    }

    async fn sync_discovered_with<P, F>(
        &self,
        charts: Vec<DiscoveredChart>,
        mut pull: P,
    ) -> anyhow::Result<()>
    where
        P: FnMut(DiscoveredChart) -> F,
        F: std::future::Future<Output = anyhow::Result<()>>,
    {
        let mut synced = 0usize;
        let mut failures = Vec::new();
        for chart in charts {
            let name = chart.name.clone();
            let version = chart.version.clone();
            match pull(chart).await {
                Ok(()) => synced += 1,
                Err(error) => {
                    tracing::warn!("Chart sync: failed to pull {name} {version}: {error}");
                    failures.push(format!("{name} {version}: {error}"));
                }
            }
        }

        // Reload even after a partial failure: successfully pulled charts are
        // complete atomic directory swaps and are safer to expose than to hide
        // behind a stale in-memory catalog. The transaction still fails and
        // does not advance last_synced until every requested chart succeeds.
        {
            let mut catalog = self.catalog.write().await;
            catalog.reload();
        }

        if !failures.is_empty() {
            anyhow::bail!("chart sync failed for: {}", failures.join("; "));
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

        let args = helm_pull_args(
            &chart_ref,
            &chart.version,
            &staging.to_string_lossy(),
            CONFIG.charts.plain_http,
        );
        let output = Command::new("helm").args(&args).output();
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
    #[derive(Deserialize)]
    struct VersionMetadata {
        version: String,
    }
    let metadata: VersionMetadata = serde_yaml::from_str(content)?;
    Ok(metadata.version)
}

fn discover_charts_from_local_source(root: &Path) -> anyhow::Result<Vec<DiscoveredChart>> {
    if !root.is_dir() {
        anyhow::bail!(
            "KUBARR_CHARTS_SOURCE_DIR is not a readable directory: {}",
            root.display()
        );
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        anyhow::anyhow!(
            "failed to resolve KUBARR_CHARTS_SOURCE_DIR {}: {error}",
            root.display()
        )
    })?;

    let mut pending = VecDeque::from([root.to_path_buf()]);
    let mut names = HashSet::new();
    let mut charts = Vec::new();
    while let Some(dir) = pending.pop_front() {
        let entries = fs::read_dir(&dir).map_err(|error| {
            anyhow::anyhow!("failed to read chart source {}: {error}", dir.display())
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                anyhow::anyhow!("failed to read chart source {}: {error}", dir.display())
            })?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push_back(entry.path());
            }
        }

        let metadata_path = dir.join("Chart.yaml");
        let file_type = match fs::symlink_metadata(&metadata_path) {
            Ok(metadata) => metadata.file_type(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                anyhow::bail!("failed to inspect {}: {error}", metadata_path.display())
            }
        };
        let metadata_read_path = if file_type.is_file() {
            metadata_path.clone()
        } else if file_type.is_symlink() {
            let target = fs::canonicalize(&metadata_path).map_err(|error| {
                anyhow::anyhow!(
                    "failed to resolve chart metadata symlink {}: {error}",
                    metadata_path.display()
                )
            })?;
            if !target.starts_with(&canonical_root) {
                anyhow::bail!(
                    "chart metadata symlink {} resolves outside KUBARR_CHARTS_SOURCE_DIR to {}",
                    metadata_path.display(),
                    target.display()
                );
            }
            if !target.is_file() {
                anyhow::bail!(
                    "chart metadata symlink {} does not resolve to a regular file",
                    metadata_path.display()
                );
            }
            target
        } else {
            continue;
        };
        let content = fs::read_to_string(&metadata_read_path)?;
        let metadata: ChartMetadata = serde_yaml::from_str(&content).map_err(|error| {
            anyhow::anyhow!(
                "invalid chart metadata {}: {error}",
                metadata_path.display()
            )
        })?;
        validate_chart_name(&metadata.name).map_err(|error| {
            anyhow::anyhow!("invalid chart name in {}: {error}", metadata_path.display())
        })?;
        validate_chart_version(&metadata.version).map_err(|error| {
            anyhow::anyhow!(
                "invalid chart version in {}: {error}",
                metadata_path.display()
            )
        })?;
        if !names.insert(metadata.name.clone()) {
            anyhow::bail!("duplicate chart name '{}' in local source", metadata.name);
        }
        charts.push(DiscoveredChart {
            name: metadata.name,
            version: metadata.version,
        });
    }

    if charts.is_empty() {
        anyhow::bail!(
            "KUBARR_CHARTS_SOURCE_DIR contains no charts: {}",
            root.display()
        );
    }
    charts.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(charts)
}

fn validate_chart_name(name: &str) -> anyhow::Result<()> {
    let safe = !name.is_empty()
        && name.len() <= 253
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'.'
        })
        && name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && name
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && !name.contains("..");
    if !safe {
        anyhow::bail!("'{name}' is not a safe Helm/OCI chart name");
    }
    Ok(())
}

fn validate_chart_version(version: &str) -> anyhow::Result<()> {
    let mut build_parts = version.split('+');
    let version_without_build = build_parts.next().unwrap_or_default();
    let build = build_parts.next();
    let one_build = build_parts.next().is_none();
    let mut prerelease_parts = version_without_build.splitn(2, '-');
    let core = prerelease_parts.next().unwrap_or_default();
    let prerelease = prerelease_parts.next();
    let core_is_semver = core.split('.').count() == 3
        && core.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'))
        });
    let identifiers_are_valid = |identifiers: &str, reject_leading_zero: bool| {
        !identifiers.is_empty()
            && identifiers.split('.').all(|identifier| {
                !identifier.is_empty()
                    && identifier
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                    && !(reject_leading_zero
                        && identifier.len() > 1
                        && identifier.bytes().all(|byte| byte.is_ascii_digit())
                        && identifier.starts_with('0'))
            })
    };
    let safe = !version.is_empty()
        && core_is_semver
        && one_build
        && prerelease.is_none_or(|value| identifiers_are_valid(value, true))
        && build.is_none_or(|value| identifiers_are_valid(value, false));
    if !safe {
        anyhow::bail!("'{version}' is not a safe semantic chart version");
    }
    Ok(())
}

fn helm_pull_args(
    chart_ref: &str,
    version: &str,
    destination: &str,
    plain_http: bool,
) -> Vec<String> {
    let mut args = vec![
        "pull".to_string(),
        chart_ref.to_string(),
        "--version".to_string(),
        version.to_string(),
        "--untar".to_string(),
        "--destination".to_string(),
        destination.to_string(),
    ];
    if plain_http {
        args.push("--plain-http".to_string());
    }
    args
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
        self.service.sync_and_refresh(_db).await
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

    fn write_chart(parent: &Path, directory: &str, name: &str, version: &str) {
        let directory = parent.join(directory);
        fs::create_dir_all(&directory).expect("chart directory");
        fs::write(
            directory.join("Chart.yaml"),
            format!("apiVersion: v2\nname: {name}\nversion: {version}\n"),
        )
        .expect("chart metadata");
    }

    fn discovered(name: &str, version: &str) -> DiscoveredChart {
        DiscoveredChart {
            name: name.into(),
            version: version.into(),
        }
    }

    fn test_service() -> Arc<ChartSyncService> {
        use crate::services::catalog::AppCatalog;
        use std::collections::HashMap;
        use tokio::sync::RwLock;

        Arc::new(ChartSyncService::new(Arc::new(RwLock::new(
            AppCatalog::with_apps(HashMap::new()),
        ))))
    }

    #[tokio::test]
    async fn serialized_sync_success_updates_last_synced() {
        let service = test_service();
        assert_eq!(service.last_synced().await, None);

        service
            .run_serialized(
                service
                    .sync_discovered_with(vec![discovered("alpha", "1.0.0")], |_| async { Ok(()) }),
            )
            .await
            .expect("sync succeeds");

        assert!(service.last_synced().await.is_some());
    }

    #[tokio::test]
    async fn serialized_sync_failure_is_aggregated_and_preserves_last_synced() {
        let service = test_service();
        service
            .run_serialized(
                service.sync_discovered_with(vec![discovered("initial", "1.0.0")], |_| async {
                    Ok(())
                }),
            )
            .await
            .expect("initial sync");
        let initial_timestamp = service.last_synced().await;

        let error = service
            .run_serialized(service.sync_discovered_with(
                vec![discovered("alpha", "1.0.0"), discovered("beta", "2.0.0")],
                |chart| async move { anyhow::bail!("injected failure for {}", chart.name) },
            ))
            .await
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("alpha 1.0.0"), "{message}");
        assert!(message.contains("beta 2.0.0"), "{message}");
        assert_eq!(service.last_synced().await, initial_timestamp);
    }

    #[tokio::test]
    async fn concurrent_sync_transactions_execute_one_at_a_time() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::sync::{Barrier, Notify};

        let service = test_service();
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let first_entered = Arc::new(Notify::new());
        let release_first = Arc::new(Notify::new());

        let first_service = service.clone();
        let first_active = active.clone();
        let first_max = max_active.clone();
        let first_entered_task = first_entered.clone();
        let first_release = release_first.clone();
        let first = tokio::spawn(async move {
            first_service
                .run_serialized(first_service.sync_discovered_with(
                    vec![discovered("first", "1.0.0")],
                    move |_| {
                        let active = first_active.clone();
                        let max_active = first_max.clone();
                        let entered = first_entered_task.clone();
                        let release = first_release.clone();
                        async move {
                            let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                            max_active.fetch_max(count, Ordering::SeqCst);
                            entered.notify_one();
                            release.notified().await;
                            active.fetch_sub(1, Ordering::SeqCst);
                            Ok(())
                        }
                    },
                ))
                .await
        });
        first_entered.notified().await;

        let second_started = Arc::new(Barrier::new(2));
        let second_service = service.clone();
        let second_active = active.clone();
        let second_max = max_active.clone();
        let second_barrier = second_started.clone();
        let second = tokio::spawn(async move {
            second_barrier.wait().await;
            second_service
                .run_serialized(second_service.sync_discovered_with(
                    vec![discovered("second", "1.0.0")],
                    move |_| {
                        let active = second_active.clone();
                        let max_active = second_max.clone();
                        async move {
                            let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                            max_active.fetch_max(count, Ordering::SeqCst);
                            active.fetch_sub(1, Ordering::SeqCst);
                            Ok(())
                        }
                    },
                ))
                .await
        });
        second_started.wait().await;
        // Give the second transaction a deterministic opportunity to contend
        // for the lock while the first pull remains blocked by the notification.
        tokio::task::yield_now().await;
        release_first.notify_one();

        first.await.expect("first task").expect("first sync");
        second.await.expect("second task").expect("second sync");
        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn local_source_discovers_metadata_names_and_nested_charts() {
        let source = tempfile::tempdir().expect("source");
        write_chart(source.path(), "arbitrary-a", "alpha", "1.2.3");
        write_chart(
            source.path(),
            "nested/arbitrary-b",
            "beta",
            "2.0.0-rc.1+build.7",
        );
        write_chart(source.path(), ".hidden/ignored", "ignored", "1.0.0");

        let charts = discover_charts_from_local_source(source.path()).expect("discovery");
        assert_eq!(
            charts,
            vec![
                DiscoveredChart {
                    name: "alpha".into(),
                    version: "1.2.3".into(),
                },
                DiscoveredChart {
                    name: "beta".into(),
                    version: "2.0.0-rc.1+build.7".into(),
                },
            ]
        );
    }

    #[test]
    fn local_source_rejects_missing_empty_and_duplicate_catalogs() {
        let source = tempfile::tempdir().expect("source");
        assert!(discover_charts_from_local_source(&source.path().join("missing")).is_err());
        assert!(discover_charts_from_local_source(source.path()).is_err());

        write_chart(source.path(), "one", "duplicate", "1.0.0");
        write_chart(source.path(), "nested/two", "duplicate", "2.0.0");
        let error = discover_charts_from_local_source(source.path()).unwrap_err();
        assert!(
            error.to_string().contains("duplicate chart name"),
            "{error}"
        );
    }

    #[test]
    fn local_source_rejects_missing_or_unsafe_metadata() {
        for (metadata, expected) in [
            ("apiVersion: v2\nversion: 1.0.0\n", "name"),
            (
                "apiVersion: v2\nname: ../escape\nversion: 1.0.0\n",
                "chart name",
            ),
            (
                "apiVersion: v2\nname: Uppercase\nversion: 1.0.0\n",
                "chart name",
            ),
            ("apiVersion: v2\nname: valid\n", "version"),
            (
                "apiVersion: v2\nname: valid\nversion: latest\n",
                "chart version",
            ),
            (
                "apiVersion: v2\nname: valid\nversion: 1.2.3-\n",
                "chart version",
            ),
        ] {
            let source = tempfile::tempdir().expect("source");
            fs::write(source.path().join("Chart.yaml"), metadata).expect("metadata");
            let error = discover_charts_from_local_source(source.path()).unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn helm_pull_plain_http_is_opt_in() {
        let secure = helm_pull_args("oci://registry/chart", "1.0.0", "/cache", false);
        assert!(!secure.iter().any(|arg| arg == "--plain-http"));
        let plain = helm_pull_args("oci://registry/chart", "1.0.0", "/cache", true);
        assert_eq!(plain.last().map(String::as_str), Some("--plain-http"));
    }

    #[cfg(unix)]
    #[test]
    fn local_source_reads_configmap_symlink_and_observes_data_swap() {
        use std::os::unix::fs::symlink;

        let source = tempfile::tempdir().expect("source");
        write_chart(source.path(), "..2026_a", "configmap-chart", "1.0.0");
        write_chart(source.path(), "..2026_b", "configmap-chart", "2.0.0");
        symlink("..2026_a", source.path().join("..data")).expect("initial ..data symlink");
        symlink("..data/Chart.yaml", source.path().join("Chart.yaml"))
            .expect("ConfigMap key symlink");

        assert_eq!(
            discover_charts_from_local_source(source.path()).expect("initial discovery"),
            vec![DiscoveredChart {
                name: "configmap-chart".into(),
                version: "1.0.0".into(),
            }]
        );

        fs::remove_file(source.path().join("..data")).expect("remove old ..data symlink");
        symlink("..2026_b", source.path().join("..data")).expect("replacement ..data symlink");
        assert_eq!(
            discover_charts_from_local_source(source.path()).expect("discovery after swap"),
            vec![DiscoveredChart {
                name: "configmap-chart".into(),
                version: "2.0.0".into(),
            }]
        );
    }

    #[cfg(unix)]
    #[test]
    fn local_source_rejects_chart_metadata_symlink_outside_root() {
        use std::os::unix::fs::symlink;

        let source = tempfile::tempdir().expect("source");
        let outside = tempfile::tempdir().expect("outside");
        fs::write(
            outside.path().join("Chart.yaml"),
            "apiVersion: v2\nname: outside\nversion: 1.0.0\n",
        )
        .expect("outside metadata");
        symlink(
            outside.path().join("Chart.yaml"),
            source.path().join("Chart.yaml"),
        )
        .expect("outside symlink");

        let error = discover_charts_from_local_source(source.path()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("resolves outside KUBARR_CHARTS_SOURCE_DIR"),
            "{error}"
        );
    }
}
