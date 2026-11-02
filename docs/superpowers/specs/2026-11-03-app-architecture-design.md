# Kubarr Apps — Architecture Design

**Date:** 2026-11-03
**Status:** approved for planning
**Supersedes:** nothing; complements `2026-08-14-kubarr-production-deploy-design.md`

## Intent

Make adding an app cheap, and make apps connect to each other without anyone
writing another shell script.

Today an app is a Helm chart plus whatever bespoke machinery that chart needed.
The install path is close to app-agnostic, but everything around it accumulated
per-app special cases, and the parts that join apps together were written one
connection at a time.

The week of 2026-11-02 is the evidence. Four separate app defects reached a
live cluster in one day, three of them invisible to every test that existed:

- A values key added to 27 charts broke the first upgrade of every existing
  release, because Helm was asked to reuse the previous release's values and
  therefore never merged the new chart's defaults
  (`nil pointer evaluating interface {}.storage`).
- Setting `strategy: Recreate` on releases published without a strategy made
  their upgrades fail outright under server-side apply
  (`spec.strategy.rollingUpdate: Forbidden`), and writing the field as null did
  not clear it.
- A chart created a claim whose only workload was a suspended CronJob, so
  under a `WaitForFirstConsumer` storage class nothing ever bound it and the
  install waited until it rolled back.
- A chart rendered perfectly and could not be installed at all, because the
  installer validates the chart's own defaults and had no representation for
  "this app has no private config".

Three published credentials were found in the same period: a SABnzbd key, the
platform database password, and a Jackett key that is still in the charts. Each
was the credential of every installation, readable by anyone.

The cross-app picture: six connections, four mechanisms, three credential
models, and two connections users obviously want (Bazarr to the managers,
Prowlarr and Recyclarr to anything) that do not exist because nobody wrote the
next script.

## Goals

1. Adding an app means writing a chart and a declaration. No Rust changes, no
   new shell scripts.
2. Apps connect by declaring what they provide and what they consume. Kubarr
   resolves the rest, including credentials.
3. No credential is ever a literal in a chart, and no credential material
   travels as a Helm value.
4. Charts that Kubarr's authors did not write can be installed.
5. Cross-app connections are reproducible from declarations: a reconcile
   restores every connection Kubarr owns. Settings an app owns alone stay the
   app's own state, on its own volume.
6. Install and upgrade failures are caught before publish, not on a user's
   cluster.

## Non-goals

- Replacing Helm. It stays the renderer, permanently (see Decisions).
- A Kubernetes operator with its own CRD and drift correction. Tempting, and it
  would fix upgrade semantics outright, but it is a rewrite that buys drift
  correction nobody asked for while leaving the wiring problem untouched.
- Multi-tenant isolation. The hosted model is one instance that Benjamin
  operates, with tester accounts on it. Tenancy is Kubarr's own RBAC.
- An external secret manager. Vault or OpenBao becomes defensible when Kubarr
  manages other people's clusters; in this topology it adds an unseal step and
  a bootstrap circularity for no isolation gain.
- Reproducing app-owned configuration. See decision 5.
- Credential rotation. In scope: a single source of truth for managed keys.
  Out of scope: changing one.
- Running one app twice. See decision 8.
- Preserving existing installations. See Constraints.

## Anti-goals

These would make the work a failure even if the end state were right:

- Adding an app becomes *more* work than it is now.
- The media library on `hdd_storage` is damaged or lost.
- Wiring becomes a second place where per-app behaviour hides — a graph that
  needs a code change per edge is the current problem with extra steps.

## Constraints

- **Managed keys: one source of truth, no rotation.** The per-namespace
  Kubernetes Secret becomes the record; the duplicate plaintext row in the
  Kubarr database goes away, which removes the path the settings-endpoint guard
  was patching. Rotation is explicitly out of scope for this design, so the
  Jellyfin and SABnzbd keys remain unchangeable until it gets its own work.
- **Breaking changes are allowed.** Nobody is running Kubarr yet. The media
  library on `hdd_storage` is the only thing that cannot be re-created;
  releases, config claims and chart version numbers can all be thrown away.
- Three-node Talos cluster, Cilium, `local-path` as the default storage class
  (`WaitForFirstConsumer` — this is what makes unconsumed claims hang), an
  external NFS export for shared media, PSA labels enforced per namespace.
- Helm 4.3 with server-side apply. Charts publish to
  `ghcr.io/smokeythebandit/kubarr-charts`; publishing is gated by an install
  gate that installs and upgrades changed charts in kind.
- Foreign charts are installable by **admins only**. Tester accounts cannot
  install anything.
- The four test layers built this week keep working and keep gating:
  rendered-manifest tests, registry consistency, the install-wizard contract
  against a real charts checkout, and the install gate.

## Decisions

These were decided in conversation and are not open:

1. **Helm remains the renderer.** Foreign charts are the reason; a typed
   renderer could not install them.
2. **Foreign charts are installable, by admins only.** First-party charts carry
   a declaration and get everything; a foreign chart installs on inference.
3. **Breaking changes are acceptable**, including renumbering published chart
   versions and restructuring values.
4. **Sequence:** the wiring plane first, then consolidating the app
   declaration, then collapsing chart families, then the catalog index. Version
   renumbering can land at any point.
5. **Reproducibility covers cross-app wiring only.** Kubarr reproduces the
   connections between apps: indexers, download clients, library links and the
   credentials they need. Root folders, quality profiles, Jellyfin libraries
   and users, notification endpoints and similar app-owned settings are not
   modelled; they remain the app's own state. Most of those apps have no
   declarative config API, and modelling their whole settings surface would
   make each app's own UI a lie.
6. **Kubarr restores what it owns.** A connection Kubarr created is marked as
   its own, and the next reconcile restores it when it has been changed or
   deleted inside the app. Entries Kubarr did not create are never touched.
   Today's scripts already work this way through a name prefix
   (`PREFIX="Jackett - "`), which is the precedent to generalise.
7. **Every provider of a capability wires to every consumer**, which is what
   the download-client scripts already do. There is no primary-provider
   choice: Jackett and Prowlarr both installed means both are added as
   indexers.
8. **One instance per app.** `app_name` stays the single identity, with
   namespace, release name and chart reference all derived from it. Two
   instances of one app (`sonarr` and `sonarr-anime`) are out of scope; that is
   an identity change through the whole backend and would have to be planned
   up front rather than discovered.

## Calls made while writing this

Settled by the document's author, not by Benjamin. A builder may revisit any of
them with reasons, but should not have to decide them cold.

1. **The wiring plane never mutates a release.** Several edges have a storage
   prerequisite as well as a credential: a manager and a download client must
   share a path, and the manager needs a remote-to-local path mapping. The
   resolver therefore asserts the prerequisite and reports an edge as
   unsatisfiable when it is missing; it does not add a mount and trigger a Helm
   upgrade from a reconcile pass. Automatic remediation would make
   reconciliation a deployment trigger, which is how failure mode 1 in the
   appendix happened.
2. **Family collapse keeps one published chart per app.** The shapes become a
   shared library the thin per-app charts depend on; `system/kubarr-common`
   already is that vehicle. Five charts selecting an app by values would break
   per-app chart refs, app-name-derived namespaces and release names, the
   install gate's per-chart change detection, and Dependabot's per-chart image
   bumps, and would turn one app's image bump into a version bump for its
   whole family.
3. **Credentials flow in both directions.** Minting is the minority case. Every
   *arr generates its own key into `config.xml`, SABnzbd generates one into
   `sabnzbd.ini`, Jellyfin needed a sidecar writing into its SQLite database,
   and qBittorrent is currently wired with no credential at all. The
   declaration therefore needs a *harvest* direction — read a provider's
   self-generated secret and publish it to the graph — and the writer set must
   include non-API writers, including a row in a SQLite file.
4. **Foreign charts do not participate in the wiring graph** until a
   declaration exists for them. An admin may attach one to an installed foreign
   app, stored with that app's record; nothing infers capabilities from a chart
   that does not declare them. Guessing an app's role from its image would
   produce confident wrong edges.
5. **The appendix is a bar, not a work list.** Two of its rows are already
   fixed (`--reset-then-reuse-values` landed, and the install gate upgrades
   each changed chart from its newest published revision). Four belong to the
   publish pipeline and the operation lifecycle rather than to any plane here,
   and stay outside this design with their own owner. The rest are constraints
   on the three planes.
6. **The install gate must cover values combinations, not just defaults.** Both
   the nil-key and the strategy failures depended on non-default values. For
   first-party families the gate should exercise at least VPN enabled, GPU
   enabled, user-supplied mounts, and the no-private-config shape. Budget is
   the constraint to design against, not correctness.
7. **The default tester role grants every installable app**, with an admin able
   to revoke per app. No exposure class in the declaration schema. Testers
   cannot trigger a reconcile.

## The shape

Three planes, which is the organising idea of this design.

### Install plane

Already close to agnostic: the catalog, install options, storage contract,
chart sync, networking and VPN all read the chart. This week moved
required/reserved paths, VPN support and GPU support out of Rust tables into
chart values. The remaining work is consolidation, not invention: one declared
surface per app, and delete the last Rust-side inference.

Two known app-aware remnants to remove: `managed_key.rs` holds the credential
graph as participant lists, and a migration seeds the viewer role with
`["jellyfin", "jellyseerr"]` so new apps never join it.

### Wiring plane — the actual work

Apps declare capabilities they **provide** (`indexer-api`, `download-client`,
`media-library`, `media-manager-api`) and capabilities they **consume**. Kubarr
resolves the graph against what is installed, mints a credential per edge, and
applies the connection through a **declared mapping** rather than a script:
this key in that INI section, that element in that XML file, this POST to that
endpoint.

The existing managed keys (Jellyfin, SABnzbd) become the first two edges of
that graph instead of hardcoded descriptors. The Jackett literal disappears
with them. Bazarr's auto-connect, which prompted this design, becomes two lines
of a declaration. Prowlarr and Recyclarr stop needing hand-pasted keys —
Recyclarr currently ships suspended for exactly that reason.

Reconciliation runs on every pass. Each connection Kubarr creates carries an
ownership marker, so a pass restores a connection that was edited or deleted in
the app and leaves everything else alone. That is what makes the wiring
reproducible; it does not make an app's config volume disposable, because the
app's own settings are not modelled.

### Admission plane

For foreign charts. Because they are admin-installed, this is a review step
rather than a security boundary: render the chart and show what it asks for —
cluster-scoped resources, RBAC, host mounts, privileged containers — before
anything is applied. The install gate already renders and inspects manifests.

Guardrails are imposed regardless of what the chart ships, because the common
foreign chart is careless rather than malicious: PSA labels, a default-deny
NetworkPolicy, and a namespace quota.

The worker keeps its current privileges. Note for the record what that means:
its ClusterRole carries `bind` and `escalate` on RBAC plus CRDs and admission
webhooks, so a foreign chart installed through it can render anything. That is
acceptable only while installation is admin-only, and it is the reason this
design does not open installation to tester accounts.

### Chart families

Twenty-nine app charts, roughly five shapes: the *arr managers, the download
clients, the library servers, the multi-workload apps, the scheduled jobs. Most
of this week's subtle breakages were one chart differing from its siblings by
accident — the `Recreate` conflict hit precisely the four charts with exporter
Deployments, plus Grafana. One parameterised chart per shape means one template
to fix and one to test.

### Catalog distribution

Discovery currently walks the GitHub Contents API at `main` and gets
rate-limited (observed 403s on the production cluster), and one unpublishable
revision aborts the entire sync — which is what left a worker's chart cache
thirteen charts stale mid-install. Publish a catalog index artifact alongside
the charts: atomic, signable, and immune to one bad entry.

### Versioning

Chart versions get their own semver series, decoupled from `appVersion`. The
current `appVersion+N` scheme is not comparable under semver — `+10` and `+20`
rank equal, and Helm was measured resolving an older revision while the backend
resolved a newer one. Decoupling removes the custom resolution logic that
exists to paper over this.

## What is left to the builder

- The declaration's schema and where it lives (in-chart file versus
  `Chart.yaml` annotations). In-chart is preferred so a foreign chart can adopt
  one later without Kubarr holding an overlay, but the format is open.
- The mapping language for applying a connection, and the set of writers it
  needs: XML element, INI key, YAML path, HTTP request. The existing scripts
  are the requirements list.
- Whether a connection is applied by a Job, an init container, or a reconciling
  sidecar. Today's scripts are sidecar loops; a reconciling applier fits goal 5.
- Credential granularity: one per edge, or one per provider shared by its
  consumers. The existing managed keys are per-provider.
- The index artifact's format and how inference works for a chart with no
  declaration.
- Where the five family boundaries fall, and the escape hatches for the long
  tail (Immich's multi-workload shape, Tdarr's Nodes, Frigate's device access).
- The ownership marker's form, per writer. A name prefix works for an *arr
  notifier; an INI key or an XML element needs something else, and it has to
  survive a user renaming the thing around it.

## Appendix: failure modes to design against

Each of these happened. A design that cannot explain how it avoids them is not
finished.

| Failure | Root cause |
|---|---|
| Upgrade dereferences a nil values key | `--reuse-values` never merges new chart defaults |
| Upgrade rejected on strategy change | server-side apply keeps the stale `rollingUpdate` |
| Install hangs then rolls back | claim with no consumer under `WaitForFirstConsumer` |
| Chart renders but cannot be installed | installer validates chart defaults; no "no config" case |
| Credential readable by anyone | literal in a public chart |
| Fix never reaches users | publish skipped a version it misparsed |
| Chart deploys a nonexistent image | release whose publish job was skipped |
| Catalog goes stale cluster-wide | one chart's pull failure aborts the whole sync |
| Upgrade forbidden on a new kind | worker RBAC lacked `batch/cronjobs` |
| Wedged operation never reaped | heartbeat continued while no progress was possible |
