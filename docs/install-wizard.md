# App installation choices

`GET /api/apps/catalog/{app_name}/install-options` (`apps.view`) reads the locally
pulled OCI chart's `Chart.yaml` and `values.yaml`. It reports the chart version,
main and component NFS mount defaults, required writable targets, reserved
targets, optional private-config claim defaults, and VPN/GPU support. VPN
providers are retrieved separately through `GET /api/vpn/providers` (`vpn.view`);
install options never contain credentials.

The wizard submits a **single** `POST /api/apps/install` (`apps.install`):

```json
{
  "app_name": "qbittorrent",
  "chart_version": "1.2.3",
  "storage": {
    "mounts": [{"name":"downloads","source":"downloads/qbittorrent","target":"/downloads","mode":"read-write"}],
    "config": {"create":true,"existing_claim":"","storage_class_name":"","size":"5Gi"}
  },
  "vpn": {"vpn_provider_id": 12, "port_forwarding": false}
}
```

Omit `storage.mounts`, a workload, or private config to retain the **saved
choice**, if present, otherwise take this chart's defaults. `[]` explicitly
removes that workload's shared NFS mounts where permitted.
`storage.workloads` only accepts components advertised by install options. For
apps with private config, `config.create=true` provisions a separate RWO claim
using the selected StorageClass (empty means cluster default); `create=false`
requires an existing private claim distinct from `media-data`. The wizard does
not set upgrade/migration acknowledgements. Legacy minimal `{"app_name":…}`
installs without a saved configuration continue using chart defaults.

After migration, `app_storage_configs` holds one row per configured app:
`app_name` (primary key), `schema_version=1`, the chart version last reviewed,
validated JSON containing **only** main/component mounts and private-claim
choices, a verified-fresh-private-config flag, and creation/update timestamps.
The first versioned install materializes every supported chart default into a
complete snapshot before saving, even if `storage` was omitted. Later versioned
installs merge omitted fields with that snapshot; their submitted choices,
VPN assignment, app state, queued job and audit entry commit together. If any
validation or queue write fails, none of those changes commit. Existing apps
without a row remain **unconfigured** in this table; their updates continue
Helm reuse rather than inventing choices from a newer chart.

The queued job also carries its own immutable snapshot. GPU updates, VPN
assignment/removal updates and ordinary updates snapshot saved storage at
enqueue and reapply its typed values over `--reuse-values`; retry copies the
same storage choices. The only retry metadata promotion is a separately
persisted fresh-private-config proof from that install. If the chart changes or its required paths no longer accept the
saved settings, the worker fails with an error rather than silently replacing
them. Desired storage persists even if a queued operation fails or is cancelled;
the options endpoint labels it **saved desired**, not necessarily live. Uninstall
retains this row and never deletes its PVC or NFS data. Reinstallation keeps
those choices unless the user explicitly changes them.

The chart's private-config upgrade guard is not a migration switch. Only if
Helm proved the release absent **and Kubernetes proved the expected private PVC
absent** before creating a new private claim, and the first install completed
successfully, will Kubarr remember that proof and confirm modern private config
on later upgrades. Using an existing claim, encountering a failed/pending
release or retained claim, or changing private-claim choices does not confer
that proof; older releases still require the documented manual
stop/copy/verify upgrade, never an automatic migration acknowledgement.

Fresh-install proof is written to the database immediately after Helm reports
success, **before** cleanup of an unused VPN Secret. Cleanup failure is still
reported as an operation error; the proof survives, and a retry with unchanged
storage/chart choices may use it. Failed Helm never writes proof. If the proof
write itself fails, or the worker crashes between Helm success and the database
write, the outcome needs operator review: Kubarr will not infer a fresh private
claim merely from an existing release on retry or assert a legacy data copy.

Omitted `vpn` on legacy installs preserves the existing assignment. Explicit
`"vpn": null` means No VPN; removing an existing assignment requires
`vpn.manage`. Selecting a provider always requires both `apps.install` and
`vpn.manage`. Assignment, queued operation, state and audit outbox entry commit
in one transaction. The queued payload pins the chart version, validated
storage choices and selected provider ID (never credentials). If a sync changes
the locally pulled version or another action changes the assignment before
execution, the worker fails closed rather than installing different settings.
After submission, check the returned operation ID under Apps → Operations.

**Publishing requirement:** Chart sync pulls a versioned OCI package to the
API's chart directory, while the worker deploys that OCI version. The new
declarative storage values must be published under the corresponding version
before the wizard can install them; local edits in `kubarr-charts` alone are
not runtime install options. Chart-specific required/reserved mount paths and
VPN support are maintained in `code/api/src/services/install_options.rs` and
must be reviewed when changing chart templates. The chart's common storage
validation remains the final Helm render-time guard. Version pinning assumes
the OCI registry does not republish different contents under an existing
version; the current Helm path does not pin a registry digest.
