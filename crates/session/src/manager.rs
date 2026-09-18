//! The session manager: one supervised engine, many client sessions.
//!
//! Boundary: session lifecycle, engine ownership, supervision wiring,
//! and recovery. The engine starts lazily on the first session request
//! (blueprint §8.5) and shuts down when the manager does; every session
//! evaluates the shared policy and parks approvals on the shared broker
//! (blueprint §7.6). When the supervisor replaces a dead engine, a
//! recovery task rebuilds each session: fresh context, storage-state
//! replay, page restoration, and an `EngineRestarted` event per session
//! (blueprint §7.4).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use rutter_core::ids::SessionId;
use rutter_engine::config::LaunchMode;
use rutter_engine::engine::Engine;
use rutter_engine::error::EngineError;
use rutter_engine::supervisor::{EngineLauncher, Supervisor};
use rutter_events::{Backbone, Event};
use rutter_policy::{ApprovalBroker, RuleSet};

use crate::config::SessionConfig;
use crate::session::Session;

/// The running engine plus every open session.
struct Running {
    supervisor: Arc<Supervisor>,
    engine: Arc<dyn Engine>,
    backbone: Arc<Backbone>,
    sessions: HashMap<SessionId, Arc<Session>>,
}

/// Shared manager state; the recovery task holds this weakly.
struct Inner {
    launcher: Arc<dyn EngineLauncher>,
    mode: LaunchMode,
    config: SessionConfig,
    policy: Arc<RuleSet>,
    broker: Arc<ApprovalBroker>,
    state_dir: Option<PathBuf>,
    /// The async mutex is deliberate: starting the engine and creating
    /// contexts await, and concurrent session requests must serialize on
    /// one engine.
    running: tokio::sync::Mutex<Option<Running>>,
}

/// Hands out sessions over one shared engine.
pub struct SessionManager {
    inner: Arc<Inner>,
}

impl SessionManager {
    /// Creates a manager; nothing starts until the first session. The
    /// recovery task spawns on the first engine start.
    pub fn new(
        launcher: Arc<dyn EngineLauncher>,
        mode: LaunchMode,
        config: SessionConfig,
        policy: Arc<RuleSet>,
        broker: Arc<ApprovalBroker>,
        state_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                launcher,
                mode,
                config,
                policy,
                broker,
                state_dir,
                running: tokio::sync::Mutex::new(None),
            }),
        }
    }

    /// The approval broker human decisions go to (dashboard policy API).
    pub fn broker(&self) -> Arc<ApprovalBroker> {
        Arc::clone(&self.inner.broker)
    }

    /// Looks up an open session without creating one.
    pub async fn get_session(&self, id: &SessionId) -> Option<Arc<Session>> {
        self.inner
            .running
            .lock()
            .await
            .as_ref()
            .and_then(|running| running.sessions.get(id))
            .map(Arc::clone)
    }

    /// The event backbone once the engine started; `None` before the
    /// first session (dashboard sources replay + live events here).
    pub async fn backbone(&self) -> Option<Arc<Backbone>> {
        self.inner
            .running
            .lock()
            .await
            .as_ref()
            .map(|running| Arc::clone(&running.backbone))
    }

    /// Ids of the open sessions.
    pub async fn session_ids(&self) -> Vec<SessionId> {
        self.inner
            .running
            .lock()
            .await
            .as_ref()
            .map(|running| running.sessions.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Returns the session for `id`, starting the engine and creating
    /// the session's context on first request.
    pub async fn session(&self, id: SessionId) -> Result<Arc<Session>, EngineError> {
        let mut guard = self.inner.running.lock().await;
        if guard.is_none() {
            // start_running spawns the single recovery task for this
            // supervisor; spawning here as well would run recovery twice
            // per restart.
            let running = start_running(&self.inner).await?;
            let descriptor = running.engine.descriptor();
            // The engine event lands in the first session's history: it
            // is the session that witnessed the launch.
            running.backbone.publish(
                id.clone(),
                Event::EngineStarted {
                    backend: descriptor.backend.to_string(),
                    version: descriptor.version,
                },
            );
            *guard = Some(running);
        }

        let Some(running) = guard.as_mut() else {
            // Unreachable: the branch above filled the slot when empty.
            return Err(EngineError::Internal {
                detail: "session manager did not start".to_owned(),
            });
        };
        if let Some(session) = running.sessions.get(&id) {
            return Ok(Arc::clone(session));
        }

        let context = running
            .engine
            .create_context(self.inner.config.context_config())
            .await?;
        let state_path = self
            .inner
            .state_dir
            .as_ref()
            .map(|dir| dir.join(format!("{id}.storage.json")));
        let session = Arc::new(Session::new(
            id.clone(),
            context,
            Arc::clone(&running.backbone),
            self.inner.config.clone(),
            Arc::clone(&self.inner.policy),
            Arc::clone(&self.inner.broker),
            state_path,
        ));
        running.sessions.insert(id.clone(), Arc::clone(&session));
        running.backbone.publish(id, Event::SessionStarted);
        Ok(session)
    }

    /// Closes one session: its pages close and its history stays in the
    /// ring until the manager drops. Unknown ids succeed as no-ops.
    pub async fn close_session(&self, id: &SessionId) -> Result<(), EngineError> {
        let session = {
            let mut guard = self.inner.running.lock().await;
            match guard
                .as_mut()
                .and_then(|running| running.sessions.remove(id))
            {
                Some(session) => session,
                None => return Ok(()),
            }
        };

        session.close().await;
        session.backbone().publish(id.clone(), Event::SessionClosed);
        Ok(())
    }

    /// Shuts the engine down; open sessions die with it, but their
    /// storage states remain on disk and reload on the next start.
    pub async fn shutdown(&self) {
        let supervisor = {
            let mut guard = self.inner.running.lock().await;
            guard.take().map(|running| running.supervisor)
        };
        if let Some(supervisor) = supervisor {
            supervisor.shutdown().await;
        }
    }
}

/// Starts the engine and spawns the recovery task for its supervisor.
async fn start_running(inner: &Arc<Inner>) -> Result<Running, EngineError> {
    let supervisor = Arc::new(Supervisor::new(Arc::clone(&inner.launcher), inner.mode));
    supervisor.start().await?;
    let engine = supervisor.engine().await?;
    spawn_recovery(inner, supervisor.restart_watcher());
    Ok(Running {
        supervisor,
        engine,
        backbone: Arc::new(Backbone::new()),
        sessions: HashMap::new(),
    })
}

/// Watches for engine replacements and rebuilds every session:
/// fresh context, storage-state replay, page restoration, and an
/// `EngineRestarted` event per session (blueprint §7.4).
fn spawn_recovery(inner: &Arc<Inner>, mut watcher: tokio::sync::watch::Receiver<u64>) {
    let weak = Arc::downgrade(inner);
    tokio::spawn(async move {
        // The current value is the baseline; only real bumps recover.
        let _baseline = *watcher.borrow_and_update();
        while watcher.changed().await.is_ok() {
            let _count = *watcher.borrow_and_update();
            let Some(inner) = weak.upgrade() else {
                break;
            };
            let mut guard = inner.running.lock().await;
            let Some(running) = guard.as_mut() else {
                continue;
            };
            eprintln!(
                "rutter: engine restarted; recovering {} session(s)",
                running.sessions.len()
            );
            for session in running.sessions.values() {
                match running
                    .engine
                    .create_context(inner.config.context_config())
                    .await
                {
                    Ok(context) => session.recover(context).await,
                    Err(error) => {
                        eprintln!("rutter: recovery context failed: {error}");
                    }
                }
            }
        }
    });
}
