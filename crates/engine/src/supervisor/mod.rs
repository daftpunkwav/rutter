//! Process supervision: launch, heartbeat, restart, circuit breaker.
//!
//! Responsibilities:
//! - Launch an engine through an [`EngineLauncher`] and keep it alive.
//! - Probe health on a heartbeat; on death, relaunch under the
//!   [`RestartPolicy`] (capped backoff, window breaker).
//!
//! Boundary: process lifecycle only. Session state restoration is a
//! session-layer concern (blueprint §7.4) and is not attempted here; a
//! restart yields a fresh, empty engine. While the breaker is open or a
//! restart is in flight, [`Supervisor::engine`] reports
//! [`EngineError::Terminated`] — callers fail their affected operations.
//! The heartbeat keeps retrying after breaker windows drain, so a failed
//! or dead engine recovers on its own, and rutter itself never crashes
//! on engine death.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::config::LaunchMode;
use crate::engine::Engine;
use crate::error::EngineError;
use crate::health::HealthReport;
use crate::supervisor::policy::{RestartDecision, RestartHistory, RestartPolicy};

pub mod policy;

/// How a factory produces engine instances; the engine registration
/// point (blueprint §8.1). Backends plug in by implementing this trait;
/// the binary selects one at startup.
#[async_trait]
pub trait EngineLauncher: Send + Sync {
    /// Human-readable backend identity for diagnostics.
    fn describe(&self) -> String;

    /// Launches one engine instance in the given mode.
    async fn launch(&self, mode: LaunchMode) -> Result<Arc<dyn Engine>, EngineError>;
}

/// Interval between supervised health probes.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

/// Owns the current engine instance and its restart bookkeeping.
struct Supervised {
    engine: RwLock<Option<Arc<dyn Engine>>>,
    history: Mutex<RestartHistory>,
    /// Serializes restart cycles so concurrent failures relaunch once.
    restarting: Mutex<()>,
}

impl Supervised {
    /// Removes the current engine, leaving the slot empty for a
    /// relaunch. The heartbeat call site has already confirmed the
    /// instance is dead; dropping the last handle lets the child be
    /// reaped before the relaunch loop starts.
    async fn clear_engine(&self) {
        *self.engine.write().await = None;
    }
}

/// Keeps one engine running: initial launch, heartbeat probes, and
/// policy-driven restarts.
pub struct Supervisor {
    launcher: Arc<dyn EngineLauncher>,
    mode: LaunchMode,
    policy: RestartPolicy,
    heartbeat_interval: Duration,
    supervised: Arc<Supervised>,
    heartbeat: Mutex<Option<JoinHandle<()>>>,
    /// Bumped every time a dead engine is successfully replaced;
    /// recovery consumers watch this (blueprint §7.4).
    restarts: tokio::sync::watch::Sender<u64>,
}

impl Supervisor {
    /// Creates a supervisor for a launcher, production defaults.
    pub fn new(launcher: Arc<dyn EngineLauncher>, mode: LaunchMode) -> Self {
        let (restarts, _) = tokio::sync::watch::channel(0);
        Self {
            launcher,
            mode,
            policy: RestartPolicy::new(5, Duration::from_secs(60)),
            heartbeat_interval: HEARTBEAT_INTERVAL,
            supervised: Arc::new(Supervised {
                engine: RwLock::new(None),
                history: Mutex::new(RestartHistory::default()),
                restarting: Mutex::new(()),
            }),
            heartbeat: Mutex::new(None),
            restarts,
        }
    }

    /// Watcher for engine replacements; the count increments on every
    /// successful restart of a dead engine.
    pub fn restart_watcher(&self) -> tokio::sync::watch::Receiver<u64> {
        self.restarts.subscribe()
    }

    /// Overrides the heartbeat cadence; tests use a short interval.
    pub fn with_heartbeat(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = interval;
        self
    }

    /// Performs the initial launch and starts the heartbeat loop.
    ///
    /// The heartbeat starts even when the initial launch exhausts the
    /// breaker: once the window drains, the loop retries and the
    /// supervisor recovers on its own. A repeated `start` is a no-op.
    pub async fn start(&self) -> Result<(), EngineError> {
        let mut heartbeat_slot = self.heartbeat.lock().await;
        if heartbeat_slot.is_some() {
            return Ok(());
        }

        let result = self.launch_with_policy().await;
        if let Ok(engine) = &result {
            *self.supervised.engine.write().await = Some(Arc::clone(engine));
        }

        let supervised = Arc::clone(&self.supervised);
        let launcher = Arc::clone(&self.launcher);
        let policy = self.policy.clone();
        let interval = self.heartbeat_interval;
        let mode = self.mode;
        let restarts = self.restarts.clone();
        *heartbeat_slot = Some(tokio::spawn(async move {
            heartbeat_loop(supervised, launcher, mode, policy, interval, restarts).await;
        }));
        result.map(|_| ())
    }

    /// Returns the current engine, or `Terminated` while dead, restarting,
    /// or breaker-open. Callers treat this as an engine-level failure of
    /// the operation they were about to perform.
    pub async fn engine(&self) -> Result<Arc<dyn Engine>, EngineError> {
        self.supervised
            .engine
            .read()
            .await
            .clone()
            .ok_or(EngineError::Terminated)
    }

    /// Stops the heartbeat, shuts the engine down, and clears it.
    pub async fn shutdown(&self) {
        if let Some(handle) = self.heartbeat.lock().await.take() {
            handle.abort();
        }
        if let Some(engine) = self.supervised.engine.write().await.take()
            && let Err(error) = engine.shutdown().await
        {
            eprintln!("rutter: engine shutdown failed: {error}");
        }
    }

    /// Launch attempts under backoff and breaker; used for the initial
    /// start and every restart. Breaker-open surfaces as `Terminated`.
    async fn launch_with_policy(&self) -> Result<Arc<dyn Engine>, EngineError> {
        let _guard = self.supervised.restarting.lock().await;
        // Another restart cycle may have completed while we waited.
        if let Some(engine) = self.supervised.engine.read().await.clone() {
            return Ok(engine);
        }

        loop {
            let decision = {
                let mut history = self.supervised.history.lock().await;
                self.policy.decide(&mut history, Instant::now())
            };
            match decision {
                RestartDecision::Open => return Err(EngineError::Terminated),
                RestartDecision::Allowed(delay) => tokio::time::sleep(delay).await,
            }

            {
                let mut history = self.supervised.history.lock().await;
                history.record(Instant::now());
            }
            match self.launcher.launch(self.mode).await {
                Ok(engine) => return Ok(engine),
                Err(error) => {
                    eprintln!(
                        "rutter: {} launch failed, retrying under policy: {error}",
                        self.launcher.describe()
                    );
                }
            }
        }
    }
}

/// Heartbeat: probe health, and on failure swap in a fresh engine.
/// Loop exit happens via task abort in [`Supervisor::shutdown`].
async fn heartbeat_loop(
    supervised: Arc<Supervised>,
    launcher: Arc<dyn EngineLauncher>,
    mode: LaunchMode,
    policy: RestartPolicy,
    interval: Duration,
    restarts: tokio::sync::watch::Sender<u64>,
) {
    loop {
        tokio::time::sleep(interval).await;

        let current = supervised.engine.read().await.clone();
        let healthy = match current {
            None => false,
            Some(engine) => match engine.health().await {
                Ok(HealthReport { healthy: true, .. }) => true,
                Ok(HealthReport {
                    healthy: false,
                    detail,
                    ..
                }) => {
                    eprintln!(
                        "rutter: engine unhealthy, restarting ({})",
                        detail.unwrap_or_else(|| "no detail".to_owned())
                    );
                    false
                }
                Err(error) => {
                    eprintln!("rutter: engine health probe failed: {error}");
                    false
                }
            },
        };
        if healthy {
            continue;
        }

        let _guard = supervised.restarting.lock().await;
        // A concurrent restart may have replaced the engine already.
        if let Some(engine) = supervised.engine.read().await.clone()
            && matches!(
                engine.health().await,
                Ok(HealthReport { healthy: true, .. })
            )
        {
            continue;
        }
        supervised.clear_engine().await;

        loop {
            let decision = {
                let mut history = supervised.history.lock().await;
                policy.decide(&mut history, Instant::now())
            };
            match decision {
                RestartDecision::Open => {
                    eprintln!(
                        "rutter: restart breaker open for {}; affected operations fail until the window drains",
                        launcher.describe()
                    );
                    break;
                }
                RestartDecision::Allowed(delay) => tokio::time::sleep(delay).await,
            }

            {
                let mut history = supervised.history.lock().await;
                history.record(Instant::now());
            }
            match launcher.launch(mode).await {
                Ok(engine) => {
                    *supervised.engine.write().await = Some(engine);
                    restarts.send_modify(|count| *count += 1);
                    break;
                }
                Err(error) => {
                    eprintln!(
                        "rutter: {} restart failed, retrying under policy: {error}",
                        launcher.describe()
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
