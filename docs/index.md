# Kubarr

**Smooth sailing for self-hosted media on Kubernetes.**

![Version](https://img.shields.io/badge/version-{{VERSION}}-blue?style=flat-square)
![Release](https://img.shields.io/badge/channel-{{CHANNEL}}-green?style=flat-square)
![Build](https://img.shields.io/badge/commit-{{COMMIT}}-lightgrey?style=flat-square)

---

## What is Kubarr?

Kubarr is a dashboard that makes it simple to deploy and manage your media automation apps — Sonarr, Radarr, Prowlarr, Lidarr, and more — on a Kubernetes cluster. Think of it like [Swizzin](https://swizzin.ltd/) or [Saltbox](https://saltbox.dev/), but built for Kubernetes.

No YAML wrangling. No Helm headaches. Just pick the apps you want, click deploy, and you're done.

## Why Kubarr?

- **One-click deploys** — Pick apps from the catalog and deploy them with sensible defaults
- **Everything in one place** — Monitor all your apps, check logs, and manage configs from a single dashboard
- **Real-time status** — See at a glance what's running, what's healthy, and what needs attention
- **Easy configuration** — Edit settings, environment variables, and secrets through the UI instead of digging through files
- **Notifications** — Get alerts via email or webhook when something goes wrong
- **Secure by default** — User accounts, roles, and audit logging out of the box

## Get Started

```bash
helm install kubarr oci://ghcr.io/smokeythebandit/kubarr-charts/kubarr -n kubarr --create-namespace
```

That's it. Once the pods are ready, open the dashboard and log in with the credentials from the install output.

For more details, see the [Quick Start Guide](quick-start.md).

## What Can You Deploy?

Kubarr comes with a curated catalog of popular homelab apps. Deploy them individually or build out your full media stack:

| Category | Apps |
|----------|------|
| **Media Management** | Sonarr, Radarr, Lidarr, Readarr |
| **Indexers** | Prowlarr, Jackett |
| **Download Clients** | qBittorrent, SABnzbd |
| **Media Servers** | Plex, Jellyfin |
| **Utilities** | Overseerr, Tautulli, Organizr |

## Next Steps

- **[Quick Start](quick-start.md)** — Get running in 15 minutes
- **[Versioning System](versioning.md)** — Learn about releases and version management

---

::: info Documentation Version
You are viewing documentation for **Kubarr v{{VERSION}}** ({{CHANNEL}} channel).

- **Version**: {{VERSION}}
- **Release Channel**: {{CHANNEL}}
- **Commit**: {{COMMIT}}

See the [Versioning Guide](versioning.md) for release information and how to upgrade.
:::
