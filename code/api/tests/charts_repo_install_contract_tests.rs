//! Every published chart must be installable through the wizard.
//!
//! This test walks a real checkout of the charts repository
//! (`smokeythebandit/kubarr-charts`) rather than a hand-written fixture, so it
//! tracks the charts as they actually are. It exists because
//! `media-manager/recyclarr` once shipped a `storage.config` block that neither
//! created a claim nor named an existing one:
//!
//! ```yaml
//! storage:
//!   config:
//!     migrationConfirmed: false
//!     create: false
//!     existingClaim: ""
//!     size: 2Gi
//! ```
//!
//! `helm template` renders that happily, so the charts repository saw nothing
//! wrong, while the backend refused the chart outright: `install_options::load`
//! ends in `validate(&options, &InstallStorage::default())`, which rejects a
//! private config claim that is neither created nor named with "Select a
//! distinct existing private config claim". The wizard could not describe the
//! app and every queued install of it failed. Nothing in either repository
//! noticed.
//!
//! Set `KUBARR_CHARTS_REPO_DIR` to a checkout to run it; without that the test
//! skips, so a developer without the second repository is not blocked. CI
//! checks the charts repository out at `main` next to this one and runs it (the
//! `charts-contract` job in `.github/workflows/fast-test.yml`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use kubarr::services::catalog::AppCatalog;
use kubarr::services::install_options::{
    self, InstallOptions, InstallStorage, Mount, WorkloadMounts,
};

/// A chart directory in the checkout: the Helm chart name and where it lives.
struct Chart {
    name: String,
    dir: PathBuf,
}

/// Collect every chart in the checkout, skipping `charts/` (Helm's dependency
/// directory) and dotted directories such as `.git`.
fn charts_in(dir: &Path, found: &mut Vec<Chart>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) if !name.starts_with('.') && name != "charts" => name.to_string(),
            _ => continue,
        };
        if path.join("Chart.yaml").exists() {
            found.push(Chart { name, dir: path });
        } else {
            charts_in(&path, found);
        }
    }
}

fn yaml_at(path: &Path) -> serde_yaml::Value {
    let raw =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_yaml::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn annotation(chart_yaml: &serde_yaml::Value, key: &str) -> Option<String> {
    chart_yaml
        .get("annotations")?
        .get(key)?
        .as_str()
        .map(str::to_string)
}

fn string_list(value: Option<&serde_yaml::Value>) -> Option<Vec<String>> {
    value?.as_sequence().map(|paths| {
        paths
            .iter()
            .filter_map(|p| p.as_str().map(str::to_string))
            .collect()
    })
}

/// The paths a chart declares under `kubarr.storage.<kind>.<component>`,
/// re-derived here from the chart's own values so the assertion does not just
/// echo `install_options::declared`. An undeclared `reserved` list falls back
/// to the private claim at `/config`, matching kubarr-common's validator; an
/// undeclared `required` list means nothing is mandatory.
fn declared(values: &serde_yaml::Value, kind: &str, component: &str) -> Vec<String> {
    let listed = string_list(
        values
            .get("kubarr")
            .and_then(|k| k.get("storage"))
            .and_then(|s| s.get(kind))
            .and_then(|k| k.get(component)),
    );
    match (listed, kind) {
        (Some(paths), _) => paths,
        (None, "reserved") => vec!["/config".into()],
        (None, _) => Vec::new(),
    }
}

fn mounts_at(value: Option<&serde_yaml::Value>) -> Option<Vec<Mount>> {
    let value = value?;
    Some(
        serde_yaml::from_value(value.clone())
            .unwrap_or_else(|e| panic!("chart mount defaults are not Mounts: {e}")),
    )
}

/// What a caller that accepts everything the wizard offered sends back on
/// install: the chart's own defaults, spelled out as explicit choices.
fn accepting_defaults(options: &InstallOptions) -> InstallStorage {
    InstallStorage {
        mounts: options.mounts.clone(),
        workloads: (!options.workloads.is_empty()).then(|| {
            options
                .workloads
                .iter()
                .map(|(name, mounts)| {
                    (
                        name.clone(),
                        WorkloadMounts {
                            mounts: mounts.clone(),
                        },
                    )
                })
                .collect()
        }),
        config: options.config.clone(),
    }
}

/// Assert the options the wizard would show agree with the chart they came from.
fn assert_consistent(chart: &Chart, options: &InstallOptions) {
    let name = &chart.name;
    let values = yaml_at(&chart.dir.join("values.yaml"));
    let storage = values.get("storage");

    assert_eq!(&options.app_name, name);
    assert_eq!(
        Some(options.chart_version.as_str()),
        yaml_at(&chart.dir.join("Chart.yaml"))
            .get("version")
            .and_then(serde_yaml::Value::as_str),
        "{name}: reported chart version is not the one in Chart.yaml"
    );

    assert_eq!(
        options.mounts,
        mounts_at(storage.and_then(|s| s.get("mounts"))),
        "{name}: offered shared mounts are not the chart's storage.mounts"
    );
    let workloads: BTreeMap<String, Vec<Mount>> = storage
        .and_then(|s| s.get("workloads"))
        .and_then(serde_yaml::Value::as_mapping)
        .map(|map| {
            map.iter()
                .map(|(key, value)| {
                    let key = key
                        .as_str()
                        .unwrap_or_else(|| panic!("{name}: non-string storage.workloads key"));
                    let mounts = mounts_at(value.get("mounts"))
                        .unwrap_or_else(|| panic!("{name}: storage.workloads.{key} has no mounts"));
                    (key.to_string(), mounts)
                })
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        options.workloads, workloads,
        "{name}: offered per-workload mounts are not the chart's storage.workloads"
    );

    // The private claim is named after the release, so a chart that renames
    // itself or whose name would overflow a 63-character claim gets no
    // prediction at all rather than a wrong one.
    let predicted = options.config.as_ref().and_then(|_| {
        let claim = format!("{name}-config");
        let override_empty = |key: &str| {
            values
                .get(key)
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or("")
                .is_empty()
        };
        (claim.len() <= 63 && override_empty("fullnameOverride") && override_empty("nameOverride"))
            .then_some(claim)
    });
    assert_eq!(
        options.private_claim, predicted,
        "{name}: predicted private claim disagrees with the chart's naming"
    );
    assert_eq!(
        options.config.is_some(),
        storage.and_then(|s| s.get("config")).is_some(),
        "{name}: private config claim offered without (or withheld despite) storage.config"
    );

    // Required and reserved paths are described per component, for exactly the
    // components the chart offers mounts for -- nothing more, nothing less.
    let components: BTreeSet<String> = options
        .mounts
        .is_some()
        .then(|| "main".to_string())
        .into_iter()
        .chain(options.workloads.keys().cloned())
        .collect();
    assert_eq!(
        options.required.keys().cloned().collect::<BTreeSet<_>>(),
        components,
        "{name}: required paths are reported for the wrong set of components"
    );
    assert_eq!(
        options.reserved.keys().cloned().collect::<BTreeSet<_>>(),
        components,
        "{name}: reserved paths are reported for the wrong set of components"
    );
    for component in &components {
        assert_eq!(
            options.required[component],
            declared(&values, "required", component),
            "{name}: required paths for {component} are not kubarr.storage.required.{component}"
        );
        assert_eq!(
            options.reserved[component],
            declared(&values, "reserved", component),
            "{name}: reserved paths for {component} are not kubarr.storage.reserved.{component}"
        );
    }

    // A capability toggle exists because the chart ships the wiring and says
    // so; `enabled: false` is the off position of a switch that is there.
    assert_eq!(
        options.vpn_supported,
        values.get("vpn").and_then(|v| v.get("enabled")).is_some(),
        "{name}: VPN support does not follow the chart's vpn.enabled"
    );
    assert_eq!(
        options.gpu_supported,
        values.get("gpu").and_then(|g| g.get("enabled")).is_some(),
        "{name}: GPU support does not follow the chart's gpu.enabled"
    );

    assert!(
        options.saved_storage.is_none() && options.saved_chart_version.is_none(),
        "{name}: a never-installed app must not report saved storage"
    );
}

#[test]
fn every_offered_chart_in_the_charts_repository_can_be_installed() {
    let Some(checkout) = std::env::var_os("KUBARR_CHARTS_REPO_DIR").map(PathBuf::from) else {
        println!(
            "skipped: set KUBARR_CHARTS_REPO_DIR to a checkout of \
             smokeythebandit/kubarr-charts to check the published charts against the \
             install wizard"
        );
        return;
    };
    assert!(
        checkout.join("README.md").exists(),
        "KUBARR_CHARTS_REPO_DIR={} is not a kubarr-charts checkout",
        checkout.display()
    );

    let mut charts = Vec::new();
    charts_in(&checkout, &mut charts);
    charts.sort_by(|a, b| a.name.cmp(&b.name));
    assert!(
        charts.len() >= 10,
        "found only {} charts under {}; this is not the charts repository",
        charts.len(),
        checkout.display()
    );

    // `helm pull --untar` lands every chart directly in CONFIG.charts.dir, one
    // directory per chart name, while the repository groups them by category.
    // Stage the production-shaped flat layout the backend reads, with symlinks
    // so nothing is copied and the checkout is never written to.
    let staging = tempfile::tempdir().unwrap();
    for chart in &charts {
        let link = staging.path().join(&chart.name);
        assert!(
            !link.exists(),
            "two charts are both named {}; a flat chart directory cannot hold both",
            chart.name
        );
        std::os::unix::fs::symlink(&chart.dir, &link).unwrap();
    }

    // A separate integration-test binary owns its process environment, so
    // CONFIG is initialised against the staged charts on first read.
    std::env::set_var("KUBARR_CHARTS_DIR", staging.path());
    let catalog = AppCatalog::new();

    let mut installable = Vec::new();
    let mut refused = Vec::new();
    let mut uncatalogued = Vec::new();
    for chart in &charts {
        match catalog.get_app(&chart.name) {
            Some(app) if app.is_system || app.is_hidden => refused.push(chart),
            Some(_) => installable.push(chart),
            None => uncatalogued.push(chart),
        }
    }

    // A chart leaves the catalog only by declaring no `kubarr.io/category`.
    // Dropping out because its Chart.yaml stopped parsing would otherwise look
    // exactly like full coverage.
    for chart in &uncatalogued {
        let category = annotation(
            &yaml_at(&chart.dir.join("Chart.yaml")),
            "kubarr.io/category",
        );
        assert_eq!(
            category, None,
            "{} declares category {category:?} yet never reached the catalog",
            chart.name
        );
    }

    for chart in &refused {
        let error = install_options::load(&catalog, &chart.name)
            .expect_err("a system or hidden chart must not be installable as an app")
            .to_string();
        assert!(
            error.contains("This chart is not available for app installs"),
            "{}: refused for the wrong reason: {error}",
            chart.name
        );
    }

    for chart in &installable {
        // The wizard cannot offer an app it cannot describe, and every queued
        // install of it fails with whatever this returns.
        let options = install_options::load(&catalog, &chart.name).unwrap_or_else(|e| {
            panic!(
                "{} cannot be offered by the install wizard: {e}\n  chart: {}",
                chart.name,
                chart.dir.display()
            )
        });
        assert_consistent(chart, &options);
        // What the wizard itself checks on install, with the defaults the user
        // was shown: this is the check recyclarr failed.
        install_options::validate(&options, &InstallStorage::default()).unwrap_or_else(|e| {
            panic!("{}: the chart's own defaults are rejected: {e}", chart.name)
        });
        install_options::validate(&options, &accepting_defaults(&options)).unwrap_or_else(|e| {
            panic!(
                "{}: accepting the offered defaults unchanged is rejected: {e}",
                chart.name
            )
        });
    }

    let names = |charts: &[&Chart]| {
        charts
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!(
        "charts repository at {}: {} charts\n  \
         installable ({}): {}\n  \
         system or hidden ({}): {}\n  \
         not catalogued, no kubarr.io/category ({}): {}",
        checkout.display(),
        charts.len(),
        installable.len(),
        names(&installable),
        refused.len(),
        names(&refused),
        uncatalogued.len(),
        names(&uncatalogued),
    );
    assert!(
        installable.len() >= 10,
        "only {} installable charts; the catalog is not seeing the repository",
        installable.len()
    );
    // The chart this test was written for. It is a regular media-manager app,
    // so if it ever stops being offered, that is the regression, not progress.
    assert!(
        installable.iter().any(|c| c.name == "recyclarr"),
        "recyclarr is no longer offered as an installable app"
    );
}
