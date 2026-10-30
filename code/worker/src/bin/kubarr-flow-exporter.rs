//! Turns Cilium's conntrack table into per-class byte counters.
//!
//! Hubble counts flow *events* and carries no byte field anywhere -- verified
//! against live flows, not just the protocol definition -- so the network page
//! could only ever report connections per second. Cilium's BPF conntrack map is
//! the one place that holds bytes *and* the address they went to, once
//! `bpf.ctAccounting` is enabled.
//!
//! There is no metric for that map, so it has to be read. A privileged sidecar
//! dumps it to a shared file on a timer and this process reads the file, which
//! keeps the privilege in the one container that needs it. This process holds
//! no capabilities and never touches the host.
//!
//! What it deliberately does not measure: storage. NFS is mounted by the node's
//! own kernel rather than by the pods that use it, so that traffic never
//! crosses a Cilium endpoint and the conntrack map has no entry for it --
//! measured on a live cluster while reading hundreds of megabytes off the
//! share, both conntrack and Hubble reported exactly zero. Storage bytes come
//! from node-exporter's `mountstats` collector instead.

use std::collections::HashMap;
use std::io::Write;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Where the privileged sidecar leaves its conntrack dump.
const DEFAULT_DUMP: &str = "/run/conntrack/ct.txt";
const DEFAULT_LISTEN: &str = "0.0.0.0:9970";
const DEFAULT_INTERVAL_SECONDS: u64 = 15;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Class {
    Internet,
    Lan,
    Cluster,
}

impl Class {
    fn label(self) -> &'static str {
        match self {
            Class::Internet => "internet",
            Class::Lan => "lan",
            Class::Cluster => "cluster",
        }
    }
}

/// An IPv4 network, matched by masking. Only v4 is handled: Cilium prints v6
/// conntrack entries in a different shape, and this cluster has no v6 traffic
/// to classify. A v6 line is skipped rather than guessed at.
#[derive(Clone, Copy, Debug)]
struct Cidr {
    base: u32,
    mask: u32,
}

impl Cidr {
    fn parse(text: &str) -> Option<Self> {
        let (addr, bits) = text.split_once('/')?;
        let addr: Ipv4Addr = addr.trim().parse().ok()?;
        let bits: u32 = bits.trim().parse().ok()?;
        if bits > 32 {
            return None;
        }
        // A /0 would shift by 32, which is undefined for u32.
        let mask = if bits == 0 {
            0
        } else {
            u32::MAX << (32 - bits)
        };
        Some(Cidr {
            base: u32::from(addr) & mask,
            mask,
        })
    }

    fn contains(&self, addr: Ipv4Addr) -> bool {
        u32::from(addr) & self.mask == self.base
    }
}

#[derive(Clone, Debug)]
struct Topology {
    /// Addresses belonging to this cluster: the pod ranges, plus the node.
    local: Vec<Cidr>,
    /// The pod ranges alone. External traffic is only counted when a Pod is
    /// one end of it; see `classify`.
    pods: Vec<Cidr>,
    /// Ranges that count as the local network rather than the internet.
    lan: Vec<Cidr>,
}

impl Topology {
    fn from_env() -> Self {
        let pod_cidrs = env_or("KUBARR_POD_CIDRS", "10.244.0.0/16");
        let lan_cidrs = env_or(
            "KUBARR_LAN_CIDRS",
            "10.0.0.0/8,172.16.0.0/12,192.168.0.0/16",
        );

        let mut local: Vec<Cidr> = parse_cidrs(&pod_cidrs);
        // The node's own address is cluster infrastructure, not the LAN. Without
        // it, every pod-to-apiserver call would be filed as LAN traffic -- the
        // same ordering mistake that once put 438 control-plane flows in the
        // wrong bucket.
        if let Ok(node_ip) = std::env::var("KUBARR_NODE_IP") {
            if let Ok(addr) = node_ip.trim().parse::<Ipv4Addr>() {
                local.push(Cidr {
                    base: u32::from(addr),
                    mask: u32::MAX,
                });
            }
        }

        Topology {
            local,
            pods: parse_cidrs(&pod_cidrs),
            lan: parse_cidrs(&lan_cidrs),
        }
    }

    fn is_local(&self, addr: Ipv4Addr) -> bool {
        self.local.iter().any(|net| net.contains(addr))
    }

    fn classify(&self, source: Ipv4Addr, destination: Ipv4Addr) -> Option<Class> {
        let source_local = self.is_local(source);
        let destination_local = self.is_local(destination);
        if source_local && destination_local {
            return Some(Class::Cluster);
        }
        // Whichever end is not ours is the one that decides the class. A
        // conntrack entry is stored from the endpoint's point of view, so the
        // remote address is not always the one printed second.
        let remote = if source_local { destination } else { source };
        if self.is_local(remote) {
            return Some(Class::Cluster);
        }
        // External traffic is counted at the pod, never at the node.
        //
        // An app behind a VPN produces two conntrack entries for one download:
        // the pod's connection to the peer, and the node's connection to the
        // VPN server carrying the same bytes wrapped. Counting both reports
        // the same payload twice. Measured while qBittorrent was downloading:
        // pod entries totalled 26.5 MB/s, which matches cAdvisor's 27.3 MB/s
        // for that pod, while the node's post-masquerade entries added another
        // 18.6 MB/s of the same data.
        //
        // The node's own external traffic -- image pulls, the VPN tunnel
        // itself -- is therefore not in these numbers. It is counted
        // separately as host bytes so that nothing is simply invisible.
        let local_side = if source_local { source } else { destination };
        if !self.is_pod(local_side) {
            return None;
        }
        if self.lan.iter().any(|net| net.contains(remote)) {
            Some(Class::Lan)
        } else {
            Some(Class::Internet)
        }
    }

    /// True for an address belonging to a Pod, as opposed to the node itself.
    fn is_pod(&self, addr: Ipv4Addr) -> bool {
        self.pods.iter().any(|net| net.contains(addr))
    }

    /// Name one end of a link for the edge metric.
    fn endpoint_name(&self, addr: Ipv4Addr) -> String {
        if self.is_pod(addr) {
            return addr.to_string();
        }
        if self.is_local(addr) {
            return "host".to_string();
        }
        if self.lan.iter().any(|net| net.contains(addr)) {
            "lan".to_string()
        } else {
            "internet".to_string()
        }
    }

    /// The link a connection represents, or None when neither end is a Pod.
    ///
    /// A link with no Pod on it is the node talking to the outside world, which
    /// the classes already exclude to avoid counting a VPN payload twice; the
    /// graph leaves it out for the same reason.
    fn edge(&self, source: Ipv4Addr, destination: Ipv4Addr) -> Option<Edge> {
        if !self.is_pod(source) && !self.is_pod(destination) {
            return None;
        }
        Some(Edge {
            source: self.endpoint_name(source),
            destination: self.endpoint_name(destination),
        })
    }
}

fn parse_cidrs(list: &str) -> Vec<Cidr> {
    list.split(',')
        .filter_map(|entry| Cidr::parse(entry.trim()))
        .collect()
}

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| fallback.to_string())
}

/// One connection, reduced to what classification needs.
///
/// Direction is deliberately not part of this. Cilium stores a forward and a
/// reverse conntrack entry for the same connection, printed as `IN` and `OUT`,
/// and both carry the *same* totals rather than one direction each -- measured
/// on a live cluster, 9250 of 14633 tuples appeared twice with byte-for-byte
/// identical counters. Keying on direction counted every connection twice.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Key {
    source: Ipv4Addr,
    source_port: u16,
    destination: Ipv4Addr,
    destination_port: u16,
}

/// Parse a line of `cilium-dbg bpf ct list global`.
///
/// ```text
/// TCP OUT 10.244.0.75:37238 -> 10.244.0.165:8428 expires=... Packets=10 Bytes=1336 ...
/// ```
fn parse_line(line: &str) -> Option<(Key, u64)> {
    let mut parts = line.split_whitespace();
    let _proto = parts.next()?;
    let direction = parts.next()?;
    if direction != "IN" && direction != "OUT" {
        // SVC rows are the load balancer's side of a service connection, and
        // the backend's own IN/OUT row counts the same bytes. Taking both
        // would double every NodePort byte -- on this cluster that was 2494
        // rows out of 15000. Skipped on purpose, and not a parse failure.
        return None;
    }
    let source = parts.next()?;
    let arrow = parts.next()?;
    if arrow != "->" {
        return None;
    }
    let destination = parts.next()?;

    let bytes = line
        .split_whitespace()
        .find_map(|token| token.strip_prefix("Bytes="))
        .and_then(|value| value.parse::<u64>().ok())?;

    let (source, source_port) = split_endpoint(source)?;
    let (destination, destination_port) = split_endpoint(destination)?;

    Some((
        Key {
            source,
            source_port,
            destination,
            destination_port,
        },
        bytes,
    ))
}

/// A service row, which is skipped rather than failed: see `parse_line`.
fn is_service_row(line: &str) -> bool {
    let mut parts = line.split_whitespace();
    parts.next();
    parts.next() == Some("SVC")
}

fn split_endpoint(text: &str) -> Option<(Ipv4Addr, u16)> {
    let (addr, port) = text.rsplit_once(':')?;
    Some((addr.parse().ok()?, port.parse().ok()?))
}

/// One measured link, named the way the metric labels it.
///
/// Pod addresses are emitted as addresses: this process has no way to resolve
/// them to a namespace (the identity list needs the agent's own API socket),
/// and the backend already knows which Pod holds which address. External ends
/// collapse to their class instead, because a torrent client talks to
/// thousands of peers and one series each would be a cardinality bomb.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Edge {
    source: String,
    destination: String,
}

#[derive(Default)]
struct Totals {
    bytes: HashMap<&'static str, u64>,
    /// Bytes per link, for the topology graph.
    edges: HashMap<Edge, u64>,
    /// External bytes the node itself moved, which the three classes exclude
    /// on purpose so a VPN's payload is not counted twice. Reported so the
    /// traffic is visible somewhere rather than silently dropped.
    host_bytes: u64,
    samples: u64,
    entries: usize,
    parse_failures: u64,
}

impl Totals {
    fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# HELP kubarr_network_bytes_total Bytes observed by Cilium conntrack, by destination class.\n");
        out.push_str("# TYPE kubarr_network_bytes_total counter\n");
        for class in [Class::Internet, Class::Lan, Class::Cluster] {
            let value = self.bytes.get(class.label()).copied().unwrap_or(0);
            out.push_str(&format!(
                "kubarr_network_bytes_total{{class=\"{}\"}} {}\n",
                class.label(),
                value
            ));
        }
        out.push_str(
            "# HELP kubarr_network_edge_bytes_total Bytes per link, for the topology graph.\n",
        );
        out.push_str("# TYPE kubarr_network_edge_bytes_total counter\n");
        for (edge, value) in &self.edges {
            out.push_str(&format!(
                "kubarr_network_edge_bytes_total{{source=\"{}\",destination=\"{}\"}} {}\n",
                edge.source, edge.destination, value
            ));
        }
        out.push_str("# HELP kubarr_network_host_bytes_total External bytes the node moved itself, excluded from the classes to avoid counting a VPN payload twice.\n");
        out.push_str("# TYPE kubarr_network_host_bytes_total counter\n");
        out.push_str(&format!(
            "kubarr_network_host_bytes_total {}\n",
            self.host_bytes
        ));
        out.push_str(
            "# HELP kubarr_network_conntrack_entries Conntrack rows in the most recent dump.\n",
        );
        out.push_str("# TYPE kubarr_network_conntrack_entries gauge\n");
        out.push_str(&format!(
            "kubarr_network_conntrack_entries {}\n",
            self.entries
        ));
        out.push_str(
            "# HELP kubarr_network_samples_total Conntrack dumps folded into the counters.\n",
        );
        out.push_str("# TYPE kubarr_network_samples_total counter\n");
        out.push_str(&format!("kubarr_network_samples_total {}\n", self.samples));
        // A dump the sidecar wrote while Cilium was restarting parses as
        // nothing. Without this the counters would simply stop moving and look
        // like an idle network.
        out.push_str(
            "# HELP kubarr_network_parse_failures_total Dump lines that could not be read.\n",
        );
        out.push_str("# TYPE kubarr_network_parse_failures_total counter\n");
        out.push_str(&format!(
            "kubarr_network_parse_failures_total {}\n",
            self.parse_failures
        ));
        out
    }
}

/// Fold one dump into the running counters.
///
/// Conntrack entries are per connection and disappear when the connection
/// does, so the per-class totals have to be accumulated here: the map itself is
/// not a counter. An entry whose byte count went *down* was recycled for a new
/// connection between samples, and its current value is the best estimate of
/// what that new connection has moved.
fn fold(
    previous: &HashMap<Key, u64>,
    current: &HashMap<Key, u64>,
    topology: &Topology,
    totals: &mut Totals,
) {
    for (key, &now) in current {
        let before = previous.get(key).copied().unwrap_or(0);
        let delta = if now >= before { now - before } else { now };
        if delta == 0 {
            continue;
        }
        match topology.classify(key.source, key.destination) {
            Some(class) => *totals.bytes.entry(class.label()).or_insert(0) += delta,
            None => totals.host_bytes += delta,
        }
        if let Some(edge) = topology.edge(key.source, key.destination) {
            *totals.edges.entry(edge).or_insert(0) += delta;
        }
    }
    totals.entries = current.len();
    totals.samples += 1;
}

/// Read and parse a dump, returning the rows and how many lines were
/// unreadable. Parsing stays out of the lock: the dump is tens of thousands of
/// lines, and holding the metrics lock across that would stall every scrape.
fn read_dump(path: &str) -> std::io::Result<(HashMap<Key, u64>, u64)> {
    let text = std::fs::read_to_string(path)?;
    let mut rows = HashMap::new();
    let mut failures = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if is_service_row(line) {
            continue;
        }
        match parse_line(line) {
            Some((key, bytes)) => {
                // The forward and reverse rows land on the same key. They
                // normally agree exactly; taking the larger means a row read
                // mid-update cannot pull the total down.
                let entry = rows.entry(key).or_insert(0);
                *entry = (*entry).max(bytes);
            }
            None => failures += 1,
        }
    }
    Ok((rows, failures))
}

async fn serve(listen: &str, totals: Arc<Mutex<Totals>>) -> std::io::Result<()> {
    let listener = TcpListener::bind(listen).await?;
    eprintln!("flow exporter listening on {listen}");
    loop {
        let (mut socket, _) = listener.accept().await?;
        let totals = Arc::clone(&totals);
        tokio::spawn(async move {
            let mut discard = [0u8; 1024];
            let _ = socket.read(&mut discard).await;
            let body = match totals.lock() {
                Ok(totals) => totals.render(),
                // A poisoned lock means a fold panicked. Serving nothing is
                // better than serving a half-updated count as if it were real.
                Err(_) => String::new(),
            };
            let status = if body.is_empty() {
                "503 Service Unavailable"
            } else {
                "200 OK"
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let dump = env_or("KUBARR_CONNTRACK_DUMP", DEFAULT_DUMP);
    let listen = env_or("KUBARR_FLOW_EXPORTER_LISTEN", DEFAULT_LISTEN);
    let interval = env_or(
        "KUBARR_FLOW_EXPORTER_INTERVAL_SECONDS",
        &DEFAULT_INTERVAL_SECONDS.to_string(),
    )
    .parse::<u64>()
    .unwrap_or(DEFAULT_INTERVAL_SECONDS);
    let topology = Topology::from_env();

    eprintln!(
        "reading {dump} every {interval}s; local={} lan={}",
        topology.local.len(),
        topology.lan.len()
    );

    let totals = Arc::new(Mutex::new(Totals::default()));
    let server = Arc::clone(&totals);
    let listen_for_server = listen.clone();
    tokio::spawn(async move {
        if let Err(error) = serve(&listen_for_server, server).await {
            eprintln!("flow exporter listener stopped: {error}");
            std::process::exit(1);
        }
    });

    // The first dump is a baseline, not a measurement. Every connection in it
    // was opened before this process started, and folding it in would report
    // their entire lifetime's traffic as having happened in one interval --
    // gigabytes in fifteen seconds, every time the exporter restarts.
    let mut previous: Option<HashMap<Key, u64>> = None;
    loop {
        match read_dump(&dump) {
            Ok((current, failures)) => {
                // The lock is taken only to publish, and never across an await.
                {
                    let mut guard = match totals.lock() {
                        Ok(guard) => guard,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    guard.parse_failures += failures;
                    match previous.as_ref() {
                        Some(previous) => fold(previous, &current, &topology, &mut guard),
                        None => {
                            guard.entries = current.len();
                            guard.samples += 1;
                        }
                    }
                }
                previous = Some(current);
            }
            Err(error) => {
                // The sidecar has not written yet, or is mid-write. Keep the
                // counters where they are and try again.
                let _ = writeln!(std::io::stderr(), "conntrack dump unreadable: {error}");
            }
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topology() -> Topology {
        Topology {
            local: vec![
                Cidr::parse("10.244.0.0/16").expect("pod cidr"),
                Cidr::parse("192.168.1.20/32").expect("node"),
            ],
            pods: vec![Cidr::parse("10.244.0.0/16").expect("pod cidr")],
            lan: parse_cidrs("10.0.0.0/8,172.16.0.0/12,192.168.0.0/16"),
        }
    }

    fn addr(text: &str) -> Ipv4Addr {
        text.parse().expect("address")
    }

    #[test]
    fn pod_to_pod_is_cluster_traffic() {
        assert_eq!(
            topology().classify(addr("10.244.0.75"), addr("10.244.0.165")),
            Some(Class::Cluster)
        );
    }

    #[test]
    fn the_node_counts_as_cluster_not_lan() {
        // The node's address sits inside 192.168.0.0/16, so address order alone
        // would file every apiserver call as LAN traffic. Local wins first.
        assert_eq!(
            topology().classify(addr("10.244.0.144"), addr("192.168.1.20")),
            Some(Class::Cluster)
        );
    }

    #[test]
    fn a_private_address_that_is_not_ours_is_lan() {
        assert_eq!(
            topology().classify(addr("10.244.0.75"), addr("192.168.1.1")),
            Some(Class::Lan)
        );
    }

    #[test]
    fn anything_else_is_internet() {
        assert_eq!(
            topology().classify(addr("10.244.0.75"), addr("140.82.121.33")),
            Some(Class::Internet)
        );
    }

    #[test]
    fn the_nodes_own_external_traffic_is_not_one_of_the_classes() {
        // A VPN'd app produces two entries for one download: the pod's
        // connection to the peer and the node's tunnel to the VPN server.
        // Counting both reports the same payload twice -- measured at 26.5
        // MB/s from pods against 18.6 MB/s of the same data from the node.
        assert_eq!(
            topology().classify(addr("192.168.1.20"), addr("140.82.121.33")),
            None
        );
    }

    #[test]
    fn the_remote_end_decides_whichever_side_it_is_printed_on() {
        // Conntrack stores a tuple from the endpoint's point of view, so an
        // inbound row has the remote address first.
        let topology = topology();
        assert_eq!(
            topology.classify(addr("140.82.121.33"), addr("10.244.0.75")),
            Some(Class::Internet)
        );
        assert_eq!(
            topology.classify(addr("10.244.0.75"), addr("140.82.121.33")),
            Some(Class::Internet)
        );
    }

    #[test]
    fn parses_a_real_conntrack_line() {
        let line = "TCP OUT 10.244.0.75:37238 -> 10.244.0.165:8428 expires=13730 Packets=10 Bytes=1336 RxFlagsSeen=0x1b LastRxReport=13720 Flags=0x0013 RevNAT=0 SourceSecurityID=53735";
        let (key, bytes) = parse_line(line).expect("parses");
        assert_eq!(key.source, addr("10.244.0.75"));
        assert_eq!(key.destination_port, 8428);
        assert_eq!(bytes, 1336);
    }

    #[test]
    fn skips_a_line_with_no_byte_count() {
        // IPv6 rows and header lines both land here.
        assert!(parse_line("TCP OUT [fd00::1]:80 -> [fd00::2]:443 expires=1").is_none());
        assert!(parse_line("garbage").is_none());
    }

    #[test]
    fn an_edge_names_pods_by_address_and_everything_else_by_class() {
        let t = topology();
        // Pod to pod: both ends are addresses the backend can resolve.
        let edge = t
            .edge(addr("10.244.0.75"), addr("10.244.0.165"))
            .expect("pod to pod is an edge");
        assert_eq!(edge.source, "10.244.0.75");
        assert_eq!(edge.destination, "10.244.0.165");

        // Pod to the world: the far end collapses to its class, because a
        // torrent client talks to thousands of peers.
        let edge = t
            .edge(addr("10.244.0.75"), addr("140.82.121.33"))
            .expect("pod to internet is an edge");
        assert_eq!(edge.destination, "internet");

        let edge = t
            .edge(addr("10.244.0.75"), addr("192.168.1.1"))
            .expect("pod to lan is an edge");
        assert_eq!(edge.destination, "lan");

        // Node to the world has no Pod on it, so it is not drawn -- the same
        // rule that keeps a VPN payload from being counted twice.
        assert!(t
            .edge(addr("192.168.1.20"), addr("140.82.121.33"))
            .is_none());
    }

    #[test]
    fn edge_bytes_accumulate_alongside_the_class_totals() {
        let t = topology();
        let mut totals = Totals::default();
        let key = Key {
            source: addr("10.244.0.75"),
            source_port: 1000,
            destination: addr("10.244.0.165"),
            destination_port: 8428,
        };
        let first: HashMap<Key, u64> = [(key.clone(), 500)].into_iter().collect();
        let second: HashMap<Key, u64> = [(key, 1500)].into_iter().collect();
        fold(&first, &second, &t, &mut totals);

        assert_eq!(totals.bytes.get("cluster"), Some(&1000));
        assert_eq!(
            totals.edges.get(&Edge {
                source: "10.244.0.75".to_string(),
                destination: "10.244.0.165".to_string(),
            }),
            Some(&1000)
        );
    }

    #[test]
    fn the_forward_and_reverse_rows_of_one_connection_count_once() {
        // Cilium prints both, with the same totals. Counting each separately
        // doubled every figure on the page.
        let forward =
            "TCP OUT 10.244.0.7:37644 -> 10.244.0.8:8000 expires=1 Packets=1422 Bytes=4077709";
        let reverse =
            "TCP IN 10.244.0.7:37644 -> 10.244.0.8:8000 expires=1 Packets=1422 Bytes=4077709";
        let (fk, fb) = parse_line(forward).expect("forward parses");
        let (rk, rb) = parse_line(reverse).expect("reverse parses");

        assert_eq!(fk, rk, "both rows describe one connection");
        assert_eq!(fb, rb);

        let mut rows: HashMap<Key, u64> = HashMap::new();
        for (key, bytes) in [(fk, fb), (rk, rb)] {
            let entry = rows.entry(key).or_insert(0);
            *entry = (*entry).max(bytes);
        }
        assert_eq!(rows.len(), 1);
        assert_eq!(rows.values().sum::<u64>(), 4_077_709);
    }

    #[test]
    fn a_service_row_is_skipped_rather_than_counted() {
        // The load balancer's row and the backend's row describe the same
        // bytes. Counting both doubles every NodePort connection.
        let svc = "TCP SVC 192.168.1.1:7264 -> 192.168.1.20:30080 expires=18452 Packets=6 Bytes=2356 RevNAT=11";
        assert!(is_service_row(svc));
        assert!(parse_line(svc).is_none());

        let backend = "TCP OUT 192.168.1.1:40964 -> 10.244.0.7:8080 expires=18452 Packets=12 Bytes=4739 RevNAT=11";
        assert!(!is_service_row(backend));
        assert!(parse_line(backend).is_some());
    }

    #[test]
    fn counters_accumulate_across_samples() {
        let topology = topology();
        let mut totals = Totals::default();
        let key = Key {
            source: addr("10.244.0.75"),
            source_port: 1000,
            destination: addr("140.82.121.33"),
            destination_port: 443,
        };

        let first: HashMap<Key, u64> = [(key.clone(), 1000)].into_iter().collect();
        fold(&HashMap::new(), &first, &topology, &mut totals);
        assert_eq!(totals.bytes.get("internet"), Some(&1000));

        let second: HashMap<Key, u64> = [(key.clone(), 2500)].into_iter().collect();
        fold(&first, &second, &topology, &mut totals);
        assert_eq!(totals.bytes.get("internet"), Some(&2500));
    }

    #[test]
    fn a_recycled_entry_counts_from_zero_rather_than_underflowing() {
        // Conntrack reuses a slot for a new connection, so the byte count can
        // go down. Subtracting would underflow a u64 into nonsense.
        let topology = topology();
        let mut totals = Totals::default();
        let key = Key {
            source: addr("10.244.0.75"),
            source_port: 1000,
            destination: addr("140.82.121.33"),
            destination_port: 443,
        };
        let first: HashMap<Key, u64> = [(key.clone(), 9000)].into_iter().collect();
        let second: HashMap<Key, u64> = [(key.clone(), 120)].into_iter().collect();

        fold(&HashMap::new(), &first, &topology, &mut totals);
        fold(&first, &second, &topology, &mut totals);

        assert_eq!(totals.bytes.get("internet"), Some(&9120));
    }

    #[test]
    fn a_vanished_connection_keeps_what_it_moved() {
        // The entry is gone from the new dump, but the bytes it carried were
        // already folded in and must not be lost.
        let topology = topology();
        let mut totals = Totals::default();
        let key = Key {
            source: addr("10.244.0.75"),
            source_port: 1000,
            destination: addr("140.82.121.33"),
            destination_port: 443,
        };
        let first: HashMap<Key, u64> = [(key, 4096)].into_iter().collect();
        fold(&HashMap::new(), &first, &topology, &mut totals);
        fold(&first, &HashMap::new(), &topology, &mut totals);

        assert_eq!(totals.bytes.get("internet"), Some(&4096));
        assert_eq!(totals.entries, 0);
    }

    #[test]
    fn rendered_metrics_name_every_class_even_at_zero() {
        // A class that is missing from the output reads as "no data" in
        // PromQL, which is a different thing from "no traffic".
        let rendered = Totals::default().render();
        for class in ["internet", "lan", "cluster"] {
            assert!(
                rendered.contains(&format!(
                    "kubarr_network_bytes_total{{class=\"{class}\"}} 0"
                )),
                "{class} missing from:\n{rendered}"
            );
        }
    }
}
