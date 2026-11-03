//! Single-active-worker election using a Kubernetes `Lease`.
//!
//! Every worker replica calls [`LeaderElector::acquire`] at startup; only the
//! replica holding the Lease runs any loop. The holder renews in the background
//! and signals [`LeaderGuard::lost`] if it can no longer prove it is the leader.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{MicroTime, ObjectMeta};
use kube::api::{Api, PostParams};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub const LEASE_NAME: &str = "kubarr-worker-leader";
pub const LEASE_DURATION: Duration = Duration::from_secs(15);
pub const RENEW_EVERY: Duration = Duration::from_secs(5);
const DEFAULT_NAMESPACE: &str = "kubarr-worker";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseAction {
    Acquire,
    Renew,
    Wait,
}

/// Decide what this candidate should do given the observed lease spec.
pub fn decide(
    spec: Option<&LeaseSpec>,
    me: &str,
    now: DateTime<Utc>,
    duration: Duration,
) -> LeaseAction {
    let Some(spec) = spec else {
        return LeaseAction::Acquire;
    };
    let Some(holder) = spec.holder_identity.as_deref().filter(|h| !h.is_empty()) else {
        return LeaseAction::Acquire;
    };
    if holder == me {
        return LeaseAction::Renew;
    }
    let duration_secs = spec
        .lease_duration_seconds
        .map(|s| i64::from(s.max(0)))
        .unwrap_or(duration.as_secs() as i64);
    let expired = match spec.renew_time.as_ref().or(spec.acquire_time.as_ref()) {
        Some(MicroTime(renewed)) => {
            let renewed_secs = renewed.as_second();
            renewed_secs + duration_secs < now.timestamp()
        }
        None => true,
    };
    if expired {
        LeaseAction::Acquire
    } else {
        LeaseAction::Wait
    }
}

fn micro_now() -> MicroTime {
    MicroTime(jiff::Timestamp::now())
}

#[derive(Clone)]
pub struct LeaderElector {
    client: kube::Client,
    namespace: String,
    name: String,
    identity: String,
    duration: Duration,
    renew_every: Duration,
}

impl LeaderElector {
    pub fn new(client: kube::Client) -> Self {
        let identity = std::env::var("POD_NAME")
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| std::env::var("HOSTNAME").ok().filter(|v| !v.is_empty()))
            .unwrap_or_else(|| format!("kubarr-worker-{}", std::process::id()));
        let namespace = std::env::var("POD_NAMESPACE")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_NAMESPACE.to_string());
        Self {
            client,
            namespace,
            name: LEASE_NAME.to_string(),
            identity,
            duration: LEASE_DURATION,
            renew_every: RENEW_EVERY,
        }
    }

    fn api(&self) -> Api<Lease> {
        Api::namespaced(self.client.clone(), &self.namespace)
    }

    /// One acquisition attempt. Ok(true) when we now hold the lease.
    async fn try_acquire(&self) -> Result<bool> {
        let api = self.api();
        let existing = api.get_opt(&self.name).await?;
        let spec = existing.as_ref().and_then(|l| l.spec.as_ref());
        match decide(spec, &self.identity, Utc::now(), self.duration) {
            LeaseAction::Wait => Ok(false),
            action => {
                let mut spec = spec.cloned().unwrap_or_default();
                let same_holder = action == LeaseAction::Renew;
                if !same_holder {
                    spec.acquire_time = Some(micro_now());
                    spec.lease_transitions = Some(spec.lease_transitions.unwrap_or(0) + 1);
                }
                spec.holder_identity = Some(self.identity.clone());
                spec.lease_duration_seconds = Some(self.duration.as_secs() as i32);
                spec.renew_time = Some(micro_now());
                match existing {
                    None => {
                        let lease = Lease {
                            metadata: ObjectMeta {
                                name: Some(self.name.clone()),
                                namespace: Some(self.namespace.clone()),
                                ..Default::default()
                            },
                            spec: Some(spec),
                        };
                        api.create(&PostParams::default(), &lease).await?;
                    }
                    Some(mut lease) => {
                        lease.spec = Some(spec);
                        // resourceVersion stays set: a concurrent writer makes this 409.
                        api.replace(&self.name, &PostParams::default(), &lease)
                            .await?;
                    }
                }
                Ok(true)
            }
        }
    }

    /// Block until this replica holds the lease, or `cancel` fires first.
    pub async fn acquire(&self, cancel: &CancellationToken) -> Result<LeaderGuard> {
        loop {
            if cancel.is_cancelled() {
                return Err(anyhow!("cancelled while waiting for worker leadership"));
            }
            match self.try_acquire().await {
                Ok(true) => {
                    tracing::info!(identity = %self.identity, lease = %self.name, "Acquired worker leadership");
                    return Ok(self.start_renewing());
                }
                Ok(false) => {
                    tracing::debug!(identity = %self.identity, "Worker leader is active elsewhere; standing by")
                }
                Err(error) => tracing::warn!(%error, "Leader election attempt failed"),
            }
            tokio::select! {
                _ = cancel.cancelled() => {
                    return Err(anyhow!("cancelled while waiting for worker leadership"));
                }
                _ = tokio::time::sleep(self.renew_every) => {}
            }
        }
    }

    fn start_renewing(&self) -> LeaderGuard {
        let lost = CancellationToken::new();
        let stop = CancellationToken::new();
        let elector = self.clone();
        let task = tokio::spawn({
            let lost = lost.clone();
            let stop = stop.clone();
            async move {
                let mut last_ok = Instant::now();
                loop {
                    tokio::select! {
                        _ = stop.cancelled() => return,
                        _ = tokio::time::sleep(elector.renew_every) => {}
                    }
                    match elector.try_acquire().await {
                        Ok(true) => last_ok = Instant::now(),
                        Ok(false) => {
                            tracing::error!("Worker leadership taken over by another holder");
                            lost.cancel();
                            return;
                        }
                        Err(error) => {
                            tracing::warn!(%error, "Failed to renew worker leadership");
                            if last_ok.elapsed() >= elector.duration {
                                tracing::error!(
                                    "Worker leadership lease could not be renewed in time"
                                );
                                lost.cancel();
                                return;
                            }
                        }
                    }
                }
            }
        });
        LeaderGuard {
            lost,
            stop,
            task,
            elector: self.clone(),
        }
    }
}

pub struct LeaderGuard {
    /// Cancelled when leadership can no longer be assumed.
    pub lost: CancellationToken,
    stop: CancellationToken,
    task: JoinHandle<()>,
    elector: LeaderElector,
}

impl LeaderGuard {
    /// Stop renewing and hand the lease over immediately if it is still ours.
    pub async fn release(self) {
        self.stop.cancel();
        let _ = self.task.await;
        let api = self.elector.api();
        let result: Result<()> = async {
            if let Some(mut lease) = api.get_opt(&self.elector.name).await? {
                let ours = lease
                    .spec
                    .as_ref()
                    .and_then(|s| s.holder_identity.as_deref())
                    == Some(self.elector.identity.as_str());
                if ours {
                    if let Some(spec) = lease.spec.as_mut() {
                        spec.holder_identity = None;
                    }
                    api.replace(&self.elector.name, &PostParams::default(), &lease)
                        .await?;
                }
            }
            Ok(())
        }
        .await;
        match result {
            Ok(()) => tracing::info!("Released worker leadership"),
            Err(error) => tracing::warn!(%error, "Failed to release worker leadership"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DURATION: Duration = Duration::from_secs(15);

    fn spec(holder: Option<&str>, renewed_ago_secs: i64, now: DateTime<Utc>) -> LeaseSpec {
        LeaseSpec {
            holder_identity: holder.map(str::to_string),
            lease_duration_seconds: Some(15),
            renew_time: Some(MicroTime(
                jiff::Timestamp::from_second(now.timestamp() - renewed_ago_secs).unwrap(),
            )),
            ..Default::default()
        }
    }

    #[test]
    fn decide_acquires_missing_lease() {
        assert_eq!(
            decide(None, "me", Utc::now(), DURATION),
            LeaseAction::Acquire
        );
    }

    #[test]
    fn decide_acquires_expired_foreign_lease() {
        let now = Utc::now();
        let s = spec(Some("other"), 20, now);
        assert_eq!(decide(Some(&s), "me", now, DURATION), LeaseAction::Acquire);
    }

    #[test]
    fn decide_waits_on_live_foreign_lease() {
        let now = Utc::now();
        let s = spec(Some("other"), 5, now);
        assert_eq!(decide(Some(&s), "me", now, DURATION), LeaseAction::Wait);
    }

    #[test]
    fn decide_renews_own_lease() {
        let now = Utc::now();
        let s = spec(Some("me"), 5, now);
        assert_eq!(decide(Some(&s), "me", now, DURATION), LeaseAction::Renew);
    }

    #[test]
    fn decide_acquires_released_lease() {
        let now = Utc::now();
        let s = spec(None, 1, now);
        assert_eq!(decide(Some(&s), "me", now, DURATION), LeaseAction::Acquire);
    }
}
