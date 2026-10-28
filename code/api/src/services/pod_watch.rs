//! Keeps [`crate::services::pod_cache`] current from a Kubernetes pod watch.
//!
//! The worker holds one watch over all pods and writes each affected namespace
//! as events arrive. Compared with polling, there is no interval to tune and no
//! window in which the dashboard contradicts an action the user just took.
use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::runtime::watcher::{self, watcher, Event};
use kube::{Api, Client};
use sea_orm::DbConn;

use crate::services::k8s::pod_status_from_pod;
use crate::services::pod_cache::{self, CachedPod};

/// How often the watch reports in while the cluster is quiet.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

/// How long a reader will trust the cache after the last heartbeat.
///
/// Comfortably more than two heartbeats, so a single slow write does not make a
/// healthy watch look dead.
pub const MAX_STREAM_SILENCE: Duration = Duration::from_secs(45);

/// Folds watch events into per-namespace pod sets.
///
/// Kept separate from the I/O loop so the event semantics — especially the
/// relist handshake, where a missing pod means "gone" rather than "unchanged" —
/// can be tested without a cluster.
#[derive(Default)]
pub struct PodWatchState {
    observed: HashMap<String, HashMap<String, CachedPod>>,
    /// Pods accumulated since `Init`, applied wholesale on `InitDone`.
    relisting: Option<HashMap<String, HashMap<String, CachedPod>>>,
}

impl PodWatchState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one event and return the namespaces whose contents changed.
    pub fn apply(&mut self, event: Event<Pod>) -> Vec<String> {
        match event {
            Event::Init => {
                self.relisting = Some(HashMap::new());
                Vec::new()
            }
            Event::InitApply(pod) => {
                // Buffered: a half-applied relist would briefly report pods as
                // missing that are merely further down the stream.
                if let Some(buffer) = self.relisting.as_mut() {
                    insert_pod(buffer, pod);
                } else {
                    insert_pod(&mut self.observed, pod);
                }
                Vec::new()
            }
            Event::InitDone => {
                let Some(buffer) = self.relisting.take() else {
                    return Vec::new();
                };
                let mut next = buffer;
                // Namespaces that dropped out of the relist still need writing:
                // their pods are gone, and an untouched row would claim they
                // are still there.
                for namespace in self.observed.keys() {
                    next.entry(namespace.clone()).or_default();
                }
                self.observed = next;
                self.observed.keys().cloned().collect()
            }
            Event::Apply(pod) => {
                let namespace = pod_namespace(&pod);
                insert_pod(&mut self.observed, pod);
                namespace.into_iter().collect()
            }
            Event::Delete(pod) => {
                let Some(namespace) = pod_namespace(&pod) else {
                    return Vec::new();
                };
                let name = pod.metadata.name.clone().unwrap_or_default();
                // The namespace entry stays behind as an empty set so readers
                // can tell "no pods" from "never observed".
                self.observed
                    .entry(namespace.clone())
                    .or_default()
                    .remove(&name);
                vec![namespace]
            }
        }
    }

    /// Pods currently recorded for a namespace.
    pub fn namespace(&self, namespace: &str) -> Vec<CachedPod> {
        self.observed
            .get(namespace)
            .map(|pods| pods.values().cloned().collect())
            .unwrap_or_default()
    }
}

fn pod_namespace(pod: &Pod) -> Option<String> {
    pod.metadata.namespace.clone()
}

fn insert_pod(target: &mut HashMap<String, HashMap<String, CachedPod>>, pod: Pod) {
    let Some(namespace) = pod_namespace(&pod) else {
        return;
    };
    let Some(name) = pod.metadata.name.clone() else {
        return;
    };
    let created_unix = pod
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|time| time.0.as_second());
    let status = pod_status_from_pod(pod, &namespace);
    target.entry(namespace).or_default().insert(
        name,
        CachedPod {
            pod: status,
            created_unix,
        },
    );
}

/// Run the pod watch until the process ends.
///
/// `kube`'s watcher reconnects and relists on its own; this loop's job is to
/// write what it reports and to stop vouching for the cache while the stream is
/// broken, so readers fall back to live queries.
pub async fn run(client: Client, db: DbConn) {
    let api: Api<Pod> = Api::all(client);
    let mut stream = watcher(api, watcher::Config::default()).boxed();
    let mut state = PodWatchState::new();
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut healthy = false;

    loop {
        tokio::select! {
            event = stream.next() => {
                let Some(event) = event else {
                    tracing::warn!("pod watch stream ended, pod cache will go stale");
                    return;
                };
                match event {
                    Ok(event) => {
                        let synced = matches!(event, Event::InitDone);
                        for namespace in state.apply(event) {
                            let pods = state.namespace(&namespace);
                            if let Err(error) = pod_cache::store_namespace(&db, &namespace, &pods).await {
                                tracing::warn!(%error, %namespace, "failed to record pod status");
                            }
                        }
                        if synced {
                            healthy = true;
                        }
                        if healthy {
                            touch(&db).await;
                        }
                    }
                    Err(error) => {
                        // The watcher will retry. Until it resyncs, withhold the
                        // heartbeat so readers stop trusting these rows.
                        tracing::warn!(%error, "pod watch error, pausing pod cache heartbeat");
                        healthy = false;
                    }
                }
            }
            _ = heartbeat.tick() => {
                // A quiet cluster produces no events; without this the cache
                // would expire while it is still perfectly accurate.
                if healthy {
                    touch(&db).await;
                }
            }
        }
    }
}

async fn touch(db: &DbConn) {
    if let Err(error) = pod_cache::touch_heartbeat(db).await {
        tracing::warn!(%error, "failed to record pod watch heartbeat");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    fn pod(name: &str, namespace: &str) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn records_an_applied_pod_against_its_namespace() {
        let mut state = PodWatchState::new();

        let dirty = state.apply(Event::Apply(pod("sonarr-0", "sonarr")));

        assert_eq!(dirty, vec!["sonarr".to_string()]);
        assert_eq!(state.namespace("sonarr").len(), 1);
    }

    #[test]
    fn a_deleted_pod_leaves_its_namespace_known_but_empty() {
        let mut state = PodWatchState::new();
        state.apply(Event::Apply(pod("sonarr-0", "sonarr")));

        let dirty = state.apply(Event::Delete(pod("sonarr-0", "sonarr")));

        // The namespace must still be written, as an empty set, or its row
        // would keep advertising a pod that no longer exists.
        assert_eq!(dirty, vec!["sonarr".to_string()]);
        assert!(state.namespace("sonarr").is_empty());
    }

    #[test]
    fn a_relist_is_invisible_until_it_completes() {
        let mut state = PodWatchState::new();
        state.apply(Event::Apply(pod("sonarr-0", "sonarr")));

        assert!(state.apply(Event::Init).is_empty());
        assert!(state
            .apply(Event::InitApply(pod("sonarr-0", "sonarr")))
            .is_empty());

        // Mid-relist the old view still stands; nothing has been written yet.
        assert_eq!(state.namespace("sonarr").len(), 1);
    }

    #[test]
    fn a_relist_drops_pods_the_cluster_no_longer_has() {
        let mut state = PodWatchState::new();
        state.apply(Event::Apply(pod("sonarr-0", "sonarr")));
        state.apply(Event::Apply(pod("radarr-0", "radarr")));

        state.apply(Event::Init);
        state.apply(Event::InitApply(pod("sonarr-0", "sonarr")));
        let mut dirty = state.apply(Event::InitDone);
        dirty.sort();

        // radarr vanished while the watch was disconnected: it must be rewritten
        // as empty, not left untouched.
        assert_eq!(dirty, vec!["radarr".to_string(), "sonarr".to_string()]);
        assert_eq!(state.namespace("sonarr").len(), 1);
        assert!(state.namespace("radarr").is_empty());
    }

    #[test]
    fn captures_the_creation_time_needed_to_age_a_pod() {
        let mut state = PodWatchState::new();
        let mut started = pod("sonarr-0", "sonarr");
        started.metadata.creation_timestamp =
            Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(
                jiff::Timestamp::from_second(1_700_000_000).expect("timestamp"),
            ));

        state.apply(Event::Apply(started));

        assert_eq!(
            state.namespace("sonarr")[0].created_unix,
            Some(1_700_000_000)
        );
    }
}
