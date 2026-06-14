# Quick Start

Get Kubarr running on your cluster in a few minutes.

## Prerequisites

- A Kubernetes cluster (k3s, K3d, Kind, EKS, GKE, AKS — anything 1.20+)
- `kubectl` configured to talk to your cluster
- `helm` 3.0+

## Install Kubarr

```bash
helm install kubarr oci://ghcr.io/smokeythebandit/kubarr-charts/kubarr \
  -n kubarr --create-namespace
```

Wait for everything to come up:

```bash
kubectl wait --for=condition=ready pod -l app.kubernetes.io/name=kubarr \
  -n kubarr --timeout=300s
```

## Open the Dashboard

```bash
kubectl port-forward -n kubarr svc/kubarr-frontend 8080:80
```

Visit [http://localhost:8080](http://localhost:8080). The setup wizard will walk you through creating your admin account.

::: tip
If you have an ingress controller, you can skip port-forwarding and set up a proper hostname instead. See the [Configuration Reference](configuration.md) for ingress options.
:::

## Deploy Your First App

1. Go to **Applications** → **Catalog**
2. Pick an app (e.g., Sonarr)
3. Click **Deploy** and accept the defaults
4. That's it — the app will be running in seconds

## What's Next

- **[GitHub](https://github.com/smokeythebandit/Kubarr)** — Source code and issue tracker

## Uninstall

```bash
helm uninstall kubarr -n kubarr
kubectl delete namespace kubarr
```

## Troubleshooting

### Pods not starting

```bash
kubectl get pods -n kubarr
kubectl describe pod -n kubarr <pod-name>
kubectl get events -n kubarr --sort-by='.lastTimestamp'
```

### Can't access the dashboard

Make sure the port-forward is still running — it drops when pods restart. Just run the `port-forward` command again.

### Need help?

- [GitHub Issues](https://github.com/smokeythebandit/Kubarr/issues)
- [GitHub Discussions](https://github.com/smokeythebandit/Kubarr/discussions)
