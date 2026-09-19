//! Functional tests for the session flow through the crate's public
//! API: a `SessionManager` hands out sessions whose execute path
//! (navigate → snapshot), approval-granted cookies, wait_for contract,
//! and close semantics all run against the scripted engine in
//! [`common`] — no real browser.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::FlowLauncher;
use rutter_core::action::{Action, Origin};
use rutter_core::cookie::Cookie;
use rutter_core::error::ActionError;
use rutter_core::ids::SessionId;
use rutter_engine::config::LaunchMode;
use rutter_engine::supervisor::EngineLauncher;
use rutter_events::Event;
use rutter_policy::{ApprovalBroker, ApprovalId, Decision, RuleSet};
use rutter_session::config::SessionConfig;
use rutter_session::error::SessionError;
use rutter_session::manager::SessionManager;

/// A manager over one scripted launcher with the default policy; the
/// same path production uses (manager → supervisor → session).
fn flow_manager() -> (SessionManager, Arc<FlowLauncher>) {
    let launcher = FlowLauncher::new();
    let manager = SessionManager::new(
        Arc::clone(&launcher) as Arc<dyn EngineLauncher>,
        LaunchMode::Headless,
        SessionConfig::default(),
        Arc::new(RuleSet::default_set()),
        Arc::new(ApprovalBroker::new()),
        None,
    );
    (manager, launcher)
}

#[tokio::test]
async fn navigate_seeds_a_page_and_returns_its_snapshot() {
    let (manager, launcher) = flow_manager();
    let session = manager
        .session(SessionId::new("s1"))
        .await
        .expect("session");

    let snapshot = session
        .execute(
            Action::Navigate {
                url: "https://example.com".to_owned(),
            },
            Origin::Human,
        )
        .await
        .expect("human actions bypass policy");

    // The snapshot names the page and shows the scripted element.
    assert_eq!(snapshot.url, "https://example.com");
    let text = snapshot.to_string();
    assert!(text.contains("Ok"), "scripted button is rendered: {text}");

    // The first execute lazily opened exactly one tracked page, and
    // it is the context's page.
    let listed = session.pages().await;
    assert_eq!(listed.len(), 1);
    assert!(listed[0].active, "the seeded page is active");
    assert_eq!(
        launcher.engine(0).context(0).page(0).url(),
        "https://example.com"
    );
}

#[tokio::test]
async fn set_cookies_reach_the_context_after_a_grant() {
    let (manager, launcher) = flow_manager();
    let session = manager
        .session(SessionId::new("s1"))
        .await
        .expect("session");
    let broker = manager.broker();

    // Subscribe before parking: the approval request lands on the
    // backbone, and the grant lets exactly the requested cookies through.
    let mut events = session.backbone().subscribe();
    let cookies = [Cookie {
        name: "session".to_owned(),
        value: "42".to_owned(),
        domain: "example.com".to_owned(),
        path: None,
        secure: false,
        http_only: false,
        same_site: None,
        expires: None,
    }];
    let (cookie_result, ()) = tokio::join!(session.set_cookies(&cookies), async {
        loop {
            match events.recv().await {
                Ok(envelope) => {
                    if let Event::ApprovalRequested { request_id, .. } = envelope.event {
                        assert!(
                            broker.decide(&ApprovalId::new(request_id), Decision::Grant),
                            "the open approval is granted"
                        );
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(error) => panic!("event stream closed: {error}"),
            }
        }
    },);
    cookie_result.expect("the granted approval lets the call through");

    let context = launcher.engine(0).context(0);
    let calls = context.set_cookie_calls();
    assert_eq!(calls.len(), 1, "exactly one batch reached the context");
    assert_eq!(calls[0][0].name, "session");
}

#[tokio::test]
async fn wait_for_resolves_when_the_text_appears() {
    let (manager, launcher) = flow_manager();
    let session = manager
        .session(SessionId::new("s1"))
        .await
        .expect("session");

    // The wait lazily opens the page; flip the text-wait flag once the
    // page exists so the wait resolves without burning its budget.
    let wait = session.wait_for("needle", Duration::from_secs(5));
    let (snapshot, ()) = tokio::join!(wait, async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        launcher.engine(0).context(0).page(0).set_found(true);
    },);
    let snapshot = snapshot.expect("the wait resolves");
    assert!(snapshot.to_string().contains("Ok"));
}

#[tokio::test]
async fn wait_for_timeout_names_the_requested_budget() {
    let (manager, _launcher) = flow_manager();
    let session = manager
        .session(SessionId::new("s1"))
        .await
        .expect("session");

    let error = session
        .wait_for("needle", Duration::from_millis(150))
        .await
        .expect_err("the needle never appears");
    match &error {
        SessionError::Action(ActionError::TimedOut { elapsed, .. }) => {
            assert_eq!(*elapsed, Duration::from_millis(150));
        }
        other => panic!("expected a TimedOut error, got {other:?}"),
    }
}

#[tokio::test]
async fn close_session_closes_the_underlying_context() {
    let (manager, launcher) = flow_manager();
    let session = manager
        .session(SessionId::new("s1"))
        .await
        .expect("session");
    let context = launcher.engine(0).context(0);

    // Navigate first so the session owns an open page, then close.
    session
        .execute(
            Action::Navigate {
                url: "https://example.com".to_owned(),
            },
            Origin::Human,
        )
        .await
        .expect("navigate");

    manager.close_session(&SessionId::new("s1")).await;
    assert!(context.is_closed(), "close tears the context down");
    let _ = session;
}
