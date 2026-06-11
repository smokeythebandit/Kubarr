# Docker

Kubarr ships two container images, one for the Rust backend and one for the frontend SPA. Both Dockerfiles live in `docker/` and are built from the repository root.

## Backend (`docker/Dockerfile.backend`)

A multi-stage build that produces a statically-linked Rust binary on Alpine Linux.

### Stages

| Stage | Base | Purpose |
|---|---|---|
| `rust-builder` | `rust:alpine` | Installs musl/OpenSSL and compiles Rust dependencies (cached separately from application code) |
| `test` | extends `rust-builder` | Adds clippy and rustfmt, compiles in dev profile for CI lint and test jobs |
| `builder` | extends `rust-builder` | Builds the final `kubarr` binary with the chosen profile |
| `asset-builder` | `alpine:3.23` | Downloads the Helm CLI |
| *(final)* | `alpine:3.23` | Copies the binary, Helm, and CA certs into a minimal runtime image |

### Build args

| Arg | Default | Description |
|---|---|---|
| `PROFILE` | `release` | Cargo build profile. Use `dev-release` for faster builds during development |
| `TARGETARCH` | `amd64` | Architecture for the Helm download |
| `COMMIT_HASH` | `unknown` | Git commit SHA baked into the image |
| `BUILD_TIME` | `unknown` | Build timestamp baked into the image |

### Caching

The build uses BuildKit cache mounts for the Cargo registry, git checkouts, and compiled artifacts. A dummy `main.rs` is compiled first so that dependency builds are cached independently of source changes.

### Usage

```bash
docker build -f docker/Dockerfile.backend -t kubarr-backend:latest \
  --build-arg PROFILE=dev-release .
```

The final image exposes port **8000**.

---

## Frontend (`docker/Dockerfile.frontend`)

A two-stage build that compiles the Vite/React SPA and serves it with BusyBox httpd.

### Stages

| Stage | Base | Purpose |
|---|---|---|
| `builder` | `node:25-alpine` | Installs npm dependencies and holds the source |
| `test` | extends `builder` | Used by CI for lint and build checks |
| `built` | extends `builder` | Runs `npm run build` to produce the static bundle |
| *(final)* | `busybox:1.37` | Serves `/var/www` with BusyBox httpd (~1 MB) |

### Build args

| Arg | Default | Description |
|---|---|---|
| `COMMIT_HASH` | `unknown` | Exposed to Vite as `VITE_COMMIT_HASH` |
| `BUILD_TIME` | `unknown` | Exposed to Vite as `VITE_BUILD_TIME` |

### Usage

```bash
docker build -f docker/Dockerfile.frontend -t kubarr-frontend:latest .
```

The final image exposes port **80**. SPA routing (HTML5 history) is handled by the backend reverse proxy, not by httpd.

---

## Linting

Dockerfiles are linted by [Hadolint](https://github.com/hadolint/hadolint) in CI. The config at `docker/.hadolint.yaml` suppresses rule `DL3018` (pin versions in `apk add`), which is impractical on Alpine where packages are already pinned by the base image tag.

## `.dockerignore`

`docker/.dockerignore` excludes IDE files, test caches, `node_modules`, and other non-build artifacts to keep the build context small — especially important when building over SSH to a remote Docker host.
