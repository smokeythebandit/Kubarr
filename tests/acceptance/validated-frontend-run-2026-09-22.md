# Validated frontend acceptance — 2026-09-22

**PASS: 14 real-browser tests; harness exit status 0; scoped cleanup completed.**

Environment: Linux amd64, Kind 0.33.0, Kubernetes 1.35.8, Helm 4.3.0,
Chromium 148 / Playwright 1.60.0. The local browser runner used Node 22.23.1;
CI is configured for the frontend's declared Node 26 requirement.

Charts were pinned to `041dae44aade7d82e5f96342b4db9f306d8b9280`, including the
Sonarr exporter startup fix. The frontend image ID was
`sha256:bb04aa9cc84f7fb6243a0f745523cc00564ea5a2ac0308b56be4706fdd75dcee`.
API and worker used the previously validated authoritative-catalog builds.

## Completed phases

1. Real CLI bootstrap, database migrations, admin setup, and gateway login.
2. Ten settings/VPN browser scenarios passed against the real API and database.
   Mutations used UI controls; persistence was checked after reload and through
   read-only API requests. Settings were restored and test providers removed.
3. Sonarr installed through the UI; its real queued operation succeeded and
   Kubernetes workloads became healthy. The enabled exporter had zero restarts.
4. Catalog refresh and A-to-B chart upgrade completed through the UI. The harness
   independently checked Helm version/revision, replacement pod, and unchanged
   application image digest.
5. Restart and uninstall completed through the UI with real operation/state checks.
6. Application settings and exact NFS sentinel contents survived upgrade/restart;
   data remained readable after app removal.
7. Client namespaces were removed before NFS; the Kind node and registry were
   removed. Follow-up checks found no acceptance containers remaining.

The local transcript and exit status are retained under the git-ignored
`.acceptance-runs/frontend-20260921-third/` directory. Separate HTML reports were
produced for each browser phase. No requests were mocked or operation records
manually marked successful.

## Fixes driven by this lane

- Added the missing permission-gated Restart control to the Apps detail panel.
- Added accessible settings/VPN switches and an app-details region.
- Made pod-health validation wait for old rollout pods to disappear rather than
  failing on a normal transient Completed/terminating pod.
- Fixed Exportarr starting before Sonarr generated its configuration/API key.

VPN CRUD does not prove tunnel connectivity, assignment redeployment, or leak
protection. App upgrade coverage is chart-only; this frontend scenario does not
repeat the API scenario's in-flight worker-drain assertion. This is local evidence,
not a claim that GitHub Actions has run successfully.
