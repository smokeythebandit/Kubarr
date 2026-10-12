use std::env;

use crate::install_events::Operation;
use crate::style::{detail, status_label, RED};
use crate::types::{
    BootstrapOptions, InstallOptions, StorageModeOption, WebAccess, APP_NAMESPACE,
    BACKEND_CHART_REF, BACKEND_IMAGE, BOOTSTRAP_RELEASE_NAMESPACE, FRONTEND_CHART_REF,
    FRONTEND_IMAGE, GATEWAY_CHART_REF, IMAGE_TAG, WORKER_CHART_REF, WORKER_IMAGE,
};
use crate::util::{
    chart_ref, check_tool, has_help, next_value, parse_port, try_run_or_print_progress,
};

pub fn install(args: Vec<String>) {
    if has_help(&args) {
        println!(
            "Install or upgrade Kubarr component charts with Helm.\n\nUSAGE:\n    kubarr install [OPTIONS]\n\nOPTIONS:\n    --frontend-node-port <port>   Expose OpenResty as NodePort on this port\n    --backend-node-port <port>    Expose backend as NodePort on this port\n    --values <path>               Extra Helm values file; repeatable\n    --wait                        Wait for resources to become ready\n    --dry-run                     Print commands without running them"
        );
        return;
    }

    let options = match InstallOptions::parse(args) {
        Ok(options) => options,
        Err(err) => {
            eprintln!("{} {err}", status_label("error", RED));
            std::process::exit(2);
        }
    };

    if let Err(err) = perform_install(&options) {
        eprintln!("{} {err}", status_label("error", RED));
        std::process::exit(1);
    }
}

pub fn perform_install(options: &InstallOptions) -> Result<(), String> {
    perform_install_with_storage(options, &[])
}

pub fn perform_bootstrap_install(options: &BootstrapOptions) -> Result<(), String> {
    let storage_values = storage_set_values(options);
    perform_install_with_storage(&options.install, &storage_values)
}

fn perform_install_with_storage(
    options: &InstallOptions,
    storage_values: &[(String, String)],
) -> Result<(), String> {
    if !options.dry_run {
        check_tool("helm")?;
    }
    detail("release", &options.release);
    detail("backend namespace", "kubarr-backend");
    detail("frontend namespace", "kubarr-frontend");
    detail("gateway namespace", "openresty");
    detail("worker namespace", "kubarr-worker");
    let backend_image = env_value("KUBARR_BACKEND_IMAGE", BACKEND_IMAGE);
    let frontend_image = env_value("KUBARR_FRONTEND_IMAGE", FRONTEND_IMAGE);
    let worker_image = env_value("KUBARR_WORKER_IMAGE", WORKER_IMAGE);
    let image_tag = env_value("KUBARR_IMAGE_TAG", IMAGE_TAG);
    let image_pull_policy = env_value("KUBARR_IMAGE_PULL_POLICY", "IfNotPresent");

    let mut backend_sets = vec![
        ("namespace.name".to_string(), "kubarr-backend".to_string()),
        (
            "database.secretName".to_string(),
            "kubarr-db-app".to_string(),
        ),
        ("image.repository".to_string(), backend_image),
        ("image.tag".to_string(), image_tag.clone()),
        ("image.pullPolicy".to_string(), image_pull_policy.clone()),
        (
            "storage.media.existingClaim".to_string(),
            "media-data".to_string(),
        ),
    ];
    for (key, value) in storage_values {
        backend_sets.push((key.clone(), value.clone()));
    }
    if options.web_access.is_some() {
        backend_sets.push(("service.type".into(), "ClusterIP".into()));
        backend_sets.push(("service.nodePort".into(), "null".into()));
        backend_sets.push(("service.port".into(), "8000".into()));
    }
    install_release(
        "kubarr-backend",
        "kubarr-backend",
        BACKEND_CHART_REF,
        options,
        &backend_sets,
    )?;

    let frontend_sets = vec![
        ("namespace.name".to_string(), "kubarr-frontend".to_string()),
        ("image.repository".to_string(), frontend_image),
        ("image.tag".to_string(), image_tag.clone()),
        ("image.pullPolicy".to_string(), image_pull_policy.clone()),
    ];
    install_release(
        "kubarr-frontend",
        "kubarr-frontend",
        FRONTEND_CHART_REF,
        options,
        &frontend_sets,
    )?;

    let mut gateway_sets = vec![
        ("namespace.name".to_string(), "openresty".to_string()),
        (
            "backend.service.namespace".to_string(),
            "kubarr-backend".to_string(),
        ),
        (
            "frontend.service.namespace".to_string(),
            "kubarr-frontend".to_string(),
        ),
    ];
    match options.web_access {
        Some(WebAccess::NodePort(port)) => {
            gateway_sets.push(("service.type".into(), "NodePort".into()));
            gateway_sets.push(("service.nodePort".into(), port.to_string()));
        }
        Some(WebAccess::LoadBalancer) => {
            gateway_sets.push(("service.type".into(), "LoadBalancer".into()));
        }
        Some(WebAccess::PortForward) => {
            gateway_sets.push(("service.type".into(), "ClusterIP".into()));
        }
        None => {
            if let Some(port) = options.frontend_node_port {
                gateway_sets.push(("service.type".into(), "NodePort".into()));
                gateway_sets.push(("service.nodePort".into(), port.to_string()));
            }
        }
    }
    if options.web_access.is_some() {
        gateway_sets.push(("service.port".into(), "8080".into()));
    }
    // The explicit LoadBalancer check renders this same chart version. Local
    // chart overrides use their path, and are revalidated at /api/start.
    install_release(
        "openresty",
        "openresty",
        GATEWAY_CHART_REF,
        options,
        &gateway_sets,
    )?;

    let mut worker_sets = vec![
        ("namespace.name".to_string(), "kubarr-worker".to_string()),
        (
            "database.secretName".to_string(),
            "kubarr-db-app".to_string(),
        ),
        ("image.repository".to_string(), worker_image),
        ("image.tag".to_string(), image_tag),
        ("image.pullPolicy".to_string(), image_pull_policy),
        (
            "storage.media.existingClaim".to_string(),
            "media-data".to_string(),
        ),
    ];
    for (key, value) in storage_values {
        worker_sets.push((key.clone(), value.clone()));
    }
    install_release(
        "kubarr-worker",
        "kubarr-worker",
        WORKER_CHART_REF,
        options,
        &worker_sets,
    )?;
    Ok(())
}

fn install_release(
    release: &str,
    chart_name: &str,
    chart_ref_default: &str,
    options: &InstallOptions,
    set_values: &[(String, String)],
) -> Result<(), String> {
    let chart = chart_ref(chart_name, chart_ref_default);
    detail("chart", &chart);

    // Keep Helm 3's client-side apply and readiness checks explicit under Helm 4.
    let mut helm_args = vec![
        "upgrade".to_string(),
        "--install".to_string(),
        "--server-side=false".to_string(),
        release.to_string(),
        chart,
        "-n".to_string(),
        BOOTSTRAP_RELEASE_NAMESPACE.to_string(),
    ];
    if release == "openresty" {
        if let Some(version) = &options.gateway_chart_version {
            if chart_ref_default.starts_with("oci://")
                && std::env::var_os("KUBARR_CHARTS_DIR").is_none()
            {
                helm_args.extend(["--version".to_string(), version.clone()]);
            }
        }
    }
    if let Some(port) = options
        .backend_node_port
        .filter(|_| options.web_access.is_none())
    {
        if release == "kubarr-backend" {
            helm_args.extend(["--set".to_string(), "service.type=NodePort".to_string()]);
            helm_args.extend(["--set".to_string(), format!("service.nodePort={port}")]);
        }
    }

    for (key, value) in set_values {
        helm_args.extend(["--set".to_string(), format!("{}={}", key, value)]);
    }
    for values in &options.values_files {
        helm_args.extend(["--values".to_string(), values.clone()]);
    }
    if options.wait {
        helm_args.push("--wait=legacy".to_string());
    }

    let helm_refs: Vec<&str> = helm_args.iter().map(String::as_str).collect();
    let operation = match release {
        "kubarr-backend" => Operation::Backend,
        "kubarr-frontend" => Operation::Frontend,
        "openresty" => Operation::Gateway,
        "kubarr-worker" => Operation::Worker,
        _ => return Err("unknown Kubarr release".into()),
    };
    try_run_or_print_progress("helm", &helm_refs, options.dry_run, false, Some(operation))
}

fn env_value(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn storage_set_values(options: &BootstrapOptions) -> Vec<(String, String)> {
    let mut values = vec![(
        "storage.media.existingClaim".to_string(),
        "media-data".to_string(),
    )];

    match &options.storage.mode {
        StorageModeOption::ManagedNfs(storage) => {
            values.push(("storage.media.nfs.size".to_string(), storage.size.clone()));
        }
        StorageModeOption::ExternalNfs(storage) => {
            values.push((
                "storage.media.nfs.server".to_string(),
                storage.server.clone(),
            ));
            values.push((
                "storage.media.nfs.path".to_string(),
                storage.export_path.clone(),
            ));
        }
    }

    values
}

impl InstallOptions {
    pub fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut options = Self::default_for_bootstrap(false);
        options.frontend_node_port = None;
        options.backend_node_port = None;
        options.wait = false;
        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--namespace" | "-n" => options.namespace = next_value(&mut iter, &arg)?,
                "--release" => options.release = next_value(&mut iter, &arg)?,
                "--frontend-node-port" => {
                    options.frontend_node_port = Some(parse_port(next_value(&mut iter, &arg)?)?)
                }
                "--backend-node-port" => {
                    options.backend_node_port = Some(parse_port(next_value(&mut iter, &arg)?)?)
                }
                "--values" | "-f" => options.values_files.push(next_value(&mut iter, &arg)?),
                "--wait" => options.wait = true,
                "--dry-run" => options.dry_run = true,
                other => return Err(format!("unknown install option '{other}'")),
            }
        }
        Ok(options)
    }

    pub fn default_for_bootstrap(dry_run: bool) -> Self {
        Self {
            namespace: APP_NAMESPACE.into(),
            release: "kubarr".into(),
            frontend_node_port: None,
            backend_node_port: Some(30081),
            web_access: None,
            gateway_chart_version: None,
            values_files: vec![],
            server_name: None,
            admin: None,
            wait: true,
            dry_run,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install_events::{self, INSTALL_TEST_LOCK};
    use std::sync::mpsc;

    fn plans(options: &InstallOptions) -> Vec<String> {
        let _guard = INSTALL_TEST_LOCK.lock().unwrap();
        let (tx, rx) = mpsc::channel();
        install_events::set_install_log(tx);
        perform_install(options).unwrap();
        install_events::clear_install_log();
        rx.into_iter()
            .filter_map(|event| match event {
                install_events::InstallEvent::Legacy(line) if line.starts_with("[PLAN] helm") => {
                    Some(line)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn web_modes_set_gateway_and_private_backend_without_touching_cluster() {
        for (mode, gateway_values) in [
            (
                WebAccess::NodePort(30123),
                "service.type=NodePort --set service.nodePort=30123",
            ),
            (WebAccess::LoadBalancer, "service.type=LoadBalancer"),
            (WebAccess::PortForward, "service.type=ClusterIP"),
        ] {
            let mut options = InstallOptions::default_for_bootstrap(true);
            options.web_access = Some(mode);
            options.backend_node_port = Some(30081); // A web selection overrides the legacy default.
            let commands = plans(&options);
            assert_eq!(commands.len(), 4);
            assert!(commands[0].contains("kubarr-backend"));
            assert!(commands[0].contains("service.type=ClusterIP --set service.nodePort=null"));
            assert!(commands[0].contains("service.port=8000"));
            assert!(!commands[0].contains("service.type=NodePort"));
            assert!(commands[2].contains("openresty"));
            assert!(commands[2].contains(gateway_values));
            assert!(commands[2].contains("service.port=8080"));
            if mode != WebAccess::NodePort(30123) {
                assert!(!commands[2].contains("service.nodePort="));
            }
        }
    }

    #[test]
    fn existing_cli_ports_keep_their_chart_targets() {
        let mut options = InstallOptions::parse(vec![
            "--frontend-node-port".into(),
            "30111".into(),
            "--backend-node-port".into(),
            "30112".into(),
            "--dry-run".into(),
        ])
        .unwrap();
        assert_eq!(options.web_access, None);
        let commands = plans(&options);
        assert!(commands[0].contains("service.nodePort=30112"));
        assert!(commands[2].contains("service.nodePort=30111"));
        options = InstallOptions::default_for_bootstrap(true);
        assert_eq!(options.backend_node_port, Some(30081));
    }

    #[test]
    fn web_loadbalancer_pins_gateway_chart_version_only() {
        let mut options = InstallOptions::default_for_bootstrap(true);
        options.web_access = Some(WebAccess::LoadBalancer);
        options.gateway_chart_version = Some("1.29.2+5.3".into());
        let commands = plans(&options);
        assert!(commands[..2]
            .iter()
            .all(|command| !command.contains("--version 1.29.2+5.3")));
        if std::env::var_os("KUBARR_CHARTS_DIR").is_none() {
            assert!(commands[2].contains("--version 1.29.2+5.3"));
        }
        assert!(!commands[3].contains("--version 1.29.2+5.3"));
    }
}
