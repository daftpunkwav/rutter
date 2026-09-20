//! Functional tests for the dashboard's public API: per-launch token
//! generation properties. Endpoint security internals (host/token
//! checks) and the access hand-off have their unit tests beside the code
//! in `src/auth.rs` and `src/lib.rs`.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::sync::Arc;

use rutter_dashboard::DashboardServer;
use rutter_engine::config::LaunchMode;
use rutter_engine::supervisor::EngineLauncher;
use rutter_policy::{ApprovalBroker, RuleSet};
use rutter_session::config::SessionConfig;
use rutter_session::manager::SessionManager;

/// A manager wired to a launcher that is never launched; the token is
/// generated from process facts, not engine facts.
fn server() -> DashboardServer {
    struct UnusedLauncher;
    #[async_trait::async_trait]
    impl EngineLauncher for UnusedLauncher {
        fn describe(&self) -> String {
            "unused".to_owned()
        }
        async fn launch(
            &self,
            _mode: LaunchMode,
        ) -> Result<Arc<dyn rutter_engine::engine::Engine>, rutter_engine::error::EngineError>
        {
            Err(rutter_engine::error::EngineError::Unsupported {
                operation: "launch".to_owned(),
                reason: "token tests never launch engines".to_owned(),
            })
        }
    }
    DashboardServer::new(
        Arc::new(SessionManager::new(
            Arc::new(UnusedLauncher),
            LaunchMode::Headless,
            SessionConfig::default(),
            Arc::new(RuleSet::default_set()),
            Arc::new(ApprovalBroker::new()),
            None,
        )),
        0,
        // No hand-off path: these tests read the token, never the channel.
        None,
    )
}

#[test]
fn tokens_are_sixteen_hex_characters() {
    let token = server().token();
    assert_eq!(token.len(), 16, "64-bit hex token: {token}");
    assert!(
        token.chars().all(|c| c.is_ascii_hexdigit()),
        "hex only, so it travels in a cookie: {token}"
    );
}

#[test]
fn two_servers_do_not_share_a_token() {
    // The token is seeded from OS entropy per launch; a repeat would
    // let one dashboard's URL open another's approvals.
    let first = server().token();
    let second = server().token();
    assert_ne!(first, second, "tokens must be per-launch");
}

#[test]
fn one_server_always_hands_out_the_same_token() {
    // The accessor reads a value minted once at construction. Minting
    // per call would hand a second reader a token the running server
    // does not accept, and its dashboard would never open.
    let server = server();
    assert_eq!(server.token(), server.token(), "token() reads, not mints");
}
