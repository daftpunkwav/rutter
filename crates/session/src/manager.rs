//! The session manager: one supervised engine, many client sessions.
//!
//! Boundary: session lifecycle and engine ownership. The engine starts
//! lazily on the first session request (blueprint §8.5) and shuts down
//! when the manager does; per-session state beyond pages (cookies,
//! storage) persists only as far as the engine process in M1 — storage
//! state replay is M2.

use std::collections::HashMap;
use std::sync::Arc;

use rutter_core::ids::SessionId;
use rutter_engine::config::LaunchMode;
use rutter_engine::engine::Engine;
use rutter_engine::error::EngineError;
use rutter_engine::supervisor::{EngineLauncher, Supervisor};
use rutter_events::{Backbone, Event};

use crate::config::SessionConfig;
use crate::session::Session;

/// The running engine plus every open session.
struct Running {
    supervisor: Arc<Supervisor>,
    engine: Arc<dyn Engine>,
    backbone: Arc<Backbone>,
    sessions: HashMap<SessionId, Arc<Session>>,
}

/// Hands out sessions over one shared engine.
pub struct SessionManager {
    launcher: Arc<dyn EngineLauncher>,
    mode: LaunchMode,
    config: SessionConfig,
    /// The async mutex is deliberate: starting the engine and creating
    /// contexts await, and concurrent session requests must serialize on
    /// one engine.
    running: tokio::sync::Mutex<Option<Running>>,
}

impl SessionManager {
    /// Creates a manager; nothing starts until the first session.
    pub fn new(launcher: Arc<dyn EngineLauncher>, mode: LaunchMode, config: SessionConfig) -> Self {
        Self {
            launcher,
            mode,
            config,
            running: tokio::sync::Mutex::new(None),
        }
    }

    /// Returns the session for `id`, starting the engine and creating
    /// the session's context on first request.
    pub async fn session(&self, id: SessionId) -> Result<Arc<Session>, EngineError> {
        let mut guard = self.running.lock().await;
        if guard.is_none() {
            let running = self.start_running().await?;
            let descriptor = running.engine.descriptor();
            // The engine event lands in the first session's history: it
            // is the session that witnessed the launch.
            running.backbone.publish(
                id.clone(),
                Event::EngineStarted {
                    backend: format!("{:?}", descriptor.backend),
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
            .create_context(self.config.context_config())
            .await?;
        let session = Arc::new(Session::new(
            id.clone(),
            context,
            Arc::clone(&running.backbone),
            self.config.clone(),
        ));
        running.sessions.insert(id.clone(), Arc::clone(&session));
        running.backbone.publish(id, Event::SessionStarted);
        Ok(session)
    }

    /// Closes one session: its pages close and its history stays in the
    /// ring until the manager drops. Unknown ids succeed as no-ops.
    pub async fn close_session(&self, id: &SessionId) -> Result<(), EngineError> {
        let session = {
            let mut guard = self.running.lock().await;
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

    /// Shuts the engine down; open sessions die with it (M1 has no
    /// storage-state replay; blueprint §10 defers that to M2).
    pub async fn shutdown(&self) {
        let supervisor = {
            let mut guard = self.running.lock().await;
            guard.take().map(|running| running.supervisor)
        };
        if let Some(supervisor) = supervisor {
            supervisor.shutdown().await;
        }
    }

    async fn start_running(&self) -> Result<Running, EngineError> {
        let supervisor = Arc::new(Supervisor::new(Arc::clone(&self.launcher), self.mode));
        supervisor.start().await?;
        let engine = supervisor.engine().await?;
        Ok(Running {
            supervisor,
            engine,
            backbone: Arc::new(Backbone::new()),
            sessions: HashMap::new(),
        })
    }
}
