# Kubarr Architecture

Kubarr is split into a small set of focused processes: a browser UI, an OpenResty gateway, a Rust API, and a Rust worker. The gateway owns HTTP routing and app proxying, the API owns user-facing control-plane requests, and the worker owns long-running app lifecycle work.

## Overview

<div class="architecture-map" role="img" aria-label="Kubarr architecture diagram">
  <div class="arch-row arch-edge">
    <div class="arch-node arch-user">
      <span class="arch-kicker">Users</span>
      <strong>Browser</strong>
      <small>Dashboard, app access, API calls</small>
    </div>
  </div>

  <div class="arch-arrow">HTTP :30080</div>

  <div class="arch-row">
    <div class="arch-node arch-gateway">
      <span class="arch-kicker">Gateway</span>
      <strong>OpenResty</strong>
      <small>Static routing, auth-gated app proxy, WebSocket upgrades</small>
    </div>
  </div>

  <div class="arch-split">
    <div class="arch-lane">
      <div class="arch-arrow">/</div>
      <div class="arch-node arch-frontend">
        <span class="arch-kicker">Frontend</span>
        <strong>React SPA</strong>
        <small>Served as static assets</small>
      </div>
    </div>
    <div class="arch-lane">
      <div class="arch-arrow">/api, /auth</div>
      <div class="arch-node arch-api">
        <span class="arch-kicker">API</span>
        <strong>Rust / Axum</strong>
        <small>Auth, settings, catalog, file browser, proxy authorization</small>
      </div>
    </div>
    <div class="arch-lane">
      <div class="arch-arrow">/{app}/...</div>
      <div class="arch-node arch-apps">
        <span class="arch-kicker">Installed Apps</span>
        <strong>Media Stack</strong>
        <small>Sonarr, Radarr, qBittorrent, Jellyfin, and others</small>
      </div>
    </div>
  </div>

  <div class="arch-row arch-core">
    <div class="arch-node arch-worker">
      <span class="arch-kicker">Worker</span>
      <strong>Rust Worker</strong>
      <small>App operations queue, reconciliation, Helm/Kubernetes lifecycle</small>
    </div>
    <div class="arch-node arch-db">
      <span class="arch-kicker">State</span>
      <strong>PostgreSQL</strong>
      <small>Users, roles, sessions, operations, app state, audit logs</small>
    </div>
    <div class="arch-node arch-storage">
      <span class="arch-kicker">Storage</span>
      <strong>NFS media-data</strong>
      <small>Shared RWX media, downloads, config, cache, transcode paths</small>
    </div>
    <div class="arch-node arch-k8s">
      <span class="arch-kicker">Cluster</span>
      <strong>Kubernetes + Helm</strong>
      <small>Namespaces, Services, PV/PVCs, app releases</small>
    </div>
  </div>
</div>

## Components

- **Gateway**: OpenResty is the public entrypoint. It serves `/healthz`, sends `/api/*` and `/auth/*` to the API, sends `/` to the frontend, and proxies `/{app}/...` directly to installed app Services after asking the API to authorize the request.
- **Frontend**: React/TypeScript SPA served as static files behind the gateway.
- **API**: Rust/Axum service for authentication, user management, settings, catalog reads, file browsing, monitoring endpoints, and gateway proxy authorization.
- **Worker**: Separate Rust process that owns queued app operations and reconciliation. It talks to Kubernetes and Helm without blocking user-facing API requests.
- **Database**: PostgreSQL StatefulSet storing Kubarr internal state: users, roles, sessions, operations, app states, audit logs, settings, and VPN configuration.
- **Storage**: Shared NFS-backed `media-data` claim. App charts mount chart-defined subpaths for config, downloads, media, cache, transcode, and system data.
- **Apps**: Each installed app runs from its Helm chart, usually in its own namespace, with Services reachable by the gateway and lifecycle managed by the worker.

## Request Flow

| Request | Gateway behavior | Backend behavior |
|---------|------------------|------------------|
| `/` | Proxies to frontend Service | None |
| `/api/*` | Proxies to API Service | Handles authenticated control-plane request |
| `/auth/*` | Proxies to API Service | Login, logout, session management |
| `/{app}/...` | Calls `/api/proxy-auth/{app}`, then proxies to the app Service | Authorizes the session and returns the app upstream |
| App install/restart/delete | Proxies API request | API enqueues an operation; worker performs it |

## Design Notes

- App proxy data-plane traffic does not flow through the Rust API anymore; OpenResty handles it after API authorization.
- The API and worker share Rust library code but run as separate images and Deployments.
- Long-running lifecycle work is asynchronous: the API records intent, the worker reconciles actual cluster state.
- The chart keeps the Kubernetes Service named `kubarr-backend` for compatibility, while the image/process is the API.
