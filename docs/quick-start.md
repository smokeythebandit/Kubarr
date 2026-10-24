# Quick Start

Install Kubarr with the standalone CLI's browser wizard.

## Prerequisites

- Linux x86_64 or ARM64 for the installer binary.
- An existing Kubernetes cluster and usable kubeconfig, or a host on which to create local single-node k3s.
- Helm 4.3 supports Kubernetes 1.34–1.37; release E2E uses 1.35.8.

The wizard checks the selected destination and required tools. Creating local k3s requires host-administration privileges; installing the CLI and hosting the wizard run as your normal user.

## Install Kubarr

```sh
curl -fsSL https://kubarr.com/install.sh | bash
```

The script installs `~/.local/bin/kubarr` and starts the wizard. Open the printed invitation URL and keep the terminal running until setup finishes.

1. Select an existing context, upload a kubeconfig, or choose local k3s.
2. Configure dashboard access and storage.
3. Create the initial administrator and select optional Grafana.
4. Review the choices and start installation.

The CLI installs separate storage, database, monitoring, API, worker, frontend, and gateway charts. There is no umbrella `kubarr` chart.

To reopen the wizard later:

```sh
export PATH="$HOME/.local/bin:$PATH"
kubarr bootstrap
```

For remote-host access, storage checks, and diagnostics, see the CLI's [Bootstrap Guide](https://github.com/smokeythebandit/kubarr-cli/blob/main/docs/bootstrap.md).

## Open the Dashboard

Use the access instructions shown when installation completes:

- **NodePort:** `http://<node-ip>:30080`, or the port selected in the wizard.
- **LoadBalancer:** the gateway's assigned address on port 8080.
- **Port-forward:** with kubectl targeting the installed cluster, run:

  ```sh
  kubectl port-forward -n openresty svc/kubarr-gateway 8080:8080
  ```

  Open [http://localhost:8080](http://localhost:8080) on the forwarding machine.

Sign in with the administrator account created during installation. Dashboard traffic goes through the OpenResty gateway.

## Install Your First App

1. Open **Apps** and select an available catalog app, such as Sonarr.
2. Start its installation wizard and review its storage mounts and configuration claim.
3. Confirm the installation and follow its status in **Operations**.

See the [App Installation Wizard](install-wizard.md) and [Storage](storage.md) guides for details.

## Troubleshooting

```sh
kubarr doctor
kubectl get pods -n kubarr-backend
kubectl get pods -n kubarr-worker
kubectl get pods -n openresty
kubectl get svc -n openresty kubarr-gateway
```

Inspect a failing pod in its workload namespace:

```sh
kubectl describe pod -n <namespace> <pod-name>
kubectl get events -n <namespace> --sort-by='.lastTimestamp'
```

For port-forward access, keep the forwarding command running and restart it if its pod connection drops.

## Upgrades and Removal

Read the [storage migration guide](https://github.com/smokeythebandit/kubarr-charts/blob/main/STORAGE-MIGRATION.md) before upgrading an existing installation to declarative storage.

Platform components are separate releases. Use `helm list -A` to identify their release names and release namespaces before removing individual components with `helm uninstall <release> -n <release-namespace>`. Workload namespaces can differ from release namespaces. Review retained PVCs, PVs, and stored data separately; uninstalling a release is not a data-erasure procedure.

## Next Steps

- [Configuration](configuration.md)
- [Architecture](architecture.md)
- [Development](development.md)
- [GitHub Issues](https://github.com/smokeythebandit/Kubarr/issues)
