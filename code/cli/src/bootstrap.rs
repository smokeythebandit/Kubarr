use crate::bootstrap_web::run_web;
use crate::checks::check_cluster_prerequisites;
use crate::cluster::setup_cluster_if_needed;
use crate::database::install_database;
use crate::db_bootstrap::bootstrap_database;
use crate::install::perform_bootstrap_install;
use crate::install_access::print_access_hint;
use crate::observability::{
    install_fluent_bit, install_grafana, install_victorialogs, install_victoriametrics,
};
use crate::storage::{configure_storage, write_storage_config};
use crate::style::{print_banner, step};
use crate::types::BootstrapOptions;
use crate::types::{ClusterMode, StorageModeOption};
use crate::util::{check_tool, has_help, scope_target};
use crate::wizard::parse_bootstrap_options;

pub fn bootstrap(args: Vec<String>) -> Result<(), String> {
    if has_help(&args) {
        println!(
            "Set up Kubarr from scratch.\n\nUSAGE:\n    kubarr bootstrap [--web [--web-port <port>]]\n    kubarr bootstrap [INSTALL OPTIONS]\n\nWithout options, hosts the browser wizard. --web explicitly hosts it.\n--web-port accepts 0-65535 (0 selects an available port).\nThe wizard prints the location of a private 0600 local failure log (max 1 MiB).\nIt may contain sensitive subprocess output; remove it when finished.\n\nINSTALL OPTIONS:\n    --cluster-mode <mode>         existing or single-node\n    --namespace <name>            Namespace [default: kubarr-backend]\n    --release <name>              Release [default: kubarr]\n    --server-name <name>          Display name\n    --admin-username <name>       Initial admin username\n    --admin-email <email>         Initial admin email\n    --admin-password <password>   Initial admin password\n    --frontend-node-port <port>   Frontend NodePort\n    --backend-node-port <port>    Backend NodePort [default: 30081]\n    --values <path>               Extra Helm values file; repeatable\n    --storage-mode <mode>         managed-nfs or external-nfs\n    --storage-size <size>         Managed NFS PVC size [default: 1Ti]\n    --storage-class <name>        StorageClass for managed NFS PVC\n    --nfs-server <host>           External NFS server hostname or IP\n    --nfs-path <path>             External NFS export path\n    --grafana                     Install optional Grafana dashboards\n    --skip-cluster-check          Skip cluster prerequisites\n    --dry-run                     Print commands without running them"
        );
        return Ok(());
    }

    if args.is_empty()
        || args
            .first()
            .is_some_and(|arg| arg == "--web" || arg == "--web-port")
    {
        let mut port = 0;
        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--web" => {}
                "--web-port" => {
                    port = iter
                        .next()
                        .ok_or("missing value for --web-port")?
                        .parse::<u16>()
                        .map_err(|_| "--web-port must be between 0 and 65535")?
                }
                _ => return Err(format!("unknown web option '{arg}'")),
            }
        }
        return run_web(port);
    }

    print_banner();
    let options = parse_bootstrap_options(args)?;
    // Flag-driven installs own their k3s snapshot for the entire bootstrap;
    // the web installer already has its own per-thread target scope.
    let _target_scope = scope_target(None);
    run_bootstrap_install(&options)
}

pub fn run_bootstrap_install(options: &BootstrapOptions) -> Result<(), String> {
    if !options.install.dry_run {
        if crate::install_events::is_active() {
            step("Tools", "checking Helm 4 availability");
        }
        check_tool("helm")?;
    }
    setup_cluster_if_needed(options)?;
    if !options.install.dry_run {
        check_tool("kubectl")?;
        if !options.skip_cluster_check && !check_cluster_prerequisites(options) {
            return Err("bootstrap stopped because required cluster checks failed".into());
        }
    } else {
        step("Plan", "dry run requested; cluster checks are skipped");
    }

    // The web wizard cannot test storage before creating a fresh local k3s
    // cluster. Do this now, before any application components are installed.
    if !options.install.dry_run && matches!(options.cluster_mode, ClusterMode::SingleNode) {
        if let StorageModeOption::ExternalNfs(external) = &options.storage.mode {
            let input = crate::storage_probe::canonical(&external.server, &external.export_path)
                .map_err(str::to_string)?;
            step("Storage", "verifying external NFSv4.1 from local k3s");
            crate::storage_probe::verify_after_k3s(&input)?;
        }
    }

    configure_storage(options)?;
    install_fluent_bit(options)?;
    install_victorialogs(options)?;
    install_victoriametrics(options)?;
    if options.grafana_enabled {
        install_grafana(options)?;
    }
    install_database(options)?;
    step("Install", "installing Kubarr with Helm");
    perform_bootstrap_install(options)?;
    write_storage_config(options)?;
    bootstrap_database(options)?;
    print_access_hint(&options.install);
    Ok(())
}
