//! Functional tests for `SessionManager` through its public API: the
//! session cap, close semantics, and session-handle stability. Runs
//! against the scripted launcher in [`common`] — no real browser.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use std::sync::Arc;

use common::FlowLauncher;
use rutter_core::ids::SessionId;
use rutter_engine::config::LaunchMode;
use rutter_engine::supervisor::EngineLauncher;
use rutter_policy::{ApprovalBroker, RuleSet};
use rutter_session::SessionError;
use rutter_session::config::SessionConfig;
use rutter_session::manager::SessionManager;

/// A manager with a scripted launcher and the given session cap.
fn manager_with(max_sessions: usize) -> (SessionManager, Arc<FlowLauncher>) {
    let launcher = FlowLauncher::new();
    let manager = SessionManager::new(
        Arc::clone(&launcher) as Arc<dyn EngineLauncher>,
        LaunchMode::Headless,
        SessionConfig {
            max_sessions,
            ..SessionConfig::default()
        },
        Arc::new(RuleSet::default_set()),
        Arc::new(ApprovalBroker::new()),
        None,
    );
    (manager, launcher)
}

#[tokio::test]
async fn session_count_is_capped_at_max_sessions() {
    let (manager, launcher) = manager_with(1);

    manager
        .session(SessionId::new("s1"))
        .await
        .expect("first session");
    let second = manager.session(SessionId::new("s2")).await;
    match second {
        Err(SessionError::Capacity { detail }) => {
            assert!(detail.contains('1'), "the cap is named: {detail}");
        }
        Err(error) => panic!("expected a Capacity error, got {error}"),
        Ok(_) => panic!("the second session must be refused by the cap"),
    }
    // The refused session must not exist: the cap protects the
    // engine, it does not park the request.
    assert_eq!(manager.session_ids().await, vec![SessionId::new("s1")]);

    // Closing one session frees the slot.
    manager.close_session(&SessionId::new("s1")).await;
    manager
        .session(SessionId::new("s2"))
        .await
        .expect("the freed slot is reusable");
    let _ = launcher.engine(0);
}

#[tokio::test]
async fn close_session_closes_the_context_and_unknown_ids_succeed() {
    let (manager, launcher) = manager_with(4);

    let id = SessionId::new("s1");
    manager.session(id.clone()).await.expect("session");
    let context = launcher.engine(0).context(0);

    // An unknown id is a no-op, not an error.
    manager.close_session(&SessionId::new("ghost")).await;
    manager.close_session(&id).await;
    assert!(
        context.is_closed(),
        "the session teardown must close its browser context"
    );
    assert!(manager.get_session(&id).await.is_none());
    // A closed session is not resurrected by a second close.
    manager.close_session(&id).await;
}

#[tokio::test]
async fn close_session_frees_the_backbone_history() {
    // The ring of a closed session is never replayed (the dashboard
    // replays open sessions only); it must leave the backbone's map, or
    // a serve process churning through session ids grows it forever.
    let (manager, _launcher) = manager_with(2);
    let id = SessionId::new("s1");
    manager.session(id.clone()).await.expect("session");

    let backbone = manager.backbone().await.expect("backbone");
    assert!(
        !backbone.replay(&id).is_empty(),
        "the open session has history"
    );

    manager.close_session(&id).await;
    assert!(
        backbone.replay(&id).is_empty(),
        "a closed session's ring is dropped"
    );
}

#[tokio::test]
async fn session_return_is_stable_across_calls() {
    let (manager, _launcher) = manager_with(4);
    let id = SessionId::new("s1");
    let first = manager.session(id.clone()).await.expect("session");
    let second = manager.session(id).await.expect("session");
    assert!(
        Arc::ptr_eq(&first, &second),
        "the same session handle is handed out, not a new one"
    );
}
