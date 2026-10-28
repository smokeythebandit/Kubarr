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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_namespace_is_cluster_traffic() {
        assert_eq!(
            classify_destination("kubarr-backend"),
            Some(FlowClass::Cluster)
        );
        assert_eq!(
            classify_destination("victoriametrics"),
            Some(FlowClass::Cluster)
        );
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
        assert_eq!(
            classify_destination("reserved:remote-node"),
            Some(FlowClass::Cluster)
        );
        assert_eq!(
            classify_destination("reserved:ingress"),
            Some(FlowClass::Cluster)
        );
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
        assert_eq!(
            classify_destination("reserved:world"),
            Some(FlowClass::Internet)
        );
        assert_eq!(
            classify_destination("reserved:world-ipv4"),
            Some(FlowClass::Internet)
        );
        assert_eq!(
            classify_destination("reserved:world-ipv6"),
            Some(FlowClass::Internet)
        );
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
