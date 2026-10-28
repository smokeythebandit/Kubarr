# Network Flow Classification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure how much of the cluster's network activity goes to the internet, to the LAN, and between pods, and serve it from an API.

**Architecture:** The worker opens a gRPC stream to Hubble, classifies every egress flow by destination into internet/lan/cluster, and keeps in-memory counters exposed at `/metrics`. VictoriaMetrics scrapes the worker through its existing annotation-based pod discovery, and the backend answers `/api/networking/flow-classes` with VM range queries. No new database tables.

**Tech Stack:** Rust (tonic + prost for gRPC, axum for the metrics listener), Cilium 1.20.4 / Hubble, VictoriaMetrics, Helm.

**Spec:** `docs/superpowers/specs/2026-10-28-network-flow-classification-design.md`

## Global Constraints

- Cilium pinned at chart version `1.20.4`; Hubble protos vendored from `cilium/cilium` tag `v1.20.4`.
- Metric names exactly: `kubarr_network_flows_total{app,class,verdict}` and `kubarr_network_flow_watch_connected`.
- `class` label values exactly: `internet`, `lan`, `cluster`, `unclassified`. `verdict` label values exactly: `forwarded`, `dropped`.
- Worker metrics listener on port `9100`, path `/metrics`.
- LAN defaults to RFC1918 (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`) minus pod and service CIDRs, overridable by `KUBARR_LAN_CIDRS` (comma-separated).
- Hubble RPC is `observer.Observer/GetFlows` with `follow=true` — **not** `Observe`.
- Count only flows where `traffic_direction == EGRESS` and `Type == L3_L4`. Anything else double-counts.
- Rust lints in CI: `-D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic`. Tests may use `expect`; library code may not.
- `cargo fmt` must be clean; CI runs `cargo fmt --check`.
- No new crates beyond `tonic`, `prost`, and `tonic-build`. CIDR matching is hand-rolled rather than pulling in `ipnet`, matching the spec's decision to hand-render metrics instead of adding a Prometheus client.

## File Structure

| File | Responsibility |
|---|---|
| `code/api/src/services/flow_classifier.rs` (create) | `Cidr` type and the pure `classify` decision. No I/O. |
| `code/api/src/services/flow_metrics.rs` (create) | Counter registry and Prometheus text rendering. |
| `code/api/src/services/flow_watch.rs` (create) | Per-flow handling (testable) plus the thin gRPC transport loop. |
| `code/api/proto/flow/flow.proto`, `proto/observer/observer.proto`, `proto/relay/relay.proto` (create) | Vendored Hubble protos. |
| `code/api/build.rs` (create) | `tonic-build` compilation of the vendored protos. |
| `code/api/src/endpoints/metrics_server.rs` (create) | Worker's axum `/metrics` listener. |
| `code/api/src/endpoints/networking.rs` (modify) | `GET /flow-classes` handler. |
| `code/api/src/endpoints/monitoring.rs` (modify) | Share `query_vm_range` and `VICTORIAMETRICS_URL`. |
| `code/api/src/application/runtime.rs` (modify) | Spawn `flow_watch` and the metrics listener in `run_worker`. |
| `kubarr-charts/system/kubarr-worker/values.yaml` (modify) | Scrape annotations and container port. |
| `kubarr-charts/monitoring/victoriametrics/values.yaml` (modify) | Add worker namespace to scrape egress allow-list. |
| `kubarr-cli/src/cni.rs` (create) | Ensure Hubble on an existing Cilium install; report other CNIs. |

**Scope note on Task 8.** The spec says the CLI owns the Cilium install. Investigation found the CLI's `SingleNode` path installs k3s with its default flannel CNI (`curl -sfL https://get.k3s.io | sh -s -`), and `ClusterMode::Existing` covers bring-your-own clusters like the Talos production cluster. Installing a CNI into someone's existing cluster is destructive, and switching greenfield k3s off flannel changes cluster bootstrap and the acceptance harness. Task 8 therefore implements the safe half: ensure Hubble on an already-present Cilium release, and report clearly when the CNI is something else. **The greenfield k3s-with-Cilium switch is deliberately not in this plan** and needs its own decision and spec.

---

### Task 1: Classify a flow by destination

**Files:**
- Create: `code/api/src/services/flow_classifier.rs`
- Modify: `code/api/src/services/mod.rs`

**Interfaces:**
- Produces: `pub enum FlowClass { Internet, Lan, Cluster }`, `pub struct Cidr`, `pub fn Cidr::parse(&str) -> Option<Cidr>`, `pub fn Cidr::contains(&self, ip: IpAddr) -> bool`, `pub fn default_lan_cidrs() -> Vec<Cidr>`, `pub fn classify(destination_labels: &[String], destination_ip: Option<IpAddr>, lan: &[Cidr]) -> Option<FlowClass>`, `pub fn class_label(class: Option<FlowClass>) -> &'static str`.

- [ ] **Step 1: Write the failing test**

Append to `code/api/src/services/flow_classifier.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn ip(text: &str) -> Option<IpAddr> {
        text.parse().ok()
    }

    fn lan() -> Vec<Cidr> {
        default_lan_cidrs()
    }

    #[test]
    fn pod_destinations_are_cluster_traffic() {
        let labels = vec!["k8s:app.kubernetes.io/name=sonarr".to_string()];
        assert_eq!(
            classify(&labels, ip("10.244.0.15"), &lan()),
            Some(FlowClass::Cluster)
        );
    }

    #[test]
    fn node_and_apiserver_destinations_are_cluster_traffic() {
        // Nodes sit on the LAN, but this is kubelet and apiserver chatter:
        // infrastructure, not the LAN activity the split is meant to surface.
        for reserved in [
            "reserved:host",
            "reserved:remote-node",
            "reserved:kube-apiserver",
        ] {
            let labels = vec![reserved.to_string()];
            assert_eq!(
                classify(&labels, ip("192.168.1.20"), &lan()),
                Some(FlowClass::Cluster),
                "{reserved} must classify as cluster"
            );
        }
    }

    #[test]
    fn other_private_addresses_are_lan() {
        // The NFS server: no node identity, private address.
        let labels = vec!["reserved:world".to_string()];
        assert_eq!(classify(&labels, ip("192.168.1.6"), &lan()), Some(FlowClass::Lan));
        assert_eq!(classify(&labels, ip("10.0.5.4"), &lan()), Some(FlowClass::Lan));
        assert_eq!(classify(&labels, ip("172.16.9.9"), &lan()), Some(FlowClass::Lan));
    }

    #[test]
    fn public_addresses_are_internet() {
        let labels = vec!["reserved:world".to_string()];
        assert_eq!(classify(&labels, ip("1.1.1.1"), &lan()), Some(FlowClass::Internet));
        assert_eq!(
            classify(&labels, ip("2606:4700:4700::1111"), &lan()),
            Some(FlowClass::Internet)
        );
    }

    #[test]
    fn identity_wins_over_an_overlapping_lan_range() {
        // A pod CIDR inside RFC1918 must not be misread as LAN traffic.
        let labels = vec!["k8s:io.kubernetes.pod.namespace=radarr".to_string()];
        let overlapping = vec![Cidr::parse("10.0.0.0/8").expect("cidr")];
        assert_eq!(
            classify(&labels, ip("10.244.0.15"), &overlapping),
            Some(FlowClass::Cluster)
        );
    }

    #[test]
    fn loopback_and_link_local_belong_to_no_class() {
        let labels: Vec<String> = Vec::new();
        assert_eq!(classify(&labels, ip("127.0.0.1"), &lan()), None);
        assert_eq!(classify(&labels, ip("169.254.1.1"), &lan()), None);
        assert_eq!(classify(&labels, ip("::1"), &lan()), None);
    }

    #[test]
    fn a_flow_without_a_destination_is_never_guessed() {
        let labels: Vec<String> = Vec::new();
        assert_eq!(classify(&labels, None, &lan()), None);
        assert_eq!(class_label(None), "unclassified");
    }

    #[test]
    fn cidr_parsing_rejects_nonsense_and_matches_boundaries() {
        assert!(Cidr::parse("not-a-cidr").is_none());
        assert!(Cidr::parse("192.168.1.0/33").is_none());
        let net = Cidr::parse("192.168.1.0/24").expect("cidr");
        assert!(net.contains("192.168.1.0".parse().expect("ip")));
        assert!(net.contains("192.168.1.255".parse().expect("ip")));
        assert!(!net.contains("192.168.2.0".parse().expect("ip")));
    }

    #[test]
    fn class_labels_are_the_names_the_metrics_contract_promises() {
        assert_eq!(class_label(Some(FlowClass::Internet)), "internet");
        assert_eq!(class_label(Some(FlowClass::Lan)), "lan");
        assert_eq!(class_label(Some(FlowClass::Cluster)), "cluster");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd code/api && cargo test --lib services::flow_classifier`
Expected: FAIL — `file not found for module` or `cannot find function classify`.

- [ ] **Step 3: Write minimal implementation**

Prepend to `code/api/src/services/flow_classifier.rs`:

```rust
//! Classifies a flow by where it was going.
//!
//! This is the one place in the pipeline where a wrong answer is silent: a
//! misclassified destination simply reports the wrong class forever. It is
//! therefore a pure function of the destination's identity labels and address,
//! with no I/O, and tested exhaustively.
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowClass {
    Internet,
    Lan,
    Cluster,
}

/// An IPv4 or IPv6 network, matched by prefix.
///
/// Hand-rolled rather than pulling in a CIDR crate: containment is a handful of
/// bit operations and keeps the dependency list flat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    network: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn parse(text: &str) -> Option<Self> {
        let (address, prefix) = text.split_once('/')?;
        let network: IpAddr = address.trim().parse().ok()?;
        let prefix: u8 = prefix.trim().parse().ok()?;
        let max = if network.is_ipv4() { 32 } else { 128 };
        if prefix > max {
            return None;
        }
        Some(Self { network, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.network, ip) {
            (IpAddr::V4(network), IpAddr::V4(candidate)) => {
                prefix_matches(&network.octets(), &candidate.octets(), self.prefix)
            }
            (IpAddr::V6(network), IpAddr::V6(candidate)) => {
                prefix_matches(&network.octets(), &candidate.octets(), self.prefix)
            }
            // Mixed families never match.
            _ => false,
        }
    }
}

fn prefix_matches(network: &[u8], candidate: &[u8], prefix: u8) -> bool {
    let whole_bytes = (prefix / 8) as usize;
    if network[..whole_bytes] != candidate[..whole_bytes] {
        return false;
    }
    let remaining = prefix % 8;
    if remaining == 0 {
        return true;
    }
    let mask = 0xffu8 << (8 - remaining);
    (network[whole_bytes] & mask) == (candidate[whole_bytes] & mask)
}

/// RFC1918 ranges. Pod and service CIDRs are excluded by the identity check in
/// `classify`, which runs first, rather than by subtracting ranges here.
pub fn default_lan_cidrs() -> Vec<Cidr> {
    ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"]
        .iter()
        .filter_map(|text| Cidr::parse(text))
        .collect()
}

/// Labels Cilium attaches to destinations that are part of the cluster itself.
const CLUSTER_RESERVED: [&str; 4] = [
    "reserved:host",
    "reserved:remote-node",
    "reserved:kube-apiserver",
    "reserved:ingress",
];

/// Classify by destination. `destination_labels` are Cilium's identity labels
/// in `foo=bar` form; `destination_ip` is the raw address.
///
/// Returns `None` when the destination cannot be determined. The caller records
/// that as `unclassified` rather than assigning a class by guesswork.
pub fn classify(
    destination_labels: &[String],
    destination_ip: Option<IpAddr>,
    lan: &[Cidr],
) -> Option<FlowClass> {
    // Identity before address, deliberately: a pod CIDR that overlaps an
    // RFC1918 range would otherwise report in-cluster traffic as LAN.
    if destination_labels.iter().any(is_cluster_label) {
        return Some(FlowClass::Cluster);
    }

    let ip = destination_ip?;
    if ip.is_loopback() || is_link_local(ip) || ip.is_unspecified() {
        return None;
    }

    if lan.iter().any(|net| net.contains(ip)) {
        return Some(FlowClass::Lan);
    }

    Some(FlowClass::Internet)
}

fn is_cluster_label(label: &String) -> bool {
    if CLUSTER_RESERVED.contains(&label.as_str()) {
        return true;
    }
    // Any Kubernetes-sourced label means the destination is a cluster endpoint.
    label.starts_with("k8s:")
}

fn is_link_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// The exact `class` label value the metrics contract promises.
pub fn class_label(class: Option<FlowClass>) -> &'static str {
    match class {
        Some(FlowClass::Internet) => "internet",
        Some(FlowClass::Lan) => "lan",
        Some(FlowClass::Cluster) => "cluster",
        None => "unclassified",
    }
}
```

Add to `code/api/src/services/mod.rs`, in alphabetical position among the existing `pub mod` lines:

```rust
pub mod flow_classifier;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd code/api && cargo test --lib services::flow_classifier`
Expected: PASS — 9 tests.

- [ ] **Step 5: Check formatting and lints**

Run: `cd code/api && cargo fmt && cargo clippy --no-deps --lib -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic`
Expected: no output.

- [ ] **Step 6: Commit**

```bash
git add code/api/src/services/flow_classifier.rs code/api/src/services/mod.rs
git commit -m "feat(network): classify flows by destination class

Identity is checked before address so a pod CIDR overlapping an RFC1918 range
cannot report in-cluster traffic as LAN. Node and apiserver destinations count
as cluster traffic even though nodes sit on the LAN, because that is kubelet
and overlay chatter rather than LAN activity worth acting on. A destination
that cannot be determined is recorded as unclassified rather than guessed."
```

---

### Task 2: Count classified flows and render them as metrics

**Files:**
- Create: `code/api/src/services/flow_metrics.rs`
- Modify: `code/api/src/services/mod.rs`

**Interfaces:**
- Consumes: `FlowClass`, `class_label` from Task 1.
- Produces: `pub enum Verdict { Forwarded, Dropped }`, `pub struct FlowCounters`, `pub fn FlowCounters::new() -> Self`, `pub fn record(&self, app: &str, class: Option<FlowClass>, verdict: Verdict)`, `pub fn set_connected(&self, connected: bool)`, `pub fn render(&self) -> String`.

- [ ] **Step 1: Write the failing test**

Append to `code/api/src/services/flow_metrics.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::flow_classifier::FlowClass;

    #[test]
    fn renders_counters_in_prometheus_text_format() {
        let counters = FlowCounters::new();
        counters.record("sonarr", Some(FlowClass::Internet), Verdict::Forwarded);
        counters.record("sonarr", Some(FlowClass::Internet), Verdict::Forwarded);
        counters.record("sonarr", Some(FlowClass::Cluster), Verdict::Dropped);
        counters.set_connected(true);

        let rendered = counters.render();

        assert!(rendered.contains("# TYPE kubarr_network_flows_total counter"));
        assert!(rendered.contains(
            "kubarr_network_flows_total{app=\"sonarr\",class=\"internet\",verdict=\"forwarded\"} 2"
        ));
        assert!(rendered.contains(
            "kubarr_network_flows_total{app=\"sonarr\",class=\"cluster\",verdict=\"dropped\"} 1"
        ));
        assert!(rendered.contains("# TYPE kubarr_network_flow_watch_connected gauge"));
        assert!(rendered.contains("kubarr_network_flow_watch_connected 1"));
    }

    #[test]
    fn reports_disconnection_so_readers_can_distrust_the_numbers() {
        let counters = FlowCounters::new();
        counters.set_connected(false);

        assert!(counters.render().contains("kubarr_network_flow_watch_connected 0"));
    }

    #[test]
    fn an_undeterminable_destination_is_labelled_unclassified() {
        let counters = FlowCounters::new();
        counters.record("radarr", None, Verdict::Forwarded);

        assert!(counters.render().contains(
            "kubarr_network_flows_total{app=\"radarr\",class=\"unclassified\",verdict=\"forwarded\"} 1"
        ));
    }

    #[test]
    fn label_values_are_escaped_so_output_cannot_be_broken() {
        let counters = FlowCounters::new();
        counters.record("odd\"name\\here", Some(FlowClass::Lan), Verdict::Forwarded);

        let rendered = counters.render();

        assert!(rendered.contains(r#"app="odd\"name\\here""#));
    }

    #[test]
    fn output_is_stable_between_renders() {
        // Scrapers diff successive responses; label ordering must not wobble.
        let counters = FlowCounters::new();
        counters.record("sonarr", Some(FlowClass::Lan), Verdict::Forwarded);
        counters.record("radarr", Some(FlowClass::Internet), Verdict::Forwarded);

        assert_eq!(counters.render(), counters.render());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd code/api && cargo test --lib services::flow_metrics`
Expected: FAIL — `cannot find struct FlowCounters`.

- [ ] **Step 3: Write minimal implementation**

Prepend to `code/api/src/services/flow_metrics.rs`:

```rust
//! Counters for classified flows, rendered as Prometheus text.
//!
//! Hand-rendered rather than via a Prometheus client crate: this is two
//! metrics, the dependency list stays flat, and the exact scrape output
//! becomes assertable in a unit test.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::services::flow_classifier::{class_label, FlowClass};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Forwarded,
    Dropped,
}

impl Verdict {
    fn label(self) -> &'static str {
        match self {
            Verdict::Forwarded => "forwarded",
            Verdict::Dropped => "dropped",
        }
    }
}

/// Flow counts keyed by app, class and verdict.
///
/// A BTreeMap rather than a HashMap so successive scrapes render identically;
/// scrapers diff responses and wobbling label order is noise.
#[derive(Debug)]
pub struct FlowCounters {
    counts: Mutex<BTreeMap<(String, &'static str, &'static str), u64>>,
    connected: AtomicBool,
}

impl Default for FlowCounters {
    fn default() -> Self {
        Self::new()
    }
}

impl FlowCounters {
    pub fn new() -> Self {
        Self {
            counts: Mutex::new(BTreeMap::new()),
            connected: AtomicBool::new(false),
        }
    }

    pub fn record(&self, app: &str, class: Option<FlowClass>, verdict: Verdict) {
        let key = (app.to_string(), class_label(class), verdict.label());
        // A poisoned lock must not take the worker down: flow accounting is a
        // diagnostic, and the stream keeps running.
        if let Ok(mut counts) = self.counts.lock() {
            *counts.entry(key).or_insert(0) += 1;
        }
    }

    pub fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Relaxed);
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# TYPE kubarr_network_flows_total counter\n");
        if let Ok(counts) = self.counts.lock() {
            for ((app, class, verdict), count) in counts.iter() {
                out.push_str(&format!(
                    "kubarr_network_flows_total{{app=\"{}\",class=\"{}\",verdict=\"{}\"}} {}\n",
                    escape(app),
                    class,
                    verdict,
                    count
                ));
            }
        }
        out.push_str("# TYPE kubarr_network_flow_watch_connected gauge\n");
        out.push_str(&format!(
            "kubarr_network_flow_watch_connected {}\n",
            u8::from(self.connected.load(Ordering::Relaxed))
        ));
        out
    }
}

/// Escape a label value per the Prometheus text format.
fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
```

Add to `code/api/src/services/mod.rs`:

```rust
pub mod flow_metrics;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd code/api && cargo test --lib services::flow_metrics`
Expected: PASS — 5 tests.

- [ ] **Step 5: Check formatting and lints**

Run: `cd code/api && cargo fmt && cargo clippy --no-deps --lib -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic`
Expected: no output.

- [ ] **Step 6: Commit**

```bash
git add code/api/src/services/flow_metrics.rs code/api/src/services/mod.rs
git commit -m "feat(network): count classified flows and render them for scraping

Counts are held in a BTreeMap so successive scrapes render byte-identically;
scrapers diff responses and wobbling label order is noise. Label values are
escaped, and a poisoned lock drops a sample rather than taking the worker down,
because flow accounting is a diagnostic and the stream must keep running."
```

---

### Task 3: Vendor the Hubble protos and generate the client

**Files:**
- Create: `code/api/proto/flow/flow.proto`, `code/api/proto/observer/observer.proto`, `code/api/proto/relay/relay.proto`
- Create: `code/api/build.rs`
- Modify: `code/api/Cargo.toml`
- Create: `code/api/src/services/hubble.rs`
- Modify: `code/api/src/services/mod.rs`

**Interfaces:**
- Produces: module `crate::services::hubble` re-exporting the generated types, specifically `hubble::flow::Flow`, `hubble::flow::Verdict`, `hubble::flow::TrafficDirection`, `hubble::flow::FlowType`, `hubble::flow::Endpoint`, `hubble::flow::Ip`, `hubble::observer::observer_client::ObserverClient`, `hubble::observer::GetFlowsRequest`, `hubble::observer::GetFlowsResponse`.

- [ ] **Step 1: Vendor the protos**

The protos are fetched from the pinned Cilium tag. `observer.proto` imports `flow/flow.proto` and `relay/relay.proto`, so all three are required.

```bash
cd code/api && mkdir -p proto/flow proto/observer proto/relay
curl -fsSL https://raw.githubusercontent.com/cilium/cilium/v1.20.4/api/v1/flow/flow.proto     -o proto/flow/flow.proto
curl -fsSL https://raw.githubusercontent.com/cilium/cilium/v1.20.4/api/v1/observer/observer.proto -o proto/observer/observer.proto
curl -fsSL https://raw.githubusercontent.com/cilium/cilium/v1.20.4/api/v1/relay/relay.proto   -o proto/relay/relay.proto
grep -c . proto/flow/flow.proto proto/observer/observer.proto proto/relay/relay.proto
```

Expected: three non-empty files. `flow.proto` is ~1033 lines and `observer.proto` ~302.

- [ ] **Step 2: Add the dependencies and build script**

In `code/api/Cargo.toml`, add to `[dependencies]`:

```toml
tonic = "0.14"
prost = "0.14"
```

Add a new section after `[dependencies]` (or extend it if it already exists):

```toml
[build-dependencies]
tonic-build = "0.14"
```

Create `code/api/build.rs`:

```rust
//! Compiles the vendored Hubble protos.
//!
//! Vendored rather than fetched at build time so an offline or air-gapped
//! build behaves identically, and so the generated API cannot shift under us
//! when upstream moves.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto");
    tonic_build::configure()
        .build_server(false)
        .compile_protos(
            &["proto/observer/observer.proto"],
            &["proto"],
        )?;
    Ok(())
}
```

- [ ] **Step 3: Write the failing test**

Create `code/api/src/services/hubble.rs`:

```rust
//! Generated Hubble gRPC types.
//!
//! The protos live under `proto/` and are compiled by `build.rs`. Only the
//! client is generated; nothing here serves Hubble.
pub mod flow {
    tonic::include_proto!("flow");
}

pub mod relay {
    tonic::include_proto!("relay");
}

pub mod observer {
    tonic::include_proto!("observer");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generated_contract_matches_what_the_classifier_expects() {
        // Pins the field and enum names the flow handler depends on. If a
        // future proto bump renames any of them, this fails here rather than
        // silently misreading flows.
        let flow = flow::Flow {
            verdict: flow::Verdict::Forwarded as i32,
            traffic_direction: flow::TrafficDirection::Egress as i32,
            r#type: flow::FlowType::L3L4 as i32,
            ip: Some(flow::Ip {
                source: "10.244.0.15".to_string(),
                destination: "1.1.1.1".to_string(),
                ..Default::default()
            }),
            source: Some(flow::Endpoint {
                namespace: "sonarr".to_string(),
                ..Default::default()
            }),
            destination: Some(flow::Endpoint {
                labels: vec!["reserved:world".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(flow.verdict, flow::Verdict::Forwarded as i32);
        assert_eq!(
            flow.destination.as_ref().map(|d| d.labels.len()),
            Some(1)
        );
        assert_eq!(
            flow.ip.as_ref().map(|ip| ip.destination.as_str()),
            Some("1.1.1.1")
        );
    }

    #[test]
    fn the_streaming_request_can_follow() {
        let request = observer::GetFlowsRequest {
            follow: true,
            ..Default::default()
        };
        assert!(request.follow);
    }
}
```

Add to `code/api/src/services/mod.rs`:

```rust
pub mod hubble;
```

- [ ] **Step 4: Run test to verify it fails, then passes**

Run: `cd code/api && cargo test --lib services::hubble`

First run requires `protoc`. If the build fails with `protoc` not found, install it (`sudo dnf install -y protobuf-compiler`) and re-run; note it as a build prerequisite in the commit message.

Expected after a successful build: PASS — 2 tests.

- [ ] **Step 5: Check formatting and lints**

Run: `cd code/api && cargo fmt && cargo clippy --no-deps --lib -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic`
Expected: no output. Generated code is excluded from clippy by `--no-deps` because it lives in `OUT_DIR`.

- [ ] **Step 6: Commit**

```bash
git add code/api/proto code/api/build.rs code/api/Cargo.toml code/api/Cargo.lock code/api/src/services/hubble.rs code/api/src/services/mod.rs
git commit -m "feat(network): vendor Hubble protos and generate the flow client

Vendored from cilium/cilium v1.20.4 rather than fetched at build time so
offline builds behave identically and the generated API cannot shift when
upstream moves. observer.proto pulls in flow.proto and relay.proto, so all
three are vendored. Requires protoc at build time.

The test pins the field and enum names the flow handler reads, so a future
proto bump fails loudly instead of silently misreading flows."
```

---

### Task 4: Turn a flow into a counted observation

**Files:**
- Create: `code/api/src/services/flow_watch.rs`
- Modify: `code/api/src/services/mod.rs`

**Interfaces:**
- Consumes: `classify`, `Cidr`, `default_lan_cidrs`, `FlowClass` (Task 1); `FlowCounters`, `Verdict` (Task 2); `hubble::flow` types (Task 3).
- Produces: `pub fn lan_cidrs_from_env() -> Vec<Cidr>`, `pub fn handle_flow(flow: &Flow, lan: &[Cidr], counters: &FlowCounters)`, `pub async fn run(endpoint: String, counters: Arc<FlowCounters>)`, `pub const RELAY_ENDPOINT_ENV: &str`, `pub fn relay_endpoint() -> String`.

- [ ] **Step 1: Write the failing test**

Append to `code/api/src/services/flow_watch.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::hubble::flow::{Endpoint, Flow, FlowType, Ip, TrafficDirection, Verdict as PbVerdict};

    fn egress_flow(source_namespace: &str, destination_ip: &str, labels: &[&str]) -> Flow {
        Flow {
            verdict: PbVerdict::Forwarded as i32,
            traffic_direction: TrafficDirection::Egress as i32,
            r#type: FlowType::L3L4 as i32,
            ip: Some(Ip {
                source: "10.244.0.15".to_string(),
                destination: destination_ip.to_string(),
                ..Default::default()
            }),
            source: Some(Endpoint {
                namespace: source_namespace.to_string(),
                ..Default::default()
            }),
            destination: Some(Endpoint {
                labels: labels.iter().map(|l| l.to_string()).collect(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn counts_an_egress_flow_against_its_source_app_and_class() {
        let counters = FlowCounters::new();
        handle_flow(
            &egress_flow("sonarr", "1.1.1.1", &["reserved:world"]),
            &default_lan_cidrs(),
            &counters,
        );

        assert!(counters.render().contains(
            "kubarr_network_flows_total{app=\"sonarr\",class=\"internet\",verdict=\"forwarded\"} 1"
        ));
    }

    #[test]
    fn ignores_ingress_so_each_flow_is_counted_once() {
        // Every pod-to-pod flow is observed at both ends. Counting ingress too
        // would double every number on the page.
        let counters = FlowCounters::new();
        let mut flow = egress_flow("sonarr", "10.244.0.20", &["k8s:app=radarr"]);
        flow.traffic_direction = TrafficDirection::Ingress as i32;

        handle_flow(&flow, &default_lan_cidrs(), &counters);

        assert!(!counters.render().contains("kubarr_network_flows_total{app="));
    }

    #[test]
    fn ignores_layer_seven_events_which_shadow_their_layer_three_flow() {
        let counters = FlowCounters::new();
        let mut flow = egress_flow("sonarr", "1.1.1.1", &["reserved:world"]);
        flow.r#type = FlowType::L7 as i32;

        handle_flow(&flow, &default_lan_cidrs(), &counters);

        assert!(!counters.render().contains("kubarr_network_flows_total{app="));
    }

    #[test]
    fn records_drops_under_their_own_verdict() {
        let counters = FlowCounters::new();
        let mut flow = egress_flow("qbittorrent", "192.168.1.6", &["reserved:world"]);
        flow.verdict = PbVerdict::Dropped as i32;

        handle_flow(&flow, &default_lan_cidrs(), &counters);

        assert!(counters.render().contains(
            "kubarr_network_flows_total{app=\"qbittorrent\",class=\"lan\",verdict=\"dropped\"} 1"
        ));
    }

    #[test]
    fn a_flow_with_no_source_namespace_is_attributed_to_the_cluster_itself() {
        let counters = FlowCounters::new();
        let mut flow = egress_flow("", "1.1.1.1", &["reserved:world"]);
        flow.source = Some(Endpoint::default());

        handle_flow(&flow, &default_lan_cidrs(), &counters);

        assert!(counters
            .render()
            .contains("kubarr_network_flows_total{app=\"cluster\",class=\"internet\""));
    }

    #[test]
    fn verdicts_other_than_forwarded_and_dropped_are_not_counted() {
        // AUDIT, ERROR and trace verdicts describe policy evaluation rather
        // than traffic, and would inflate the rates.
        let counters = FlowCounters::new();
        let mut flow = egress_flow("sonarr", "1.1.1.1", &["reserved:world"]);
        flow.verdict = PbVerdict::Audit as i32;

        handle_flow(&flow, &default_lan_cidrs(), &counters);

        assert!(!counters.render().contains("kubarr_network_flows_total{app="));
    }

    #[test]
    fn reads_lan_ranges_from_configuration_when_provided() {
        let parsed = parse_lan_cidrs("192.168.50.0/24, 10.10.0.0/16");
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].contains("192.168.50.9".parse().expect("ip")));

        // Nonsense entries are skipped rather than failing the worker, and an
        // entirely unusable setting falls back to the defaults.
        assert_eq!(parse_lan_cidrs("nonsense").len(), default_lan_cidrs().len());
    }

    #[test]
    fn defaults_to_the_in_cluster_relay_address() {
        assert_eq!(relay_endpoint(), "http://hubble-relay.kube-system.svc.cluster.local:80");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd code/api && cargo test --lib services::flow_watch`
Expected: FAIL — `cannot find function handle_flow`.

- [ ] **Step 3: Write minimal implementation**

Prepend to `code/api/src/services/flow_watch.rs`:

```rust
//! Streams flows from Hubble and counts them by destination class.
//!
//! The decision of what to count lives in `handle_flow`, which takes a decoded
//! flow and is tested without a server. The transport loop below is kept thin
//! for that reason.
use std::sync::Arc;
use std::time::Duration;

use crate::services::flow_classifier::{classify, default_lan_cidrs, Cidr};
use crate::services::flow_metrics::{FlowCounters, Verdict};
use crate::services::hubble::flow::{Flow, FlowType, TrafficDirection, Verdict as PbVerdict};
use crate::services::hubble::observer::{observer_client::ObserverClient, GetFlowsRequest};

/// Override for the comma-separated LAN ranges.
pub const LAN_CIDRS_ENV: &str = "KUBARR_LAN_CIDRS";
/// Override for the Hubble relay address.
pub const RELAY_ENDPOINT_ENV: &str = "KUBARR_HUBBLE_RELAY";

const DEFAULT_RELAY: &str = "http://hubble-relay.kube-system.svc.cluster.local:80";
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

pub fn relay_endpoint() -> String {
    std::env::var(RELAY_ENDPOINT_ENV).unwrap_or_else(|_| DEFAULT_RELAY.to_string())
}

pub fn lan_cidrs_from_env() -> Vec<Cidr> {
    match std::env::var(LAN_CIDRS_ENV) {
        Ok(raw) => parse_lan_cidrs(&raw),
        Err(_) => default_lan_cidrs(),
    }
}

/// Parse a comma-separated CIDR list, falling back to the defaults when the
/// setting yields nothing usable. A typo must not silently leave every LAN
/// flow counted as internet traffic.
fn parse_lan_cidrs(raw: &str) -> Vec<Cidr> {
    let parsed: Vec<Cidr> = raw
        .split(',')
        .filter_map(|entry| Cidr::parse(entry.trim()))
        .collect();
    if parsed.is_empty() {
        tracing::warn!(
            setting = LAN_CIDRS_ENV,
            "no usable LAN ranges configured, using RFC1918 defaults"
        );
        return default_lan_cidrs();
    }
    parsed
}

/// Count one flow, if it is one of the flows this pipeline counts.
pub fn handle_flow(flow: &Flow, lan: &[Cidr], counters: &FlowCounters) {
    // Egress only: every pod-to-pod flow is reported at both ends, so counting
    // ingress as well would double every number.
    if flow.traffic_direction != TrafficDirection::Egress as i32 {
        return;
    }
    // L7 events shadow the L3 flow they belong to.
    if flow.r#type != FlowType::L3L4 as i32 {
        return;
    }

    let verdict = if flow.verdict == PbVerdict::Forwarded as i32 {
        Verdict::Forwarded
    } else if flow.verdict == PbVerdict::Dropped as i32 {
        Verdict::Dropped
    } else {
        // AUDIT, ERROR and trace verdicts describe policy evaluation rather
        // than traffic.
        return;
    };

    let destination_labels = flow
        .destination
        .as_ref()
        .map(|endpoint| endpoint.labels.clone())
        .unwrap_or_default();
    let destination_ip = flow
        .ip
        .as_ref()
        .and_then(|ip| ip.destination.parse().ok());
    let class = classify(&destination_labels, destination_ip, lan);

    // Namespace-per-app is the convention, so the source namespace is the app.
    // Flows from the host or from cluster components have none.
    let app = flow
        .source
        .as_ref()
        .map(|endpoint| endpoint.namespace.as_str())
        .filter(|namespace| !namespace.is_empty())
        .unwrap_or("cluster");

    counters.record(app, class, verdict);
}

/// Stream flows until the process ends, reconnecting on failure.
///
/// While disconnected the connected gauge reads 0, which is how readers learn
/// to distrust the counters rather than seeing a convincing plateau.
pub async fn run(endpoint: String, counters: Arc<FlowCounters>) {
    let lan = lan_cidrs_from_env();
    loop {
        match stream_once(&endpoint, &lan, &counters).await {
            Ok(()) => tracing::warn!("Hubble flow stream ended, reconnecting"),
            Err(error) => tracing::warn!(%error, "Hubble flow stream failed, reconnecting"),
        }
        counters.set_connected(false);
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

async fn stream_once(
    endpoint: &str,
    lan: &[Cidr],
    counters: &FlowCounters,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut client = ObserverClient::connect(endpoint.to_string()).await?;
    let mut stream = client
        .get_flows(GetFlowsRequest {
            follow: true,
            ..Default::default()
        })
        .await?
        .into_inner();

    counters.set_connected(true);
    tracing::info!(endpoint, "Hubble flow stream connected");

    while let Some(response) = stream.message().await? {
        use crate::services::hubble::observer::get_flows_response::ResponseTypes;
        match response.response_types {
            Some(ResponseTypes::Flow(flow)) => handle_flow(&flow, lan, counters),
            // Node status and lost-event notices are not traffic.
            _ => continue,
        }
    }

    Ok(())
}
```

Add to `code/api/src/services/mod.rs`:

```rust
pub mod flow_watch;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd code/api && cargo test --lib services::flow_watch`
Expected: PASS — 8 tests.

Note: `relay_endpoint` and `lan_cidrs_from_env` read process environment, so the two tests touching them must not run concurrently with other tests that set the same variables. They only read defaults here, so no serialisation is needed.

- [ ] **Step 5: Check formatting and lints**

Run: `cd code/api && cargo fmt && cargo clippy --no-deps --lib -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic`
Expected: no output.

- [ ] **Step 6: Commit**

```bash
git add code/api/src/services/flow_watch.rs code/api/src/services/mod.rs
git commit -m "feat(network): stream Hubble flows and count them by class

Counts egress L3/L4 flows only. Every pod-to-pod flow is observed at both ends,
so counting ingress as well would double every number on the page, and L7
events shadow the L3 flow they belong to. Verdicts other than forwarded and
dropped describe policy evaluation rather than traffic and are skipped.

A misconfigured KUBARR_LAN_CIDRS falls back to RFC1918 rather than silently
counting every LAN flow as internet traffic."
```

---

### Task 5: Expose the metrics from the worker

**Files:**
- Create: `code/api/src/endpoints/metrics_server.rs`
- Modify: `code/api/src/endpoints/mod.rs`
- Modify: `code/api/src/application/runtime.rs` (in `run_worker`, after the pod watch spawn)

**Interfaces:**
- Consumes: `FlowCounters` (Task 2), `flow_watch::run`, `flow_watch::relay_endpoint` (Task 4).
- Produces: `pub fn metrics_router(counters: Arc<FlowCounters>) -> Router`, `pub async fn serve_metrics(counters: Arc<FlowCounters>, port: u16)`.

- [ ] **Step 1: Write the failing test**

Append to `code/api/src/endpoints/metrics_server.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn serves_the_counters_as_plain_text() {
        let counters = Arc::new(FlowCounters::new());
        counters.record(
            "sonarr",
            Some(crate::services::flow_classifier::FlowClass::Internet),
            crate::services::flow_metrics::Verdict::Forwarded,
        );

        let response = metrics_router(counters)
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(content_type.starts_with("text/plain"));

        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        let body = String::from_utf8(body.to_vec()).expect("utf8");
        assert!(body.contains("kubarr_network_flows_total{app=\"sonarr\""));
    }

    #[tokio::test]
    async fn exposes_nothing_else() {
        let response = metrics_router(Arc::new(FlowCounters::new()))
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd code/api && cargo test --lib endpoints::metrics_server`
Expected: FAIL — `cannot find function metrics_router`.

- [ ] **Step 3: Write minimal implementation**

Prepend to `code/api/src/endpoints/metrics_server.rs`:

```rust
//! The worker's scrape endpoint.
//!
//! The worker is otherwise a pure background process with no HTTP surface.
//! This listener exists so VictoriaMetrics can collect classified flow counts
//! through the same annotation-based pod discovery it already uses.
use std::sync::Arc;

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use crate::services::flow_metrics::FlowCounters;

pub fn metrics_router(counters: Arc<FlowCounters>) -> Router {
    Router::new()
        .route("/metrics", get(render))
        .with_state(counters)
}

async fn render(State(counters): State<Arc<FlowCounters>>) -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        counters.render(),
    )
}

/// Serve the scrape endpoint until the process ends.
pub async fn serve_metrics(counters: Arc<FlowCounters>, port: u16) {
    let address = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            // A missing scrape endpoint costs the network page its flow data;
            // it must not stop the worker doing its real work.
            tracing::warn!(%error, %address, "could not bind worker metrics listener");
            return;
        }
    };
    tracing::info!(%address, "Worker metrics listener started");
    if let Err(error) = axum::serve(listener, metrics_router(counters)).await {
        tracing::warn!(%error, "worker metrics listener stopped");
    }
}
```

Add to `code/api/src/endpoints/mod.rs`, among the existing `pub mod` lines:

```rust
pub mod metrics_server;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd code/api && cargo test --lib endpoints::metrics_server`
Expected: PASS — 2 tests.

- [ ] **Step 5: Wire both into the worker**

In `code/api/src/application/runtime.rs`, inside `run_worker`, immediately after the existing pod-status-watch spawn block (the one logging `"Pod status watch started"`), insert:

```rust
    // Classified flow counts are exposed for scraping rather than stored: VM
    // already owns retention and range queries for everything else on the
    // dashboard.
    let flow_counters = std::sync::Arc::new(crate::services::flow_metrics::FlowCounters::new());
    tokio::spawn({
        let counters = flow_counters.clone();
        let cancellation = cancellation.clone();
        async move {
            tokio::select! {
                _ = cancellation.cancelled() => {}
                _ = crate::endpoints::metrics_server::serve_metrics(counters, 9100) => {}
            }
        }
    });
    tokio::spawn({
        let counters = flow_counters.clone();
        let cancellation = cancellation.clone();
        let endpoint = crate::services::flow_watch::relay_endpoint();
        async move {
            tokio::select! {
                _ = cancellation.cancelled() => {}
                _ = crate::services::flow_watch::run(endpoint, counters) => {}
            }
        }
    });
    tracing::info!("Hubble flow watch started");
```

- [ ] **Step 6: Verify the whole suite and lints**

Run: `cd code/api && cargo fmt && cargo test --lib && cargo clippy --no-deps --lib -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic`
Expected: all library tests pass, no clippy output.

- [ ] **Step 7: Commit**

```bash
git add code/api/src/endpoints/metrics_server.rs code/api/src/endpoints/mod.rs code/api/src/application/runtime.rs
git commit -m "feat(network): expose classified flow counts from the worker

The worker had no HTTP surface at all. It now serves /metrics on 9100 so
VictoriaMetrics can scrape it through the annotation-based pod discovery it
already uses, which is why none of this is stored in Postgres.

A listener that cannot bind logs and gives up rather than taking the worker
down: losing flow data costs the network page a panel, not the queue."
```

---

### Task 6: Answer flow classes from VictoriaMetrics

**Files:**
- Modify: `code/api/src/endpoints/monitoring.rs:18` (`VICTORIAMETRICS_URL`) and `:226` (`query_vm_range`)
- Modify: `code/api/src/endpoints/networking.rs`
- Modify: `code/api/src/endpoints/mod.rs` (OpenAPI path registration)

**Interfaces:**
- Consumes: `query_vm_range` and `VICTORIAMETRICS_URL`, made `pub(crate)`.
- Produces: `GET /api/networking/flow-classes?window=15m|1h|6h|24h` returning `FlowClassResponse`.

- [ ] **Step 1: Share the VictoriaMetrics helpers**

In `code/api/src/endpoints/monitoring.rs`, change the two declarations so networking can reuse them rather than duplicating a second VM client:

```rust
pub(crate) const VICTORIAMETRICS_URL: &str =
    "http://victoriametrics.victoriametrics.svc.cluster.local:8428";
```

```rust
pub(crate) async fn query_vm_range(
    query: &str,
    start: f64,
    end: f64,
    step: &str,
) -> Vec<serde_json::Value> {
```

- [ ] **Step 2: Write the failing test**

Append to the existing `mod tests` block in `code/api/src/endpoints/networking.rs` (create the block at the end of the file if there is none):

```rust
#[cfg(test)]
mod flow_class_tests {
    use super::*;

    #[test]
    fn window_choices_map_to_a_span_and_a_readable_step() {
        // Roughly sixty points per window, so a sparkline has shape without
        // asking VM for thousands of samples.
        assert_eq!(window_span_and_step("15m"), (15 * 60, "15s".to_string()));
        assert_eq!(window_span_and_step("1h"), (60 * 60, "1m".to_string()));
        assert_eq!(window_span_and_step("6h"), (6 * 60 * 60, "6m".to_string()));
        assert_eq!(window_span_and_step("24h"), (24 * 60 * 60, "24m".to_string()));
        // Anything unrecognised falls back to the default rather than erroring.
        assert_eq!(window_span_and_step("nonsense"), (15 * 60, "15s".to_string()));
    }

    #[test]
    fn reads_per_class_rates_out_of_a_victoriametrics_instant_result() {
        let result = serde_json::json!([
            { "metric": { "class": "internet" }, "value": [1793193085.0, "12.4"] },
            { "metric": { "class": "lan" },      "value": [1793193085.0, "31.8"] }
        ]);
        let totals = totals_from_vm(result.as_array().expect("array"), &[]);

        assert_eq!(totals.internet.flows_per_sec, 12.4);
        assert_eq!(totals.lan.flows_per_sec, 31.8);
        // A class VM reported nothing for is zero, not absent: the series
        // existing with no traffic is different from no data at all, and
        // `stale` carries that distinction.
        assert_eq!(totals.cluster.flows_per_sec, 0.0);
    }

    #[test]
    fn merges_drop_rates_into_the_same_classes() {
        let forwarded = serde_json::json!([
            { "metric": { "class": "cluster" }, "value": [1793193085.0, "94.2"] }
        ]);
        let dropped = serde_json::json!([
            { "metric": { "class": "cluster" }, "value": [1793193085.0, "0.2"] }
        ]);

        let totals = totals_from_vm(
            forwarded.as_array().expect("array"),
            dropped.as_array().expect("array"),
        );

        assert_eq!(totals.cluster.flows_per_sec, 94.2);
        assert_eq!(totals.cluster.drops_per_sec, 0.2);
    }

    #[test]
    fn groups_per_app_rates_by_app_and_class() {
        let result = serde_json::json!([
            { "metric": { "app": "sonarr", "class": "internet" }, "value": [1.0, "2.1"] },
            { "metric": { "app": "sonarr", "class": "cluster" },  "value": [1.0, "8.8"] },
            { "metric": { "app": "radarr", "class": "lan" },      "value": [1.0, "0.4"] }
        ]);

        let apps = apps_from_vm(result.as_array().expect("array"), &[]);

        let sonarr = apps.iter().find(|a| a.app == "sonarr").expect("sonarr");
        assert_eq!(sonarr.internet.flows_per_sec, 2.1);
        assert_eq!(sonarr.cluster.flows_per_sec, 8.8);
        let radarr = apps.iter().find(|a| a.app == "radarr").expect("radarr");
        assert_eq!(radarr.lan.flows_per_sec, 0.4);
        // Sorted so the response is stable between calls.
        assert_eq!(apps[0].app, "radarr");
    }

    #[test]
    fn an_absent_connected_gauge_means_the_data_cannot_be_trusted() {
        assert!(is_stale(&[]));
        let disconnected = serde_json::json!([{ "metric": {}, "value": [1.0, "0"] }]);
        assert!(is_stale(disconnected.as_array().expect("array")));
        let connected = serde_json::json!([{ "metric": {}, "value": [1.0, "1"] }]);
        assert!(!is_stale(connected.as_array().expect("array")));
    }

    #[test]
    fn malformed_samples_are_skipped_rather_than_read_as_zero() {
        let result = serde_json::json!([
            { "metric": { "class": "internet" }, "value": [1.0, "not-a-number"] },
            { "metric": { "class": "lan" },      "value": [1.0, "7.5"] }
        ]);

        let totals = totals_from_vm(result.as_array().expect("array"), &[]);

        assert_eq!(totals.internet.flows_per_sec, 0.0);
        assert_eq!(totals.lan.flows_per_sec, 7.5);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd code/api && cargo test --lib endpoints::networking::flow_class_tests`
Expected: FAIL — `cannot find function window_span_and_step`.

- [ ] **Step 4: Write minimal implementation**

Add to `code/api/src/endpoints/networking.rs`:

```rust
// ============================================================================
// Flow classes
// ============================================================================

/// Flow rates for one destination class.
#[derive(Debug, Default, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ClassRates {
    pub flows_per_sec: f64,
    pub drops_per_sec: f64,
}

#[derive(Debug, Default, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ClassTotals {
    pub internet: ClassRates,
    pub lan: ClassRates,
    pub cluster: ClassRates,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AppClassRates {
    pub app: String,
    pub internet: ClassRates,
    pub lan: ClassRates,
    pub cluster: ClassRates,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct FlowSeriesPoint {
    pub t: i64,
    pub flows_per_sec: f64,
}

#[derive(Debug, Default, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct FlowSeries {
    pub internet: Vec<FlowSeriesPoint>,
    pub lan: Vec<FlowSeriesPoint>,
    pub cluster: Vec<FlowSeriesPoint>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct FlowClassResponse {
    pub observed_at: i64,
    /// True when the flow watch is not reporting. The UI must say so rather
    /// than drawing a convincing zero.
    pub stale: bool,
    pub totals: ClassTotals,
    pub by_app: Vec<AppClassRates>,
    pub series: FlowSeries,
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct FlowClassQuery {
    pub window: Option<String>,
}

/// Span in seconds and a step that yields roughly sixty points.
fn window_span_and_step(window: &str) -> (i64, String) {
    match window {
        "1h" => (60 * 60, "1m".to_string()),
        "6h" => (6 * 60 * 60, "6m".to_string()),
        "24h" => (24 * 60 * 60, "24m".to_string()),
        _ => (15 * 60, "15s".to_string()),
    }
}

fn sample_value(entry: &serde_json::Value) -> Option<f64> {
    entry
        .get("value")
        .and_then(|value| value.as_array())
        .and_then(|pair| pair.get(1))
        .and_then(|raw| raw.as_str())
        .and_then(|text| text.parse::<f64>().ok())
}

fn class_of(entry: &serde_json::Value) -> Option<String> {
    entry
        .get("metric")
        .and_then(|metric| metric.get("class"))
        .and_then(|class| class.as_str())
        .map(str::to_string)
}

fn rates_for<'a>(totals: &'a mut ClassTotals, class: &str) -> Option<&'a mut ClassRates> {
    match class {
        "internet" => Some(&mut totals.internet),
        "lan" => Some(&mut totals.lan),
        "cluster" => Some(&mut totals.cluster),
        // "unclassified" is deliberately not surfaced: it is a diagnostic for
        // the metrics endpoint, not a category users can act on.
        _ => None,
    }
}

fn totals_from_vm(forwarded: &[serde_json::Value], dropped: &[serde_json::Value]) -> ClassTotals {
    let mut totals = ClassTotals::default();
    for entry in forwarded {
        if let (Some(class), Some(value)) = (class_of(entry), sample_value(entry)) {
            if let Some(rates) = rates_for(&mut totals, &class) {
                rates.flows_per_sec = value;
            }
        }
    }
    for entry in dropped {
        if let (Some(class), Some(value)) = (class_of(entry), sample_value(entry)) {
            if let Some(rates) = rates_for(&mut totals, &class) {
                rates.drops_per_sec = value;
            }
        }
    }
    totals
}

fn apps_from_vm(
    forwarded: &[serde_json::Value],
    dropped: &[serde_json::Value],
) -> Vec<AppClassRates> {
    use std::collections::BTreeMap;

    let mut by_app: BTreeMap<String, ClassTotals> = BTreeMap::new();
    let mut absorb = |entries: &[serde_json::Value], drops: bool| {
        for entry in entries {
            let app = entry
                .get("metric")
                .and_then(|metric| metric.get("app"))
                .and_then(|app| app.as_str())
                .unwrap_or_default()
                .to_string();
            if app.is_empty() {
                continue;
            }
            let (Some(class), Some(value)) = (class_of(entry), sample_value(entry)) else {
                continue;
            };
            let totals = by_app.entry(app).or_default();
            if let Some(rates) = rates_for(totals, &class) {
                if drops {
                    rates.drops_per_sec = value;
                } else {
                    rates.flows_per_sec = value;
                }
            }
        }
    };
    absorb(forwarded, false);
    absorb(dropped, true);

    by_app
        .into_iter()
        .map(|(app, totals)| AppClassRates {
            app,
            internet: totals.internet,
            lan: totals.lan,
            cluster: totals.cluster,
        })
        .collect()
}

/// The watch is trusted only while it says it is connected.
fn is_stale(connected: &[serde_json::Value]) -> bool {
    connected
        .iter()
        .filter_map(sample_value)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|value| value < 1.0)
        .unwrap_or(true)
}

fn series_from_vm(result: &[serde_json::Value]) -> FlowSeries {
    let mut series = FlowSeries::default();
    for entry in result {
        let Some(class) = class_of(entry) else {
            continue;
        };
        let points: Vec<FlowSeriesPoint> = entry
            .get("values")
            .and_then(|values| values.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|pair| {
                        let pair = pair.as_array()?;
                        let t = pair.first()?.as_f64()? as i64;
                        let value = pair.get(1)?.as_str()?.parse::<f64>().ok()?;
                        Some(FlowSeriesPoint {
                            t,
                            flows_per_sec: value,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        match class.as_str() {
            "internet" => series.internet = points,
            "lan" => series.lan = points,
            "cluster" => series.cluster = points,
            _ => {}
        }
    }
    series
}

/// Flow rates by destination class
#[utoipa::path(
    get,
    path = "/api/networking/flow-classes",
    tag = "Networking",
    params(FlowClassQuery),
    responses((status = 200, body = FlowClassResponse))
)]
async fn get_flow_classes(
    Query(query): Query<FlowClassQuery>,
    _auth: Authorized<MonitoringView>,
) -> Result<impl IntoResponse> {
    use crate::endpoints::monitoring::query_vm_range;

    let window = query.window.unwrap_or_else(|| "15m".to_string());
    let (span, step) = window_span_and_step(&window);
    let end = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as f64)
        .unwrap_or_default();
    let start = end - span as f64;

    let forwarded = query_vm_range(
        "sum by (class) (rate(kubarr_network_flows_total{verdict=\"forwarded\"}[1m]))",
        end,
        end,
        &step,
    )
    .await;
    let dropped = query_vm_range(
        "sum by (class) (rate(kubarr_network_flows_total{verdict=\"dropped\"}[1m]))",
        end,
        end,
        &step,
    )
    .await;
    let per_app = query_vm_range(
        "sum by (app, class) (rate(kubarr_network_flows_total{verdict=\"forwarded\"}[1m]))",
        end,
        end,
        &step,
    )
    .await;
    let per_app_drops = query_vm_range(
        "sum by (app, class) (rate(kubarr_network_flows_total{verdict=\"dropped\"}[1m]))",
        end,
        end,
        &step,
    )
    .await;
    let history = query_vm_range(
        "sum by (class) (rate(kubarr_network_flows_total{verdict=\"forwarded\"}[1m]))",
        start,
        end,
        &step,
    )
    .await;
    let connected = query_vm_range("max(kubarr_network_flow_watch_connected)", end, end, &step).await;

    Ok((
        [crate::endpoints::short_cache(10)],
        Json(FlowClassResponse {
            observed_at: end as i64,
            stale: is_stale(&connected),
            totals: totals_from_vm(&forwarded, &dropped),
            by_app: apps_from_vm(&per_app, &per_app_drops),
            series: series_from_vm(&history),
        }),
    ))
}
```

Register the route in `networking_routes`:

```rust
        .route("/flow-classes", get(get_flow_classes))
```

Register the OpenAPI path in `code/api/src/endpoints/mod.rs`, beside the existing `networking::` entries:

```rust
        networking::get_flow_classes,
```

Ensure `networking.rs` imports what the handler needs (add any that are missing to the existing `use` block):

```rust
use axum::extract::Query;
use axum::response::IntoResponse;
use crate::middleware::permissions::{Authorized, MonitoringView};
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd code/api && cargo test --lib endpoints::networking`
Expected: PASS — 6 new tests plus any existing networking tests.

- [ ] **Step 6: Check formatting, lints and the whole suite**

Run: `cd code/api && cargo fmt && cargo clippy --no-deps --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic && cargo test`
Expected: all pass, no clippy output.

- [ ] **Step 7: Commit**

```bash
git add code/api/src/endpoints/networking.rs code/api/src/endpoints/monitoring.rs code/api/src/endpoints/mod.rs
git commit -m "feat(network): serve flow rates by destination class

Queries VictoriaMetrics through the same range-query helper that backs the
dashboard's network history, so retention and downsampling stay in one place.

The response carries stale explicitly: a class with no traffic and a watch that
has stopped reporting both look like zero, and only one of them should be drawn
as a zero. Malformed samples are skipped rather than read as zero for the same
reason. Unclassified flows stay out of the response; they are a diagnostic for
the metrics endpoint, not a category anyone can act on."
```

---

### Task 7: Let VictoriaMetrics reach the worker

**Files:**
- Modify: `kubarr-charts/system/kubarr-worker/values.yaml`
- Modify: `kubarr-charts/system/kubarr-worker/templates/deployment.yaml`
- Modify: `kubarr-charts/system/kubarr-worker/Chart.yaml`
- Modify: `kubarr-charts/monitoring/victoriametrics/values.yaml`
- Modify: `kubarr-charts/monitoring/victoriametrics/Chart.yaml`
- Create: `kubarr-charts/tests/test_flow_metrics_scrape.py`

**Interfaces:**
- Consumes: the worker serving `/metrics` on 9100 (Task 5).
- Produces: a scrapeable worker pod and a VM egress policy that permits it.

Without both halves the pipeline silently produces nothing: the worker would
expose metrics nobody collects, because VM runs default-deny egress with an
explicit namespace allow-list.

- [ ] **Step 1: Write the failing test**

Create `kubarr-charts/tests/test_flow_metrics_scrape.py`:

```python
"""The worker's flow metrics must be reachable by VictoriaMetrics.

Two independent halves have to agree: the worker pod must carry the
prometheus.io annotations the kubernetes-pods scrape job keys on, and VM's
default-deny egress policy must permit the worker's namespace. If either is
missing the network page shows nothing and nothing reports an error.
"""

import unittest

import yaml

from chart_artifacts import chart_input, render_chart


class WorkerScrapeAnnotations(unittest.TestCase):
    def test_worker_pod_is_annotated_for_scraping(self):
        docs = render_chart(chart_input("system/kubarr-worker"))
        deployments = [d for d in docs if d and d.get("kind") == "Deployment"]
        self.assertEqual(len(deployments), 1)
        annotations = (
            deployments[0]["spec"]["template"]["metadata"].get("annotations") or {}
        )
        self.assertEqual(annotations.get("prometheus.io/scrape"), "true")
        self.assertEqual(annotations.get("prometheus.io/port"), "9100")
        self.assertEqual(annotations.get("prometheus.io/path"), "/metrics")

    def test_worker_container_declares_the_metrics_port(self):
        docs = render_chart(chart_input("system/kubarr-worker"))
        deployment = next(d for d in docs if d and d.get("kind") == "Deployment")
        containers = deployment["spec"]["template"]["spec"]["containers"]
        worker = next(c for c in containers if c["name"] == "worker")
        ports = {p["containerPort"] for p in worker.get("ports", [])}
        self.assertIn(9100, ports)


class VictoriaMetricsEgress(unittest.TestCase):
    def test_victoriametrics_may_scrape_the_worker_namespace(self):
        docs = render_chart(chart_input("monitoring/victoriametrics"))
        policies = [d for d in docs if d and d.get("kind") == "NetworkPolicy"]
        self.assertTrue(policies, "victoriametrics must ship a NetworkPolicy")
        allowed = set()
        for policy in policies:
            for rule in policy["spec"].get("egress", []):
                for target in rule.get("to", []):
                    selector = target.get("namespaceSelector", {})
                    labels = selector.get("matchLabels", {})
                    name = labels.get("kubernetes.io/metadata.name")
                    if name:
                        allowed.add(name)
        self.assertIn(
            "kubarr-worker",
            allowed,
            "VM cannot scrape the worker, so flow classes would be silently empty",
        )


if __name__ == "__main__":
    unittest.main()
```

Check the helper names in `kubarr-charts/tests/chart_artifacts.py` before running; if the module exposes a different render helper than `render_chart(chart_input(...))`, use whatever the neighbouring tests in that directory use and keep the assertions unchanged.

- [ ] **Step 2: Run test to verify it fails**

```bash
cd kubarr-charts
PKG=$(mktemp -d)
while IFS= read -r chart; do helm package "$(dirname "$chart")" --destination "$PKG" >/dev/null 2>&1; done < <(find . -mindepth 2 -name Chart.yaml | sort)
CHART_PACKAGES_DIR=$PKG python3 -B -m unittest tests.test_flow_metrics_scrape -v
```

Expected: FAIL on both the annotations and the egress assertion.

- [ ] **Step 3: Write minimal implementation**

In `kubarr-charts/system/kubarr-worker/values.yaml`, set the annotations and port (replacing `podAnnotations: {}` if present):

```yaml
# The kubernetes-pods scrape job keys on these annotations, so the worker's
# classified flow counts are collected without a dedicated scrape config.
podAnnotations:
  prometheus.io/scrape: "true"
  prometheus.io/port: "9100"
  prometheus.io/path: "/metrics"

metrics:
  port: 9100
```

In `kubarr-charts/system/kubarr-worker/templates/deployment.yaml`, inside the `worker` container spec, add:

```yaml
          ports:
            - name: metrics
              containerPort: {{ .Values.metrics.port }}
              protocol: TCP
```

In `kubarr-charts/monitoring/victoriametrics/values.yaml`, add `kubarr-worker` to the `networkPolicy.scrapeNamespaces` list, keeping the existing entries.

Bump both charts: `system/kubarr-worker/Chart.yaml` to `version: 0.3.5+1`, and `monitoring/victoriametrics/Chart.yaml` from `1.154.1+14` to `1.154.1+15`.

- [ ] **Step 4: Run test to verify it passes**

Re-run the packaged test command from Step 2.
Expected: PASS — 3 tests.

- [ ] **Step 5: Run the whole chart suite**

```bash
cd kubarr-charts
PKG=$(mktemp -d)
while IFS= read -r chart; do helm package "$(dirname "$chart")" --destination "$PKG" >/dev/null 2>&1; done < <(find . -mindepth 2 -name Chart.yaml | sort)
CHART_PACKAGES_DIR=$PKG python3 -B -m unittest discover -s tests
```

Expected: all tests pass (172 before this task, 175 after).

- [ ] **Step 6: Commit**

```bash
cd kubarr-charts
git add system/kubarr-worker monitoring/victoriametrics tests/test_flow_metrics_scrape.py
git commit -m "feat(network): let VictoriaMetrics scrape the worker's flow counts

Two halves have to agree or the pipeline silently produces nothing. The worker
pod gains the prometheus.io annotations the existing kubernetes-pods job keys
on, so no new scrape config is needed. VM runs default-deny egress with an
explicit namespace allow-list that did not include kubarr-worker, so it could
not have reached the worker at all; the failure mode would have been an empty
graph with no error reported anywhere.

The test asserts both halves, because each is useless alone."
```

---

### Task 8: Ensure Hubble on an existing Cilium install

**Files:**
- Create: `kubarr-cli/src/cni.rs`
- Modify: `kubarr-cli/src/main.rs` (module declaration)
- Modify: `kubarr-cli/src/bootstrap.rs` (call site and chart version pins)
- Modify: `kubarr-cli/src/charts.rs` (chart version pins)
- Modify: `kubarr-cli/src/observability.rs` (VM scrape namespace list)

**Interfaces:**
- Produces: `pub fn ensure_hubble(options: &BootstrapOptions) -> Result<(), String>`, `pub fn hubble_upgrade_args(release: &str, namespace: &str) -> Vec<String>`, `pub enum CniState { CiliumWithHubble, CiliumWithoutHubble, OtherCni, Unknown }`, `pub fn cni_state_from_releases(json: &str) -> CniState`.

Scope, restated: this ensures Hubble on an **already present** Cilium release
and reports clearly when the CNI is something else. It does not install a CNI.
The CLI's single-node path installs k3s with its default flannel CNI, and
switching that to Cilium changes cluster bootstrap and the acceptance harness;
that belongs to its own spec.

- [ ] **Step 1: Write the failing test**

Append to `kubarr-cli/src/cni.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_a_cilium_release() {
        let releases = r#"[{"name":"cilium","namespace":"kube-system","chart":"cilium-1.20.4","status":"deployed"}]"#;
        assert_eq!(
            cni_state_from_releases(releases),
            CniState::CiliumWithoutHubble
        );
    }

    #[test]
    fn recognises_a_cluster_running_something_else() {
        let releases = r#"[{"name":"calico","namespace":"kube-system","chart":"tigera-operator-3.29.0","status":"deployed"}]"#;
        assert_eq!(cni_state_from_releases(releases), CniState::OtherCni);

        // No Helm-managed CNI at all: k3s with its bundled flannel looks like
        // this, and it is not something to overwrite.
        assert_eq!(cni_state_from_releases("[]"), CniState::OtherCni);
    }

    #[test]
    fn unparseable_helm_output_is_not_mistaken_for_an_absent_cni() {
        assert_eq!(cni_state_from_releases("not json"), CniState::Unknown);
    }

    #[test]
    fn upgrade_reuses_existing_values_and_only_adds_hubble() {
        let args = hubble_upgrade_args("cilium", "kube-system");

        // --reuse-values matters: this upgrades somebody's working CNI, and
        // dropping their settings would be a connectivity outage.
        assert!(args.contains(&"--reuse-values".to_string()));
        assert!(args.contains(&"hubble.enabled=true".to_string()));
        assert!(args.contains(&"hubble.relay.enabled=true".to_string()));
        assert!(args.contains(&"cilium".to_string()));
        assert!(args.contains(&"kube-system".to_string()));
        // Never pins or changes the chart version of a running CNI.
        assert!(!args.iter().any(|arg| arg == "--version"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd kubarr-cli && cargo test cni`
Expected: FAIL — `cannot find function cni_state_from_releases`.

- [ ] **Step 3: Write minimal implementation**

Prepend to `kubarr-cli/src/cni.rs`:

```rust
//! Enables Hubble on an existing Cilium install.
//!
//! Flow classification needs Hubble's relay. This step turns it on where
//! Cilium is already running and says so plainly where it is not: installing a
//! CNI into a cluster that already has one is destructive, and the single-node
//! path ships k3s with its own flannel.
use crate::install_events::Operation;
use crate::style::{detail, ok, step, warn};
use crate::types::BootstrapOptions;
use crate::util::{check_tool, run, try_run_or_print_progress};

const CILIUM_RELEASE: &str = "cilium";
const CILIUM_NAMESPACE: &str = "kube-system";

#[derive(Debug, PartialEq, Eq)]
pub enum CniState {
    CiliumWithHubble,
    CiliumWithoutHubble,
    OtherCni,
    Unknown,
}

/// Inspect `helm list -A -o json` output for a Cilium release.
pub fn cni_state_from_releases(json: &str) -> CniState {
    let Ok(releases) = serde_json::from_str::<serde_json::Value>(json) else {
        // Unreadable output is not evidence that no CNI exists.
        return CniState::Unknown;
    };
    let Some(releases) = releases.as_array() else {
        return CniState::Unknown;
    };
    for release in releases {
        let chart = release
            .get("chart")
            .and_then(|chart| chart.as_str())
            .unwrap_or_default();
        if chart.starts_with("cilium-") {
            return CniState::CiliumWithoutHubble;
        }
    }
    CniState::OtherCni
}

/// Helm arguments that turn Hubble on without disturbing anything else.
pub fn hubble_upgrade_args(release: &str, namespace: &str) -> Vec<String> {
    vec![
        "upgrade".to_string(),
        release.to_string(),
        "cilium/cilium".to_string(),
        "-n".to_string(),
        namespace.to_string(),
        // Keep the operator's existing values: this is a running CNI.
        "--reuse-values".to_string(),
        "--set".to_string(),
        "hubble.enabled=true".to_string(),
        "--set".to_string(),
        "hubble.relay.enabled=true".to_string(),
        "--timeout".to_string(),
        crate::util::HELM_TIMEOUT.to_string(),
    ]
}

pub fn ensure_hubble(options: &BootstrapOptions) -> Result<(), String> {
    step("Network flows", "checking whether Hubble can be enabled");
    if options.install.dry_run {
        detail("plan", "ensure Hubble on an existing Cilium install");
        return Ok(());
    }
    check_tool("helm")?;

    let releases = run("helm", &["list", "-A", "-o", "json"]).unwrap_or_default();
    match cni_state_from_releases(&releases) {
        CniState::CiliumWithHubble => {
            ok("Hubble already enabled");
            Ok(())
        }
        CniState::CiliumWithoutHubble => {
            let args = hubble_upgrade_args(CILIUM_RELEASE, CILIUM_NAMESPACE);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            try_run_or_print_progress("helm", &refs, false, false, Some(Operation::Hubble))
        }
        CniState::OtherCni => {
            // Not an error: the installer's job is Kubarr, not the CNI.
            warn("no Cilium release found; network flow classes will be unavailable");
            Ok(())
        }
        CniState::Unknown => {
            warn("could not read Helm releases; leaving the CNI untouched");
            Ok(())
        }
    }
}
```

Add `Hubble` to the `Operation` enum in `kubarr-cli/src/install_events.rs`, following the existing entries, and give it a timeout branch in `kubarr-cli/src/util.rs` beside `Operation::VictoriaMetrics`:

```rust
    Hubble,
```

Add `mod cni;` to `kubarr-cli/src/main.rs` beside the other module declarations.

In `kubarr-cli/src/bootstrap.rs`, call it after `install_victoriametrics(options)?;`, so the scrape target exists before flows start being counted:

```rust
    crate::cni::ensure_hubble(options)?;
```

In the same file, update the chart version pins at `bootstrap.rs:131` and `charts.rs:17` from `("kubarr-worker", "0.3.4+1")` to `("kubarr-worker", "0.3.5+1")`, and update any `victoriametrics` pin to `1.154.1+15`.

In `kubarr-cli/src/observability.rs`, add `"kubarr-worker"` to the `networkPolicy.scrapeNamespaces` list in `install_victoriametrics`, so a CLI install matches the chart default from Task 7.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd kubarr-cli && cargo test cni`
Expected: PASS — 4 tests.

- [ ] **Step 5: Verify the CLI suite, formatting and lints**

Run: `cd kubarr-cli && cargo fmt && cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
cd kubarr-cli
git add src/cni.rs src/main.rs src/bootstrap.rs src/charts.rs src/observability.rs src/install_events.rs src/util.rs
git commit -m "feat(network): enable Hubble where Cilium is already installed

Flow classification needs Hubble's relay. This turns it on with --reuse-values
so a running CNI keeps every setting it already has, because dropping those
would be a connectivity outage rather than a failed install.

Where no Cilium release exists the step warns and continues instead of
installing a CNI: the single-node path ships k3s with its own flannel, and
replacing a working CNI is not something an app installer should do quietly.
Unreadable Helm output is treated as unknown rather than as an absent CNI.

Also adds the worker namespace to VictoriaMetrics' scrape allow-list so a CLI
install matches the chart default."
```

---

## Self-Review

**Spec coverage.** Every spec section maps to a task: classification rules and LAN defaults → Task 1; metric names and rendering → Task 2; vendored protos and the `GetFlows` RPC → Task 3; stream consumption, reconnect and the connected gauge → Task 4; the `/metrics` listener and worker wiring → Task 5; the API shape, `stale`, windows and VM reuse → Task 6; both chart changes → Task 7; the CLI's Hubble step → Task 8.

**Two deliberate departures from the spec, both corrections found while
verifying against upstream and the codebase:**

1. The spec calls the RPC `Observe`. It is `observer.Observer/GetFlows` with
   `follow=true`, and `observer.proto` also imports `relay/relay.proto`, which
   the spec did not mention vendoring. Task 3 reflects the real protos.
2. The spec did not mention direction filtering. Flows are reported at both
   ends, so counting everything would double every figure; `traffic_direction
   == EGRESS` and `Type == L3_L4` are now explicit constraints and have their
   own tests in Task 4.

**One deliberate scope reduction.** The spec assigns the Cilium install to the
CLI. Task 8 only ensures Hubble on an existing Cilium release, because the CLI's
single-node path installs k3s with flannel and switching that is a change to
cluster bootstrap with its own blast radius. Flagged in the File Structure
section and in Task 8 itself; needs its own decision.

**Placeholder scan.** No TBDs, no "add error handling", no "similar to Task N".
Every code step carries the actual code; every test step carries the actual
assertions and the command to run.

**Type consistency.** `FlowClass`, `Cidr`, `class_label`, `FlowCounters`,
`Verdict`, `handle_flow`, `relay_endpoint`, `query_vm_range`, `ClassRates`,
`ClassTotals`, `AppClassRates`, `FlowSeries`, `FlowClassResponse` and
`CniState` are each defined once and referenced with the same signature
everywhere. `parse_lan_cidrs` is private and tested from the same module.

**Out of scope, unchanged from the spec:** sub-project B (the page and tile
rework), per-class byte volumes, Hubble UI, and L7 metrics.
