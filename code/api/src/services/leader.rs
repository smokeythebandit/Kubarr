//! Single-active-worker election using a Kubernetes `Lease`.
//!
//! Every worker replica calls [`LeaderElector::acquire`] at startup; only the
//! replica holding the Lease runs any loop. The holder renews in the background
//! and signals [`LeaderGuard::lost`] if it can no longer prove it is the leader.

use anyhow::{anyhow, Result};
use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{MicroTime, ObjectMeta};
use kube::api::{Api, PostParams};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub const LEASE_NAME: &str = "kubarr-worker-leader";
pub const LEASE_DURATION: Duration = Duration::from_secs(15);
pub const RENEW_EVERY: Duration = Duration::from_secs(5);
/// Must stay below LEASE_DURATION so we give up before a standby can take over.
pub const RENEW_DEADLINE: Duration = Duration::from_secs(10);
const DEFAULT_NAMESPACE: &str = "kubarr-worker";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseAction {
    Acquire,
    Renew,
    Wait,
}

/// What this lease looked like when last seen, and since when (local clock).
/// Expiry is measured on the observer's own monotonic clock so clock skew
/// between nodes cannot make a standby steal a live lease.
pub struct LeaseObserver {
    seen: Option<(Option<String>, Option<MicroTime>, Option<String>)>,
    since: Instant,
}

impl LeaseObserver {
    pub fn new() -> Self {
        Self {
            seen: None,
            since: Instant::now(),
        }
    }

    /// Record `lease` and return how long it has been unchanged locally.
    pub fn unchanged_for(&mut self, lease: Option<&Lease>) -> Duration {
        let key = lease.map(|l| {
            let spec = l.spec.as_ref();
            (
                spec.and_then(|s| s.holder_identity.clone()),
                spec.and_then(|s| s.renew_time.clone()),
                l.metadata.resource_version.clone(),
            )
        });
        if self.seen != key {
            self.seen = key;
            self.since = Instant::now();
        }
        self.since.elapsed()
    }
}

impl Default for LeaseObserver {
    fn default() -> Self {
        Self::new()
    }
}

/// Decide what this candidate should do. `unchanged_for` is how long the lease
/// has been observed without any change (see [`LeaseObserver`]).
pub fn decide(
    spec: Option<&LeaseSpec>,
    me: &str,
    unchanged_for: Duration,
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
    let holder_duration = spec
        .lease_duration_seconds
        .map(|s| Duration::from_secs(s.max(0) as u64))
        .unwrap_or(duration);
    if unchanged_for >= holder_duration {
        LeaseAction::Acquire
    } else {
        LeaseAction::Wait
    }
}

/// Keep renewing until `stop` fires. Cancels `lost` when a renew has not
/// succeeded within `renew_deadline`, even if a renew call is still hanging
/// (the call is dropped). `renew` returns Ok(false) if another holder took over.
async fn renew_until_lost<F, Fut>(
    mut renew: F,
    renew_every: Duration,
    renew_deadline: Duration,
    stop: CancellationToken,
    lost: CancellationToken,
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool>>,
{
    let mut last_ok = tokio::time::Instant::now();
    loop {
        let deadline = last_ok + renew_deadline;
        let attempt = async {
            tokio::time::sleep(renew_every).await;
            let started = tokio::time::Instant::now();
            (started, renew().await)
        };
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = tokio::time::sleep_until(deadline) => {
                tracing::error!("Worker leadership could not be renewed in time");
                lost.cancel();
                return;
            }
            (started, result) = attempt => match result {
                Ok(true) => last_ok = started,
                Ok(false) => {
                    tracing::error!("Worker leadership taken over by another holder");
                    lost.cancel();
                    return;
                }
                Err(error) => tracing::warn!(%error, "Failed to renew worker leadership"),
            },
        }
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
    async fn try_acquire(&self, observer: &mut LeaseObserver) -> Result<bool> {
        let api = self.api();
        let existing = api.get_opt(&self.name).await?;
        let spec = existing.as_ref().and_then(|l| l.spec.as_ref());
        let unchanged_for = observer.unchanged_for(existing.as_ref());
        match decide(spec, &self.identity, unchanged_for, self.duration) {
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
        let mut observer = LeaseObserver::new();
        loop {
            if cancel.is_cancelled() {
                return Err(anyhow!("cancelled while waiting for worker leadership"));
            }
            match self.try_acquire(&mut observer).await {
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
        let task = tokio::spawn(renew_until_lost(
            {
                let elector = self.clone();
                // While we hold the lease the decision is always Renew (or Wait if
                // someone took over), so a fresh observer per attempt is enough.
                move || {
                    let elector = elector.clone();
                    async move { elector.try_acquire(&mut LeaseObserver::new()).await }
                }
            },
            self.renew_every,
            RENEW_DEADLINE,
            stop.clone(),
            lost.clone(),
        ));
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
        let result: Result<()> = tokio::time::timeout(RENEW_EVERY, async {
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
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("timed out")));
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

    fn spec(holder: Option<&str>) -> LeaseSpec {
        LeaseSpec {
            holder_identity: holder.map(str::to_string),
            lease_duration_seconds: Some(15),
            renew_time: Some(MicroTime(jiff::Timestamp::now())),
            ..Default::default()
        }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn decide_acquires_missing_lease() {
        assert_eq!(decide(None, "me", secs(0), DURATION), LeaseAction::Acquire);
    }

    #[test]
    fn decide_acquires_expired_foreign_lease() {
        let s = spec(Some("other"));
        assert_eq!(
            decide(Some(&s), "me", secs(20), DURATION),
            LeaseAction::Acquire
        );
    }

    #[test]
    fn decide_waits_on_live_foreign_lease() {
        let s = spec(Some("other"));
        assert_eq!(decide(Some(&s), "me", secs(5), DURATION), LeaseAction::Wait);
    }

    #[test]
    fn decide_renews_own_lease() {
        let s = spec(Some("me"));
        assert_eq!(
            decide(Some(&s), "me", secs(0), DURATION),
            LeaseAction::Renew
        );
    }

    #[test]
    fn decide_acquires_released_lease() {
        let s = spec(None);
        assert_eq!(
            decide(Some(&s), "me", secs(0), DURATION),
            LeaseAction::Acquire
        );
    }

    #[test]
    fn decide_acquires_empty_holder() {
        let s = spec(Some(""));
        assert_eq!(
            decide(Some(&s), "me", secs(0), DURATION),
            LeaseAction::Acquire
        );
    }

    #[test]
    fn decide_missing_renew_time_still_waits_for_local_expiry() {
        let mut s = spec(Some("other"));
        s.renew_time = None;
        assert_eq!(decide(Some(&s), "me", secs(5), DURATION), LeaseAction::Wait);
        assert_eq!(
            decide(Some(&s), "me", secs(15), DURATION),
            LeaseAction::Acquire
        );
    }

    fn lease(holder: &str, rv: &str) -> Lease {
        Lease {
            metadata: ObjectMeta {
                resource_version: Some(rv.into()),
                ..Default::default()
            },
            spec: Some(LeaseSpec {
                holder_identity: Some(holder.into()),
                ..Default::default()
            }),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn observer_resets_on_change_and_grows_when_unchanged() {
        let mut o = LeaseObserver::new();
        let l = lease("other", "1");
        o.unchanged_for(Some(&l));
        tokio::time::advance(secs(10)).await;
        assert!(o.unchanged_for(Some(&l)) >= secs(10));
        assert!(o.unchanged_for(Some(&lease("other", "2"))) < secs(1));
    }

    async fn run_renewer<F, Fut>(renew: F) -> (CancellationToken, CancellationToken)
    where
        F: FnMut() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<bool>> + Send + 'static,
    {
        let stop = CancellationToken::new();
        let lost = CancellationToken::new();
        tokio::spawn(renew_until_lost(
            renew,
            RENEW_EVERY,
            RENEW_DEADLINE,
            stop.clone(),
            lost.clone(),
        ));
        (stop, lost)
    }

    #[tokio::test(start_paused = true)]
    async fn hung_renew_call_loses_leadership_by_deadline() {
        let (stop, lost) =
            run_renewer(|| async { std::future::pending::<Result<bool>>().await }).await;
        tokio::time::sleep(RENEW_DEADLINE + secs(1)).await;
        assert!(lost.is_cancelled());
        stop.cancel();
    }

    #[tokio::test(start_paused = true)]
    async fn failing_renew_calls_lose_leadership_by_deadline() {
        let (stop, lost) = run_renewer(|| async { Err(anyhow!("api down")) }).await;
        tokio::time::sleep(secs(4)).await;
        assert!(!lost.is_cancelled());
        tokio::time::sleep(RENEW_DEADLINE).await;
        assert!(lost.is_cancelled());
        stop.cancel();
    }

    #[tokio::test(start_paused = true)]
    async fn succeeding_renew_never_loses_leadership() {
        let (stop, lost) = run_renewer(|| async { Ok(true) }).await;
        tokio::time::sleep(secs(300)).await;
        assert!(!lost.is_cancelled());
        stop.cancel();
    }

    #[tokio::test(start_paused = true)]
    async fn takeover_loses_leadership_immediately() {
        let (stop, lost) = run_renewer(|| async { Ok(false) }).await;
        tokio::time::sleep(RENEW_EVERY + secs(1)).await;
        assert!(lost.is_cancelled());
        stop.cancel();
    }
}
