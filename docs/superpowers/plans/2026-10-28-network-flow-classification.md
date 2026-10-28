# Network Flow Classification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Serve internet/LAN/cluster flow rates and measured topology edges from Hubble metrics held in VictoriaMetrics.

**Architecture:** Cilium's Hubble exports flow metrics whose destination label distinguishes `reserved:world` (internet), `cidr:...` (LAN) and a namespace (cluster); a no-op CiliumClusterwideNetworkPolicy mints the LAN CIDR identities that make the first two separable. VictoriaMetrics scrapes the agents through its existing annotation-based pod discovery, and the backend answers with PromQL. No new tables, no new Rust dependencies, no worker changes.

**Tech Stack:** Cilium 1.20.4 / Hubble, VictoriaMetrics, PromQL, Rust (axum, existing VM client), Helm.

**Spec:** `docs/superpowers/specs/2026-10-28-network-flow-classification-design.md`

## Global Constraints

- Metric: `hubble_flows_processed_total{source,destination,protocol,type,subtype,verdict,traffic_direction}`. `traffic_direction` values are lowercase (`egress`/`ingress`); `verdict` is uppercase (`FORWARDED`/`DROPPED`).
- Class totals use `subtype="to-network"` for internet and LAN, `subtype="to-endpoint"` for cluster. Per-app attribution uses `subtype="to-stack"`, the only point where pod identity survives masquerading.
- Never sum across subtypes for one class: internet fires at two trace points and LAN at one, which inflates internet ~1.4x.
- Classification order is identity before address. Apiserver traffic carries both `cidr:192.168.1.20/32` and `reserved:host`; checking `cidr:` first misfiles control-plane traffic as LAN.
- `class` values exactly: `internet`, `lan`, `cluster`, `unclassified`.
- Rust lints: `-D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic`; `cargo fmt --check` must be clean. Library code may not `unwrap`/`expect`; tests may.
- Chart revisions are immutable: bump the version on every chart change.

## File Structure

| File | Responsibility |
|---|---|
| `code/api/src/services/flow_classes.rs` (create) | Destination-label -> class mapping and VM result parsing. Pure, no I/O. |
| `code/api/src/endpoints/networking.rs` (modify) | `GET /flow-classes`; topology edges merged with measured weights. |
| `code/api/src/endpoints/monitoring.rs` (modify) | Make `query_vm_range` and `VICTORIAMETRICS_URL` `pub(crate)`. |
| `kubarr-charts/system/kubarr-cilium-identities/` (create) | Chart shipping the no-op CCNP that mints LAN CIDR identities. |
| `kubarr-charts/monitoring/victoriametrics/values.yaml` (modify) | `kube-system` in the scrape egress allow-list. |
| `kubarr-cli/src/cni.rs` (create) | Enable Hubble metrics on an existing Cilium release. |

---

### Task 1: Map destination labels to classes

**Files:** Create `code/api/src/services/flow_classes.rs`; modify `code/api/src/services/mod.rs`.

**Interfaces produced:**
- `pub enum FlowClass { Internet, Lan, Cluster }`
- `pub fn classify_destination(destination: &str) -> Option<FlowClass>`
- `pub fn class_label(class: Option<FlowClass>) -> &'static str`

- [ ] **Step 1: Write the failing test** — label strings copied verbatim from the cluster.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_namespace_is_cluster_traffic() {
        assert_eq!(classify_destination("kubarr-backend"), Some(FlowClass::Cluster));
        assert_eq!(classify_destination("victoriametrics"), Some(FlowClass::Cluster));
    }

    #[test]
    fn the_apiserver_is_cluster_traffic_despite_its_lan_address() {
        // Captured from the cluster. It carries BOTH a cidr: label and
        // reserved:host; a draft script that tested cidr: first filed 438
        // control-plane flows as LAN.
        let apiserver = "cidr:10.0.0.0/8,cidr:10.244.0.125/32,cidr:192.168.1.20/32,reserved:host,reserved:kube-apiserver";
        assert_eq!(classify_destination(apiserver), Some(FlowClass::Cluster));
    }

    #[test]
    fn nodes_are_cluster_traffic() {
        assert_eq!(classify_destination("reserved:remote-node"), Some(FlowClass::Cluster));
        assert_eq!(classify_destination("reserved:ingress"), Some(FlowClass::Cluster));
    }

    #[test]
    fn a_cidr_identity_without_a_node_identity_is_lan() {
        assert_eq!(
            classify_destination("cidr:192.168.1.0/24,reserved:world"),
            Some(FlowClass::Lan)
        );
    }

    #[test]
    fn plain_world_is_the_internet() {
        assert_eq!(classify_destination("reserved:world"), Some(FlowClass::Internet));
        assert_eq!(classify_destination("reserved:world-ipv4"), Some(FlowClass::Internet));
        assert_eq!(classify_destination("reserved:world-ipv6"), Some(FlowClass::Internet));
    }

    #[test]
    fn unknown_and_empty_destinations_are_never_guessed() {
        assert_eq!(classify_destination(""), None);
        assert_eq!(classify_destination("reserved:unknown"), None);
        assert_eq!(class_label(None), "unclassified");
    }

    #[test]
    fn class_labels_match_the_api_contract() {
        assert_eq!(class_label(Some(FlowClass::Internet)), "internet");
        assert_eq!(class_label(Some(FlowClass::Lan)), "lan");
        assert_eq!(class_label(Some(FlowClass::Cluster)), "cluster");
    }
}
```

- [ ] **Step 2: Run it and watch it fail** — `cd code/api && cargo test --lib services::flow_classes` -> `cannot find function classify_destination`.

- [ ] **Step 3: Implement**

```rust
//! Maps a Hubble destination label to a traffic class.
//!
//! This is the only place in the pipeline that can silently misreport: a wrong
//! mapping simply shows the wrong class forever. It is a pure function over the
//! label string, tested against values captured from a live cluster.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowClass {
    Internet,
    Lan,
    Cluster,
}

/// Identities that mean the destination is part of the cluster itself.
const CLUSTER_RESERVED: [&str; 4] = [
    "reserved:host",
    "reserved:remote-node",
    "reserved:kube-apiserver",
    "reserved:ingress",
];

const WORLD: [&str; 3] = [
    "reserved:world",
    "reserved:world-ipv4",
    "reserved:world-ipv6",
];

/// Classify a Hubble `destination` label value.
///
/// Identity is checked before address, deliberately: the apiserver's label set
/// contains both a `cidr:` entry for its LAN address and `reserved:host`, and
/// testing the address first reports control-plane traffic as LAN.
pub fn classify_destination(destination: &str) -> Option<FlowClass> {
    if destination.is_empty() {
        return None;
    }
    let labels: Vec<&str> = destination.split(',').map(str::trim).collect();

    if labels.iter().any(|label| CLUSTER_RESERVED.contains(label)) {
        return Some(FlowClass::Cluster);
    }
    // destinationContext=namespace|identity yields a bare namespace name for
    // cluster endpoints, which never contains a label prefix.
    if !destination.contains(':') {
        return Some(FlowClass::Cluster);
    }
    if labels.iter().any(|label| label.starts_with("cidr:")) {
        return Some(FlowClass::Lan);
    }
    if labels.iter().any(|label| WORLD.contains(label)) {
        return Some(FlowClass::Internet);
    }
    None
}

/// The exact class name the API promises.
pub fn class_label(class: Option<FlowClass>) -> &'static str {
    match class {
        Some(FlowClass::Internet) => "internet",
        Some(FlowClass::Lan) => "lan",
        Some(FlowClass::Cluster) => "cluster",
        None => "unclassified",
    }
}
```

Register with `pub mod flow_classes;` in `code/api/src/services/mod.rs`.

- [ ] **Step 4: Run it and watch it pass** — 7 tests.
- [ ] **Step 5: fmt + clippy** — `cargo fmt && cargo clippy --no-deps --lib -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic`.
- [ ] **Step 6: Commit** — `feat(network): map Hubble destination labels to traffic classes`.

---

### Task 2: Serve flow classes from VictoriaMetrics

**Files:** modify `code/api/src/endpoints/networking.rs`, `code/api/src/endpoints/monitoring.rs` (make `query_vm_range` and `VICTORIAMETRICS_URL` `pub(crate)`), `code/api/src/endpoints/mod.rs` (OpenAPI registration).

**Interfaces consumed:** `classify_destination`, `class_label` (Task 1), `query_vm_range`.

**Interfaces produced:** `GET /api/networking/flow-classes?window=15m|1h|6h|24h` returning `FlowClassResponse { observed_at, stale, totals, by_app, series }`, where `totals`/`by_app` entries are `ClassRates { flows_per_sec, drops_per_sec }`.

Queries, exactly as validated against the cluster:

```
totals internet+lan: sum by (destination) (rate(hubble_flows_processed_total{traffic_direction="egress",subtype="to-network",verdict="FORWARDED"}[1m]))
totals cluster:      sum by (destination) (rate(hubble_flows_processed_total{traffic_direction="egress",subtype="to-endpoint",verdict="FORWARDED"}[1m]))
per app:             sum by (source, destination) (rate(hubble_flows_processed_total{traffic_direction="egress",subtype="to-stack",verdict="FORWARDED"}[1m]))
drops:               same shape, verdict="DROPPED"
```

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod flow_class_tests {
    use super::*;

    #[test]
    fn windows_map_to_a_span_and_about_sixty_points() {
        assert_eq!(window_span_and_step("15m"), (15 * 60, "15s".to_string()));
        assert_eq!(window_span_and_step("1h"), (60 * 60, "1m".to_string()));
        assert_eq!(window_span_and_step("6h"), (6 * 60 * 60, "6m".to_string()));
        assert_eq!(window_span_and_step("24h"), (24 * 60 * 60, "24m".to_string()));
        assert_eq!(window_span_and_step("garbage"), (15 * 60, "15s".to_string()));
    }

    #[test]
    fn folds_victoriametrics_rows_into_class_totals() {
        let rows = serde_json::json!([
            { "metric": { "destination": "reserved:world" }, "value": [1.0, "4.133"] },
            { "metric": { "destination": "cidr:192.168.1.0/24,reserved:world" }, "value": [1.0, "5.083"] }
        ]);
        let mut totals = ClassTotals::default();
        absorb_totals(&mut totals, rows.as_array().expect("array"), false);

        assert_eq!(totals.internet.flows_per_sec, 4.133);
        assert_eq!(totals.lan.flows_per_sec, 5.083);
        assert_eq!(totals.cluster.flows_per_sec, 0.0);
    }

    #[test]
    fn several_cluster_destinations_add_up_into_one_class() {
        // Every namespace is its own destination label, so cluster totals are a
        // sum rather than a single row.
        let rows = serde_json::json!([
            { "metric": { "destination": "victoriametrics" }, "value": [1.0, "13.625"] },
            { "metric": { "destination": "kubarr-backend" }, "value": [1.0, "10.900"] }
        ]);
        let mut totals = ClassTotals::default();
        absorb_totals(&mut totals, rows.as_array().expect("array"), false);

        assert_eq!(totals.cluster.flows_per_sec, 24.525);
    }

    #[test]
    fn drops_land_in_their_own_field() {
        let rows = serde_json::json!([
            { "metric": { "destination": "reserved:world" }, "value": [1.0, "0.2"] }
        ]);
        let mut totals = ClassTotals::default();
        absorb_totals(&mut totals, rows.as_array().expect("array"), true);

        assert_eq!(totals.internet.drops_per_sec, 0.2);
        assert_eq!(totals.internet.flows_per_sec, 0.0);
    }

    #[test]
    fn per_app_rows_group_by_source_and_class() {
        let rows = serde_json::json!([
            { "metric": { "source": "qbittorrent", "destination": "reserved:world" }, "value": [1.0, "10.99"] },
            { "metric": { "source": "", "destination": "cidr:192.168.1.0/24,reserved:world" }, "value": [1.0, "4.692"] }
        ]);

        let apps = apps_from_vm(rows.as_array().expect("array"), &[]);

        let qb = apps.iter().find(|a| a.app == "qbittorrent").expect("qbittorrent");
        assert_eq!(qb.internet.flows_per_sec, 10.99);
        // An empty source is the host: LAN traffic is kubelet NFS, not an app.
        let host = apps.iter().find(|a| a.app == "host").expect("host");
        assert_eq!(host.lan.flows_per_sec, 4.692);
    }

    #[test]
    fn no_series_means_stale_rather_than_zero() {
        assert!(is_stale(&[]));
        let present = serde_json::json!([{ "metric": {}, "value": [1.0, "1"] }]);
        assert!(!is_stale(present.as_array().expect("array")));
    }

    #[test]
    fn malformed_samples_are_skipped_not_read_as_zero() {
        let rows = serde_json::json!([
            { "metric": { "destination": "reserved:world" }, "value": [1.0, "NaN-ish"] },
            { "metric": { "destination": "cidr:10.0.0.0/8,reserved:world" }, "value": [1.0, "2.5"] }
        ]);
        let mut totals = ClassTotals::default();
        absorb_totals(&mut totals, rows.as_array().expect("array"), false);

        assert_eq!(totals.internet.flows_per_sec, 0.0);
        assert_eq!(totals.lan.flows_per_sec, 2.5);
    }
}
```

- [ ] **Step 2: Run it and watch it fail** — `cargo test --lib endpoints::networking::flow_class_tests`.
- [ ] **Step 3: Implement** the response types (`ClassRates`, `ClassTotals`, `AppClassRates`, `FlowSeriesPoint`, `FlowSeries`, `FlowClassResponse`, `FlowClassQuery`), the helpers (`window_span_and_step`, `sample_value`, `absorb_totals`, `apps_from_vm`, `is_stale`, `series_from_vm`), and the `get_flow_classes` handler issuing the six queries above. Totals sum per class rather than assigning, because each namespace is its own row. Response carries `short_cache(10)`. Register the route and the OpenAPI path.
- [ ] **Step 4: Run it and watch it pass** — 7 tests.
- [ ] **Step 5: fmt + clippy + whole suite** — `cargo fmt && cargo clippy --no-deps --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic && cargo test`.
- [ ] **Step 6: Commit** — `feat(network): serve flow rates by destination class`.

---

### Task 3: Measured topology edges

**Files:** modify `code/api/src/endpoints/networking.rs`.

**Interfaces consumed:** `classify_destination` (Task 1), `query_vm_range`, the existing `discover_service_connections`.

**Interfaces produced:** `NetworkEdge` gains `flows_per_sec: Option<f64>` and `observed: bool`; `pub(crate) fn merge_observed_edges(service_edges: Vec<NetworkEdge>, observed: &[serde_json::Value]) -> Vec<NetworkEdge>`.

The merge keeps Service metadata (names, ports) and takes existence and weight
from Hubble, so an edge with real traffic and no Service still appears, and a
Service nobody uses stops implying traffic.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod topology_merge_tests {
    use super::*;

    fn service_edge(source: &str, target: &str, port: i32) -> NetworkEdge {
        NetworkEdge {
            source: source.to_string(),
            target: target.to_string(),
            edge_type: "service".to_string(),
            port: Some(port),
            protocol: Some("TCP".to_string()),
            label: format!("{port}/TCP"),
            flows_per_sec: None,
            observed: false,
        }
    }

    fn observed_row(source: &str, destination: &str, rate: &str) -> serde_json::Value {
        serde_json::json!({ "metric": { "source": source, "destination": destination }, "value": [1.0, rate] })
    }

    #[test]
    fn a_service_edge_with_traffic_keeps_its_port_and_gains_a_weight() {
        let merged = merge_observed_edges(
            vec![service_edge("kubarr-backend", "victoriametrics", 8428)],
            &[observed_row("kubarr-backend", "victoriametrics", "13.625")],
        );

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].port, Some(8428));
        assert_eq!(merged[0].flows_per_sec, Some(13.625));
        assert!(merged[0].observed);
    }

    #[test]
    fn traffic_with_no_service_still_becomes_an_edge() {
        // qbittorrent to the internet has no Service and can never appear on a
        // Service-derived graph.
        let merged = merge_observed_edges(
            Vec::new(),
            &[observed_row("qbittorrent", "reserved:world", "10.99")],
        );

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, "qbittorrent");
        assert_eq!(merged[0].target, "internet");
        assert!(merged[0].observed);
        assert_eq!(merged[0].port, None);
    }

    #[test]
    fn a_service_nobody_uses_is_kept_but_marked_unobserved() {
        // Retained so the graph still shows the wiring, but the UI can draw it
        // differently rather than implying traffic that is not happening.
        let merged = merge_observed_edges(vec![service_edge("sonarr", "jackett", 9117)], &[]);

        assert_eq!(merged.len(), 1);
        assert!(!merged[0].observed);
        assert_eq!(merged[0].flows_per_sec, None);
    }

    #[test]
    fn external_destinations_collapse_into_class_nodes() {
        let merged = merge_observed_edges(
            Vec::new(),
            &[
                observed_row("", "cidr:192.168.1.0/24,reserved:world", "4.692"),
                observed_row("kubarr-worker", "cidr:10.0.0.0/8,cidr:192.168.1.20/32,reserved:host", "7.4"),
            ],
        );

        let targets: Vec<&str> = merged.iter().map(|e| e.target.as_str()).collect();
        assert!(targets.contains(&"lan"));
        // Node traffic is cluster infrastructure, not LAN.
        assert!(targets.contains(&"node"));
        // An empty source is the host.
        assert!(merged.iter().any(|e| e.source == "host"));
    }
}
```

- [ ] **Step 2: Run it and watch it fail.**
- [ ] **Step 3: Implement** `merge_observed_edges`, keyed on `(source, target)` with the Service edge as the base; external destinations map through `classify_destination` to the node ids `internet`, `lan`, `node`; an empty source becomes `host`. Call it from `get_network_topology` with the `to-endpoint` and `to-stack` query results, and add the `internet`/`lan`/`node` nodes when edges reference them.
- [ ] **Step 4: Run it and watch it pass** — 4 tests.
- [ ] **Step 5: fmt + clippy + whole suite.**
- [ ] **Step 6: Commit** — `feat(network): draw topology edges from observed traffic`.

---

### Task 4: Ship the configuration

**Files:** create `kubarr-charts/system/kubarr-cilium-identities/{Chart.yaml,values.yaml,templates/ccnp.yaml}`; create `kubarr-charts/tests/test_flow_identities.py`; modify `kubarr-charts/monitoring/victoriametrics/values.yaml` and `Chart.yaml`; create `kubarr-cli/src/cni.rs`; modify `kubarr-cli/src/{main.rs,bootstrap.rs,observability.rs,install_events.rs,util.rs,charts.rs}`.

**Interfaces produced:** `pub fn ensure_hubble_metrics(options: &BootstrapOptions) -> Result<(), String>`, `pub fn hubble_upgrade_args(release: &str, namespace: &str) -> Vec<String>`, `pub enum CniState { Cilium, OtherCni, Unknown }`, `pub fn cni_state_from_releases(json: &str) -> CniState`.

- [ ] **Step 1: Write the failing chart test**

```python
"""LAN traffic is only separable from internet traffic if CIDR identities exist.

The policy below selects no pod on purpose: importing it is what makes Cilium
mint the identity, and selecting nothing is what makes that free of consequence.
"""

import unittest

from chart_artifacts import chart_input, render_chart


class CidrIdentityPolicy(unittest.TestCase):
    def test_policy_selects_no_pod(self):
        docs = render_chart(chart_input("system/kubarr-cilium-identities"))
        policies = [d for d in docs if d and d.get("kind") == "CiliumClusterwideNetworkPolicy"]
        self.assertEqual(len(policies), 1)
        selector = policies[0]["spec"]["endpointSelector"]["matchLabels"]
        self.assertEqual(selector, {"kubarr.io/cidr-identity-only": "true"})

    def test_policy_lists_the_lan_ranges(self):
        docs = render_chart(chart_input("system/kubarr-cilium-identities"))
        policy = next(d for d in docs if d and d.get("kind") == "CiliumClusterwideNetworkPolicy")
        cidrs = {entry["cidr"] for rule in policy["spec"]["egress"] for entry in rule["toCIDRSet"]}
        self.assertEqual(cidrs, {"10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"})


class VictoriaMetricsReachesTheAgents(unittest.TestCase):
    def test_kube_system_is_scrapeable(self):
        docs = render_chart(chart_input("monitoring/victoriametrics"))
        allowed = set()
        for policy in [d for d in docs if d and d.get("kind") == "NetworkPolicy"]:
            for rule in policy["spec"].get("egress", []):
                for target in rule.get("to", []):
                    name = target.get("namespaceSelector", {}).get("matchLabels", {}).get(
                        "kubernetes.io/metadata.name"
                    )
                    if name:
                        allowed.add(name)
        self.assertIn(
            "kube-system",
            allowed,
            "Hubble metrics live on the Cilium agents; without this the panels are silently empty",
        )


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run it and watch it fail** — package the charts, then
  `CHART_PACKAGES_DIR=$PKG python3 -B -m unittest tests.test_flow_identities -v`.
- [ ] **Step 3: Implement the chart.** `templates/ccnp.yaml` guarded on
  `.Capabilities.APIVersions.Has "cilium.io/v2"`, `endpointSelector.matchLabels`
  of `kubarr.io/cidr-identity-only: "true"` (no pod carries it), and
  `egress[].toCIDRSet` from `.Values.lanCidrs`. Add `kube-system` to
  `monitoring/victoriametrics/values.yaml` scrape list. Bump both chart versions.
- [ ] **Step 4: Run the whole chart suite** — all tests pass.
- [ ] **Step 5: Write the failing CLI test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_a_cilium_release() {
        let releases = r#"[{"name":"cilium","namespace":"kube-system","chart":"cilium-1.20.4","status":"deployed"}]"#;
        assert_eq!(cni_state_from_releases(releases), CniState::Cilium);
    }

    #[test]
    fn a_cluster_without_cilium_is_left_alone() {
        assert_eq!(cni_state_from_releases("[]"), CniState::OtherCni);
    }

    #[test]
    fn unreadable_output_is_not_mistaken_for_an_absent_cni() {
        assert_eq!(cni_state_from_releases("not json"), CniState::Unknown);
    }

    #[test]
    fn the_upgrade_keeps_existing_values_and_pins_no_version() {
        let args = hubble_upgrade_args("cilium", "kube-system");
        assert!(args.contains(&"--reuse-values".to_string()));
        assert!(args.iter().any(|a| a.contains("hubble.metrics.enabled")));
        assert!(args.iter().any(|a| a.contains("destinationContext=namespace|identity")));
        assert!(args.iter().any(|a| a.contains("prometheus.io/scrape")));
        // Never re-pin the chart version of a running CNI.
        assert!(!args.iter().any(|a| a == "--version"));
    }
}
```

- [ ] **Step 6: Implement the CLI step.** `ensure_hubble_metrics` reads
  `helm list -A -o json`, and on a Cilium release runs the upgrade with
  `--reuse-values`, the two metric specs, and the agent scrape annotations.
  Anything else warns and continues. Call it from `bootstrap.rs` after
  `install_victoriametrics`, add `kube-system` to the CLI's own
  `networkPolicy.scrapeNamespaces` list, add `Operation::Hubble`, and update the
  chart version pins.
- [ ] **Step 7: Verify** — `cd kubarr-cli && cargo fmt && cargo test && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 8: Commit** both repos.

---

## Self-Review

**Spec coverage.** Classification mapping -> Task 1. API, queries, windows,
`stale` -> Task 2. Topology merge -> Task 3. Both chart changes, the CIDR
identity policy, and the CLI step -> Task 4. Sub-project B stays out of scope.

**What this plan dropped versus its first revision.** Vendored Hubble protos,
`tonic`/`prost`, the gRPC stream consumer, the worker metrics listener and the
worker chart changes — four tasks and roughly 600 lines — because Hubble's own
metrics plus a no-op CIDR policy produce the same data. Measured on the cluster
before rewriting.

**Placeholder scan.** No TBDs. Every test step carries real assertions over
label strings captured from the live cluster; every implementation step names
the functions and the queries.

**Type consistency.** `FlowClass`, `classify_destination`, `class_label`,
`ClassRates`, `ClassTotals`, `AppClassRates`, `FlowSeries`, `FlowClassResponse`,
`absorb_totals`, `apps_from_vm`, `is_stale`, `merge_observed_edges`, `CniState`
and `hubble_upgrade_args` are each defined once and used with one signature.

**Known risk.** `hubble_flows_processed_total` counts datapath *events*, not
connections. Figures are proportional, and the UI must say "flow events". Noted
in the spec and carried into sub-project B.
