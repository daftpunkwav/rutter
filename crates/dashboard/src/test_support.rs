//! Test-only fixtures shared by the crate's unit tests: one launcher
//! stub and one manager constructor, so the test modules cannot drift
//! into differently wired copies of the same fixture.

use std::sync::Arc;

use rutter_policy::{ApprovalBroker, RuleSet};
use rutter_session::config::SessionConfig;
use rutter_session::manager::SessionManager;

/// A manager over the launcher stub below, wired the way production
/// wires it. The dashboard tests never launch an engine through it.
pub(crate) fn test_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        Arc::new(UnsupportedLauncher),
        rutter_engine::config::LaunchMode::Headless,
        SessionConfig::default(),
        Arc::new(RuleSet::default_set()),
        Arc::new(ApprovalBroker::new()),
        None,
    ))
}

/// Launcher stub satisfying the manager constructor; the dashboard
/// tests never launch an engine through it.
pub(crate) struct UnsupportedLauncher;

#[async_trait::async_trait]
impl rutter_engine::supervisor::EngineLauncher for UnsupportedLauncher {
    fn describe(&self) -> String {
        "unsupported".to_owned()
    }

    async fn launch(
        &self,
        _mode: rutter_engine::config::LaunchMode,
    ) -> Result<Arc<dyn rutter_engine::engine::Engine>, rutter_engine::error::EngineError> {
        Err(rutter_engine::error::EngineError::Unsupported {
            operation: "launch".to_owned(),
            reason: "dashboard tests never launch engines".to_owned(),
        })
    }
}
