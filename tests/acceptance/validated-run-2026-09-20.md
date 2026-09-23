# Validated live acceptance run — 2026-09-20

Result: **PASS**, process exit status **0**, including teardown.

## Environment and artifacts

- Disposable, single-node Kind 0.33.0 / Kubernetes 1.35.8 on Linux amd64.
- Helm 4.3.0; local kubectl 1.36.4.
- Charts: `5b5fc20c7970321a886627ad73f9fa56daf2002c`.
- Real managed NFS, PostgreSQL, Kubarr API/worker/frontend, OpenResty, and Sonarr.
- Local OCI registry; chart discovery from read-only projected ConfigMaps.
- Sonarr charts: `4.1.7-acceptance.1` → `4.1.7-acceptance.2`.
- Same Sonarr application image (`linuxserver/sonarr:4.0.19`) across the upgrade.
- Harness SHA-256: `fba6f4395f17babeeb336f1cb79f6ad13b18c9df08d81483c8e05a334b5ad858`.

Candidate image IDs:

| Image | SHA-256 |
| --- | --- |
| API | `c77d3c246a6eddaccec465c5263937228b5c3b0024c5ad7ec289fffc715179a9` |
| Worker | `07152c74548e70b031020f8894e5ca417ad5acbfb2d262412baee1a95c7c8698` |
| Frontend | `947b792676db9299b93fa714e54f151aaa4704372d5f4f60c5f1a18c844d4e96` |

## Assertions completed

1. Real CLI bootstrap into an empty cluster, including database migration and admin setup.
2. Authentication through the gateway and real OCI catalog synchronization.
3. Queued Sonarr install completed; Helm version and healthy application state matched A.
4. Sonarr status and host-configuration APIs worked through the authenticated gateway.
5. Persisted an instance-name setting and unique NFS sentinel contents.
6. Catalog synchronization exposed B without stale reconciliation overwriting the target.
7. Queued upgrade ran while the worker was deliberately restarted. Captured events
   proved shutdown was received before that operation completed.
8. Helm revision increased, version became B, a new app pod rolled out, and the
   Sonarr image digest stayed unchanged.
9. Application configuration and exact NFS contents survived upgrade and app restart.
10. Uninstall completed; namespace and Helm release disappeared; NFS data remained
    readable through Kubarr's backend mount.
11. Client namespaces were removed before NFS, then the Kind node and registry
    were removed. A follow-up Docker check found no acceptance containers left.

No operations or Helm release records were manually marked successful.

The local non-secret transcript and exit status are retained under the git-ignored
`.acceptance-runs/run-20260920-ninth/`. Credentials and the run's work directory
were removed during cleanup.

## Boundaries

This is local acceptance evidence, not a completed GitHub Actions run. The CI
workflow is configured to require this suite for tagged publication; its chart
commit must be pushed first. The suite does not establish actual Sonarr binary
migration compatibility, Kubarr previous-release upgrades, multi-node recovery,
NetworkPolicy enforcement, VPN behavior, or public DNS/TLS provisioning.
