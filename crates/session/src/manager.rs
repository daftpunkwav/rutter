//! The session manager: one supervised engine, many client sessions.
//!
//! Boundary: session lifecycle, engine ownership, supervision wiring,
//! and recovery. The engine starts lazily on the first session request
//! (docs/architecture.md) and shuts down when the manager does; every session
//! evaluates the shared policy and parks approvals on the shared broker
//! (docs/policy.md). When the supervisor replaces a dead engine, a
//! recovery task rebuilds each session: fresh context, storage-state
//! replay, page restoration, and an `EngineRestarted` event per session
//! (docs/sessions.md).
//!
//! Locking: three guards, each with one job, and none of them held across
//! the slow work it does not protect. The engine slot is read-mostly, the
//! startup lock covers only the launch, and the session map covers only
//! bookkeeping. Recovery snapshots the sessions under their lock and then
//! rebuilds them outside it. One shared lock used to serialize all three,
//! which let a browser launch — up to a minute of backoff while the restart
//! breaker drains — block the dashboard from reading the event backbone.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use rutter_core::ids::SessionId;
use rutter_engine::config::LaunchMode;
use rutter_engine::error::EngineError;
use rutter_engine::supervisor::{EngineLauncher, Supervisor};
use rutter_events::{Backbone, Event};
use rutter_policy::{ApprovalBroker, RuleSet};

use crate::config::SessionConfig;
use crate::error::SessionError;
use crate::session::Session;

/// The running supervisor plus the event backbone that belongs to it.
///
/// The engine itself is never stored here: the supervisor replaces dead
/// engines under the same handle, so every consumer asks the supervisor
/// for the current one. Caching an engine `Arc` would pin the dead
/// instance and break every context creation after the first restart.
struct Running {
    supervisor: Arc<Supervisor>,
    backbone: Arc<Backbone>,
}

/// Shared manager state; the recovery task holds this weakly.
struct Inner {
    launcher: Arc<dyn EngineLauncher>,
    mode: LaunchMode,
    config: SessionConfig,
    policy: Arc<RuleSet>,
    broker: Arc<ApprovalBroker>,
    state_dir: Option<PathBuf>,
    /// The engine once it is up. Read by every session request and by the
    /// dashboard; written at start and shutdown only, so the read side gets
    /// a lock that no launch can hold.
    engine: tokio::sync::RwLock<Option<Arc<Running>>>,
    /// Serializes startup so two first requests do not launch two browsers.
    /// Held across the launch, which is exactly what it is for.
    starting: tokio::sync::Mutex<()>,
    /// Open sessions and nothing else.
    sessions: tokio::sync::Mutex<HashMap<SessionId, Arc<Session>>>,
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
                engine: tokio::sync::RwLock::new(None),
                starting: tokio::sync::Mutex::new(()),
                sessions: tokio::sync::Mutex::new(HashMap::new()),
            }),
        }
    }

    /// The approval broker human decisions go to (dashboard policy API).
    pub fn broker(&self) -> Arc<ApprovalBroker> {
        Arc::clone(&self.inner.broker)
    }

    /// Looks up an open session without creating one.
    pub async fn get_session(&self, id: &SessionId) -> Option<Arc<Session>> {
        self.inner.sessions.lock().await.get(id).map(Arc::clone)
    }

    /// The event backbone once the engine started; `None` before the
    /// first session (dashboard sources replay + live events here).
    pub async fn backbone(&self) -> Option<Arc<Backbone>> {
        self.current()
            .await
            .map(|running| Arc::clone(&running.backbone))
    }

    /// Ids of the open sessions.
    pub async fn session_ids(&self) -> Vec<SessionId> {
        self.inner.sessions.lock().await.keys().cloned().collect()
    }

    /// Returns the session for `id`, starting the engine and creating
    /// the session's context on first request.
    pub async fn session(&self, id: SessionId) -> Result<Arc<Session>, SessionError> {
        let running = self.ensure_running(&id).await?;
        let mut sessions = self.inner.sessions.lock().await;
        if let Some(session) = sessions.get(&id) {
            return Ok(Arc::clone(session));
        }
        if sessions.len() >= self.inner.config.max_sessions {
            return Err(SessionError::Capacity {
                detail: format!(
                    "this server holds {} session(s) already; close one before \
                     opening another",
                    self.inner.config.max_sessions
                ),
            });
        }

        // Asked fresh every time: a restart may have swapped the engine
        // while this caller waited on the session map.
        let engine = running.supervisor.engine().await?;
        let context = engine
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
        sessions.insert(id.clone(), Arc::clone(&session));
        running.backbone.publish(id, Event::SessionStarted);
        Ok(session)
    }

    /// Closes one session: its pages close and its event history is
    /// dropped with it (see the `forget` call below). Unknown ids
    /// succeed as no-ops. Infallible: the removed session's own close
    /// tolerates context failures, so there is no error to report.
    pub async fn close_session(&self, id: &SessionId) {
        let session = {
            let mut sessions = self.inner.sessions.lock().await;
            match sessions.remove(id) {
                Some(session) => session,
                None => return,
            }
        };

        // Outside the map lock on purpose: a close waits on the engine, and
        // the dashboard's reads must not queue behind it. Recovery may still
        // hold this session from an earlier snapshot, so it re-checks
        // membership before rebuilding anything.
        session.close().await;
        session.backbone().publish(id.clone(), Event::SessionClosed);
        // The close is the session's last event: replay consumers only
        // read open sessions (the dashboard lists open ones), so the ring
        // is dropped instead of lingering in the backbone's map — a serve
        // process that churns through session ids would otherwise grow that
        // map without bound.
        session.backbone().forget(id);
    }

    /// Shuts the engine down; open sessions die with it, but their
    /// storage states remain on disk and reload on the next start.
    pub async fn shutdown(&self) {
        let supervisor = {
            let mut engine = self.inner.engine.write().await;
            engine.take().map(|running| Arc::clone(&running.supervisor))
        };
        self.inner.sessions.lock().await.clear();
        if let Some(supervisor) = supervisor {
            supervisor.shutdown().await;
        }
    }

    /// The current engine, if one ever started.
    async fn current(&self) -> Option<Arc<Running>> {
        self.inner.engine.read().await.clone()
    }

    /// The running engine, starting it on first use.
    ///
    /// The launch happens under `starting`, never under the engine slot or
    /// the session map: a launch that backs off for a minute used to hold
    /// every reader with it.
    async fn ensure_running(&self, id: &SessionId) -> Result<Arc<Running>, SessionError> {
        if let Some(running) = self.current().await {
            return Ok(running);
        }
        let _starting = self.inner.starting.lock().await;
        // Another caller may have finished starting while this one queued.
        if let Some(running) = self.current().await {
            return Ok(running);
        }

        let running = Arc::new(start_running(&self.inner).await?);
        // The engine event lands in the first session's history: it is the
        // session that witnessed the launch.
        let engine = match running.supervisor.engine().await {
            Ok(engine) => engine,
            Err(error) => {
                // The engine slot can already be empty again if the
                // heartbeat cleared the just-started instance. Dropping
                // `running` without stopping the supervisor would orphan its
                // heartbeat task, which would keep relaunching a browser
                // nobody owns.
                running.supervisor.shutdown().await;
                return Err(error.into());
            }
        };
        let descriptor = engine.descriptor();
        running.backbone.publish(
            id.clone(),
            Event::EngineStarted {
                backend: descriptor.backend.to_string(),
                version: descriptor.version,
            },
        );
        *self.inner.engine.write().await = Some(Arc::clone(&running));
        Ok(running)
    }
}

/// Starts the engine and spawns the recovery task for its supervisor.
async fn start_running(inner: &Arc<Inner>) -> Result<Running, EngineError> {
    let supervisor = Arc::new(Supervisor::new(Arc::clone(&inner.launcher), inner.mode));
    // On failure the supervisor's heartbeat keeps retrying in the
    // background, so abort it here: leaving it running would orphan a
    // task that relaunches a browser nobody owns.
    if let Err(error) = supervisor.start().await {
        supervisor.shutdown().await;
        return Err(error);
    }
    spawn_recovery(inner, supervisor.restart_watcher());
    Ok(Running {
        supervisor,
        backbone: Arc::new(Backbone::new()),
    })
}

/// Watches for engine replacements and rebuilds every session:
/// fresh context, storage-state replay, page restoration, and an
/// `EngineRestarted` event per session (docs/sessions.md).
fn spawn_recovery(inner: &Arc<Inner>, mut watcher: tokio::sync::watch::Receiver<u64>) {
    let weak = Arc::downgrade(inner);
    tokio::spawn(async move {
        // The current value is the baseline; only real bumps recover.
        let _baseline = *watcher.borrow_and_update();
        while watcher.changed().await.is_ok() {
            let Some(inner) = weak.upgrade() else {
                break;
            };
            let running = match inner.engine.read().await.clone() {
                Some(running) => running,
                None => continue,
            };
            // Snapshot, then rebuild outside the lock: recovery is a chain
            // of engine round trips, and holding the session map across it
            // would freeze `get_session` and the dashboard's session list
            // for as long as the slowest page takes to load.
            let pending: Vec<Arc<Session>> = inner
                .sessions
                .lock()
                .await
                .values()
                .map(Arc::clone)
                .collect();
            if pending.is_empty() {
                continue;
            }
            eprintln!(
                "rutter: engine restarted; recovering {} session(s)",
                pending.len()
            );
            // Ask the supervisor for the engine that replaced the dead
            // one; a cached `Arc` would still point at the dead instance.
            let Ok(engine) = running.supervisor.engine().await else {
                eprintln!("rutter: engine unavailable during recovery");
                continue;
            };
            for session in pending {
                // Re-check membership: the snapshot was taken before this
                // point, and a session closed in the meantime must not be
                // rebuilt into a context nobody will ever close again.
                if !inner.sessions.lock().await.contains_key(session.id()) {
                    continue;
                }
                match engine.create_context(inner.config.context_config()).await {
                    Ok(context) => session.recover(context).await,
                    Err(error) => {
                        eprintln!("rutter: recovery context failed: {error}");
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    //! The recovery task's branches, driven by hand through a watch
    //! channel: no engine yet, no sessions to rebuild, the happy rebuild
    //! (cookies replayed, pages restored, event published), and the
    //! context-creation failure. The default 10 s heartbeat stays out of
    //! these tests — the watcher is the same signal the supervisor sends.

    use super::*;
    use crate::mock::MockContext;
    use rutter_core::cookie::Cookie;
    use rutter_engine::descriptor::{EngineBackend, EngineCapabilities, EngineDescriptor};
    use rutter_engine::health::HealthReport;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// An engine whose contexts are observable and whose `create_context`
    /// can be made to fail on demand.
    struct RecoveryEngine {
        fail_contexts: AtomicBool,
        contexts: Mutex<Vec<Arc<MockContext>>>,
    }

    #[async_trait::async_trait]
    impl rutter_engine::engine::Engine for RecoveryEngine {
        fn descriptor(&self) -> EngineDescriptor {
            EngineDescriptor {
                backend: EngineBackend::ChromiumHeadlessShell,
                version: "recovery".to_owned(),
                capabilities: EngineCapabilities {
                    headless: true,
                    headed: false,
                    screencast: false,
                    per_context_isolation: true,
                },
            }
        }

        async fn create_context(
            &self,
            _config: rutter_engine::config::ContextConfig,
        ) -> Result<Arc<dyn rutter_engine::context::ContextHandle>, EngineError> {
            if self.fail_contexts.load(Ordering::SeqCst) {
                return Err(EngineError::Terminated);
            }
            let context = Arc::new(MockContext::new());
            self.contexts
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(Arc::clone(&context));
            Ok(context)
        }

        async fn health(&self) -> Result<HealthReport, EngineError> {
            Ok(HealthReport {
                healthy: true,
                backend_version: Some("recovery".to_owned()),
                detail: None,
            })
        }

        async fn shutdown(&self) -> Result<(), EngineError> {
            Ok(())
        }
    }

    struct RecoveryLauncher {
        engines: Mutex<Vec<Arc<RecoveryEngine>>>,
    }

    #[async_trait::async_trait]
    impl EngineLauncher for RecoveryLauncher {
        fn describe(&self) -> String {
            "recovery".to_owned()
        }

        async fn launch(
            &self,
            _mode: LaunchMode,
        ) -> Result<Arc<dyn rutter_engine::engine::Engine>, EngineError> {
            let engine = Arc::new(RecoveryEngine {
                fail_contexts: AtomicBool::new(false),
                contexts: Mutex::new(Vec::new()),
            });
            self.engines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(Arc::clone(&engine));
            Ok(engine)
        }
    }

    fn test_inner(launcher: Arc<RecoveryLauncher>) -> Arc<Inner> {
        Arc::new(Inner {
            launcher,
            mode: LaunchMode::Headless,
            config: SessionConfig::default(),
            policy: Arc::new(RuleSet::default_set()),
            broker: Arc::new(ApprovalBroker::new()),
            state_dir: None,
            engine: tokio::sync::RwLock::new(None),
            starting: tokio::sync::Mutex::new(()),
            sessions: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// A started `Running` over a recovery launcher, plus the typed
    /// engine the supervisor launched.
    async fn start_test_running(
        launcher: Arc<RecoveryLauncher>,
    ) -> (Arc<Running>, Arc<RecoveryEngine>) {
        let supervisor = Arc::new(Supervisor::new(
            Arc::clone(&launcher) as Arc<dyn EngineLauncher>,
            LaunchMode::Headless,
        ));
        supervisor.start().await.expect("supervisor starts");
        let engine = launcher
            .engines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .first()
            .cloned()
            .expect("the supervisor launched one engine");
        let running = Arc::new(Running {
            supervisor,
            backbone: Arc::new(Backbone::new()),
        });
        (running, engine)
    }

    /// One session with a tracked page, in the manager's map.
    async fn seeded_session(running: &Running, inner: &Arc<Inner>) -> Arc<Session> {
        let session = Arc::new(Session::new(
            SessionId::new("s1"),
            Arc::new(MockContext::new()),
            Arc::clone(&running.backbone),
            SessionConfig::default(),
            Arc::clone(&inner.policy),
            Arc::clone(&inner.broker),
            None,
        ));
        session
            .execute(
                rutter_core::action::Action::Navigate {
                    url: "https://a.example".to_owned(),
                },
                rutter_core::action::Origin::Human,
            )
            .await
            .expect("navigate seeds the tracked url");
        inner
            .sessions
            .lock()
            .await
            .insert(SessionId::new("s1"), Arc::clone(&session));
        session
    }

    async fn wait_for_event(
        backbone: &Backbone,
        session: &SessionId,
        pred: impl Fn(&Event) -> bool,
    ) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if backbone
                    .replay(session)
                    .iter()
                    .any(|envelope| pred(&envelope.event))
                {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the event lands on the backbone");
    }

    #[tokio::test]
    async fn a_restart_bump_with_no_engine_yet_is_ignored() {
        let launcher = Arc::new(RecoveryLauncher {
            engines: Mutex::new(Vec::new()),
        });
        let inner = test_inner(Arc::clone(&launcher));
        let (tx, rx) = tokio::sync::watch::channel(0_u64);
        spawn_recovery(&inner, rx);
        // The task must establish its baseline before the bump lands.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        tx.send_modify(|count| *count += 1);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            launcher
                .engines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "no engine exists, so nothing may be rebuilt"
        );
    }

    #[tokio::test]
    async fn a_restart_bump_with_no_sessions_recovers_nothing() {
        let launcher = Arc::new(RecoveryLauncher {
            engines: Mutex::new(Vec::new()),
        });
        let inner = test_inner(Arc::clone(&launcher));
        let (running, engine) = start_test_running(Arc::clone(&launcher)).await;
        *inner.engine.write().await = Some(Arc::clone(&running));
        let (tx, rx) = tokio::sync::watch::channel(0_u64);
        spawn_recovery(&inner, rx);
        // The task must establish its baseline before the bump lands.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        tx.send_modify(|count| *count += 1);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            engine
                .contexts
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "no sessions, so no recovery context is created"
        );
        running.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn a_restart_rebuilds_sessions_with_cookies_and_pages() {
        let launcher = Arc::new(RecoveryLauncher {
            engines: Mutex::new(Vec::new()),
        });
        let inner = test_inner(Arc::clone(&launcher));
        let (running, engine) = start_test_running(Arc::clone(&launcher)).await;
        *inner.engine.write().await = Some(Arc::clone(&running));
        let session = seeded_session(&running, &inner).await;

        // A remembered cookie: recovery must replay it into the fresh
        // context (the in-memory copy is what survives a dead engine).
        *session.lock_last_storage() = crate::storage::StorageState {
            cookies: vec![Cookie {
                name: "session".to_owned(),
                value: "42".to_owned(),
                domain: "a.example".to_owned(),
                path: None,
                secure: false,
                http_only: false,
                same_site: None,
                expires: None,
            }],
            origins: Vec::new(),
        };

        let (tx, rx) = tokio::sync::watch::channel(0_u64);
        spawn_recovery(&inner, rx);
        // Let the task establish its baseline; a bump that lands before
        // the first poll would be mistaken for it.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        tx.send_modify(|count| *count += 1);

        wait_for_event(&running.backbone, &SessionId::new("s1"), |event| {
            matches!(event, Event::EngineRestarted)
        })
        .await;

        let contexts = engine
            .contexts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(contexts.len(), 1, "recovery created one fresh context");
        let calls = contexts[0].set_cookie_calls();
        assert!(
            calls
                .iter()
                .flatten()
                .any(|cookie| cookie.name == "session"),
            "the remembered cookie is replayed into the fresh context: {calls:?}"
        );

        let pages = session.pages().await;
        assert_eq!(pages.len(), 1, "the tracked page is restored");
        assert_eq!(pages[0].url, "https://a.example");
        assert!(pages[0].active, "exactly one restored page is active");
        wait_for_event(&running.backbone, &SessionId::new("s1"), |event| {
            matches!(event, Event::PageOpened { .. })
        })
        .await;

        running.supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn a_recovery_context_failure_leaves_the_session_as_it_was() {
        let launcher = Arc::new(RecoveryLauncher {
            engines: Mutex::new(Vec::new()),
        });
        let inner = test_inner(Arc::clone(&launcher));
        let (running, engine) = start_test_running(Arc::clone(&launcher)).await;
        *inner.engine.write().await = Some(Arc::clone(&running));
        let session = seeded_session(&running, &inner).await;

        engine.fail_contexts.store(true, Ordering::SeqCst);
        let (tx, rx) = tokio::sync::watch::channel(0_u64);
        spawn_recovery(&inner, rx);
        // The task must establish its baseline before the bump lands.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        tx.send_modify(|count| *count += 1);
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        assert!(
            running
                .backbone
                .replay(&SessionId::new("s1"))
                .iter()
                .all(|envelope| !matches!(envelope.event, Event::EngineRestarted)),
            "no restart event when the rebuild context failed"
        );
        assert_eq!(
            session.pages().await.len(),
            1,
            "the old tracking survives the failed rebuild"
        );
        running.supervisor.shutdown().await;
    }
}
