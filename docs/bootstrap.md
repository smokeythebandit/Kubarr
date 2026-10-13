# Web bootstrap

The CLI and browser installer live in the standalone [kubarr-cli repository](https://github.com/smokeythebandit/kubarr-cli), with their own versioning, tests, and Linux releases.

```sh
curl -fsSL https://kubarr.com/install.sh | sudo bash
```

The script installs the latest CLI and starts the browser wizard. Open the printed invitation URL and keep the terminal running until setup finishes. To launch the wizard again, run `kubarr bootstrap`; use `kubarr bootstrap --web-port 8080` to choose a fixed port.

See the [CLI bootstrap guide](https://github.com/smokeythebandit/kubarr-cli/blob/main/docs/bootstrap.md) for Kubernetes prerequisites, SSH access, storage checks, dashboard access, and diagnostics.

## CLI development and releases

Build and test in a checkout of `kubarr-cli`:

```sh
cargo build --locked
cargo test --locked --bin kubarr
node --test src/bootstrap_web.test.mjs
```

The CLI's version is defined by its own `Cargo.toml`. Its CI publishes Linux x86_64 and ARM64 binaries and checksums to [kubarr-cli releases](https://github.com/smokeythebandit/kubarr-cli/releases). The website downloads from those releases.

Kubarr's E2E and acceptance workflows use the pinned CLI revision in `.github/actions/setup-cli/action.yml`. That action builds the standalone source and exports `KUBARR_CLI` for the browser bootstrap driver. Update that pin when validating a new CLI version with the application.
