//! Process supervision: launch, heartbeat, restart, circuit breaker.
//!
//! Responsibilities:
//! - Launch an engine through an [`EngineLauncher`] and keep it alive.
//! - Probe health on a heartbeat; on death, relaunch under the
//!   [`RestartPolicy`] (capped backoff, window breaker).
//!
//! Boundary: process lifecycle only. Session state restoration is a
//! session-layer concern (docs/sessions.md) and is not attempted here; a
//! restart yields a fresh, empty engine. While the breaker is open or a
//! replacement is in flight, [`Supervisor::engine`] reports
//! [`EngineError::Terminated`] — callers fail their affected operations.
//! The heartbeat keeps retrying after breaker windows drain, so a failed
//! or dead engine recovers on its own, and rutter itself never crashes on
//! engine death.
//!
//! Two shapes matter here. The supervision state is one explicit
//! `Phase` value rather than an `Option` slot plus a restart flag plus a
//! timestamp history that callers had to read together, and the restart
//! policy loop exists once — `bring_up` — which the initial start and
//! every later replacement both go through. It used to exist twice, with
//! the two copies free to drift.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;

use crate::config::LaunchMode;
use crate::engine::Engine;
use crate::error::EngineError;
use crate::health::HealthReport;
use crate::supervisor::policy::{RestartDecision, RestartHistory};

pub mod policy;

pub use policy::RestartPolicy;

/// How a factory produces engine instances; the engine registration
/// point (docs/architecture.md). Backends plug in by implementing this trait;
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

/// Where the supervised engine stands.
///
/// The restart record rides with the phase because it means something
/// different in each one: attempts spent reaching the running instance, or
/// attempts spent while no instance exists.
enum Phase {
    /// No engine: before the first launch, or after a shutdown.
    Idle,
    /// A live engine, reached after the recorded attempts.
    Running {
        /// The live instance.
        engine: Arc<dyn Engine>,
        /// Launch attempts still inside the breaker window.
        history: RestartHistory,
    },
    /// No engine, and one is being launched under the replacement ticket.
    Replacing {
        /// Attempts spent on this replacement.
        history: RestartHistory,
    },
    /// The breaker is open: too many replacements inside the window. The
    /// heartbeat resumes once the window drains.
    BreakerOpen {
        /// Attempts still inside the window.
        history: RestartHistory,
    },
}

impl Phase {
    /// The live engine, if this phase has one.
    fn engine(self) -> Option<Arc<dyn Engine>> {
        match self {
            Self::Running { engine, .. } => Some(engine),
            _ => None,
        }
    }

    /// The restart record this phase carries.
    fn history(&self) -> RestartHistory {
        match self {
            Self::Running { history, .. }
            | Self::Replacing { history, .. }
            | Self::BreakerOpen { history, .. } => history.clone(),
            Self::Idle => RestartHistory::default(),
        }
    }
}

/// Owns the supervision state and the ticket that serializes replacements.
struct Supervised {
    /// Never held across an await. Launch, health, and shutdown each read a
    /// value out first, so a slow browser can never park a reader.
    phase: Mutex<Phase>,
    /// The replacement ticket: held across a launch so concurrent failures
    /// relaunch once.
    replacing: AsyncMutex<()>,
    /// Watched by the session layer, which learns of every replacement.
    restarts: tokio::sync::watch::Sender<u64>,
}

impl Supervised {
    /// Reads the phase, recovering from poisoning: supervision continues
    /// even if a holder panicked, because the value stays consistent.
    fn lock(&self) -> MutexGuard<'_, Phase> {
        self.phase
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Installs a phase.
    fn set(&self, next: Phase) {
        *self.lock() = next;
    }

    /// The live engine, if any.
    fn engine(&self) -> Option<Arc<dyn Engine>> {
        match &*self.lock() {
            Phase::Running { engine, .. } => Some(Arc::clone(engine)),
            _ => None,
        }
    }

    /// The restart record the current phase carries.
    fn history(&self) -> RestartHistory {
        self.lock().history()
    }

    /// Discards a dead engine, keeping its restart record. Dropping the last
    /// handle is what lets the child process be reaped before the relaunch.
    fn mourn(&self) {
        let mut phase = self.lock();
        if matches!(&*phase, Phase::Running { .. }) {
            let history = phase.history();
            *phase = Phase::Replacing { history };
        }
    }

    /// A healthy probe closes the breaker window. Without this, five
    /// recoveries that each worked would open the breaker against an engine
    /// that is running fine, and the next crash would be met with
    /// abandonment instead of a relaunch.
    fn note_healthy(&self) {
        let mut phase = self.lock();
        if let Phase::Running { history, .. } = &mut *phase {
            *history = RestartHistory::default();
        }
    }

    /// Takes the engine out for shutdown, leaving the idle phase behind.
    fn take_engine(&self) -> Option<Arc<dyn Engine>> {
        std::mem::replace(&mut *self.lock(), Phase::Idle).engine()
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
    heartbeat: AsyncMutex<Option<JoinHandle<()>>>,
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
                phase: Mutex::new(Phase::Idle),
                replacing: AsyncMutex::new(()),
                restarts,
            }),
            heartbeat: AsyncMutex::new(None),
        }
    }

    /// Watcher for engine replacements; the count increments on every
    /// successful restart of a dead engine.
    pub fn restart_watcher(&self) -> tokio::sync::watch::Receiver<u64> {
        self.supervised.restarts.subscribe()
    }

    /// Overrides the heartbeat cadence; tests use a short interval.
    pub fn with_heartbeat(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = interval;
        self
    }

    /// Overrides the restart policy (window, backoff, breaker); tests
    /// use fast, deterministic schedules.
    pub fn with_policy(mut self, policy: RestartPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Performs the initial launch and starts the heartbeat loop.
    ///
    /// The heartbeat starts even when the initial launch exhausts the
    /// breaker: once the window drains the loop retries, so a failed launch
    /// leaves a supervised engine in waiting rather than a permanently empty
    /// slot. A repeated `start` is a no-op.
    pub async fn start(&self) -> Result<(), EngineError> {
        let mut slot = self.heartbeat.lock().await;
        if slot.is_some() {
            return Ok(());
        }

        let outcome = bring_up(&self.supervised, &self.launcher, self.mode, &self.policy).await;

        let supervised = Arc::clone(&self.supervised);
        let launcher = Arc::clone(&self.launcher);
        let policy = self.policy.clone();
        let interval = self.heartbeat_interval;
        let mode = self.mode;
        *slot = Some(tokio::spawn(async move {
            heartbeat_loop(supervised, launcher, mode, policy, interval).await;
        }));

        outcome
    }

    /// Returns the current engine, or `Terminated` while dead, replacing,
    /// or breaker-open. Callers treat this as an engine-level failure of
    /// the operation they were about to perform.
    pub async fn engine(&self) -> Result<Arc<dyn Engine>, EngineError> {
        self.supervised.engine().ok_or(EngineError::Terminated)
    }

    /// Stops the heartbeat, shuts the engine down, and clears it.
    pub async fn shutdown(&self) {
        if let Some(handle) = self.heartbeat.lock().await.take() {
            handle.abort();
        }
        // The engine leaves the phase before the shutdown is awaited:
        // `Engine::shutdown` can wait out a full command budget, and holding
        // the state that long would park concurrent `engine()` readers
        // instead of letting them fail fast with `Terminated`.
        let engine = self.supervised.take_engine();
        if let Some(engine) = engine
            && let Err(error) = engine.shutdown().await
        {
            eprintln!("rutter: engine shutdown failed: {error}");
        }
    }
}

/// Launches one engine under the restart policy and installs it.
///
/// This is the only restart path: the initial start and every heartbeat
/// replacement call it. It holds the replacement ticket for its whole run,
/// so concurrent failures converge on one relaunch, and it answers
/// [`EngineError::Terminated`] only when the breaker is open — a launch that
/// merely failed keeps retrying inside the policy budget.
async fn bring_up(
    supervised: &Arc<Supervised>,
    launcher: &Arc<dyn EngineLauncher>,
    mode: LaunchMode,
    policy: &RestartPolicy,
) -> Result<(), EngineError> {
    let _ticket = supervised.replacing.lock().await;

    // Another replacement may have completed while this call queued for the
    // ticket; its engine is the answer either way.
    if supervised.engine().is_some() {
        return Ok(());
    }

    let mut history = supervised.history();
    loop {
        match policy.decide(&mut history, Instant::now()) {
            RestartDecision::Open => {
                supervised.set(Phase::BreakerOpen { history });
                return Err(EngineError::Terminated);
            }
            RestartDecision::Allowed(delay) => tokio::time::sleep(delay).await,
        }

        // Recorded before the attempt runs, so a launch that never returns
        // cannot slip past the breaker.
        history.record(Instant::now());
        supervised.set(Phase::Replacing {
            history: history.clone(),
        });
        match launcher.launch(mode).await {
            Ok(engine) => {
                supervised.set(Phase::Running { engine, history });
                return Ok(());
            }
            Err(error) => {
                eprintln!(
                    "rutter: {} launch failed, retrying under policy: {error}",
                    launcher.describe()
                );
            }
        }
    }
}

/// Heartbeat: probe health, and on failure bring up a replacement.
/// Loop exit happens via task abort in [`Supervisor::shutdown`].
async fn heartbeat_loop(
    supervised: Arc<Supervised>,
    launcher: Arc<dyn EngineLauncher>,
    mode: LaunchMode,
    policy: RestartPolicy,
    interval: Duration,
) {
    loop {
        tokio::time::sleep(interval).await;

        if let Some(engine) = supervised.engine() {
            match engine.health().await {
                Ok(HealthReport { healthy: true, .. }) => {
                    supervised.note_healthy();
                    continue;
                }
                Ok(HealthReport {
                    healthy: false,
                    detail,
                    ..
                }) => {
                    eprintln!(
                        "rutter: engine unhealthy, restarting ({})",
                        detail.unwrap_or_else(|| "no detail".to_owned())
                    );
                }
                Err(error) => {
                    eprintln!("rutter: engine health probe failed: {error}");
                }
            }
        }
        // Either nothing is running — the breaker is draining, or the first
        // launch failed — or the probe above just condemned the live one.
        // Both cases take the same road.
        supervised.mourn();
        if bring_up(&supervised, &launcher, mode, &policy)
            .await
            .is_err()
        {
            eprintln!(
                "rutter: restart breaker open for {}; affected operations fail until the window drains",
                launcher.describe()
            );
            continue;
        }
        supervised.restarts.send_modify(|count| *count += 1);
    }
}
