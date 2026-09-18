//! Tests for the `Session` type, run against the crate's mock engine
//! (`crate::mock`) — no real browser, no network.

use super::*;
use crate::mock::MockContext;
use rutter_core::ids::ContextId;

fn session_over(context: Arc<dyn ContextHandle>) -> Session {
    Session::new(
        SessionId::new("s-test"),
        context,
        Arc::new(Backbone::new()),
        SessionConfig::default(),
        Arc::new(RuleSet::default_set()),
        Arc::new(ApprovalBroker::new()),
        None,
    )
}

fn session_with_short_approval(context: Arc<dyn ContextHandle>) -> Session {
    Session::new(
        SessionId::new("s-test"),
        context,
        Arc::new(Backbone::new()),
        SessionConfig::default(),
        Arc::new(RuleSet::default_set().with_approval_timeout(Duration::from_millis(100))),
        Arc::new(ApprovalBroker::new()),
        None,
    )
}

#[tokio::test]
async fn close_closes_the_context_even_when_the_context_errors() {
    // The close contract: teardown always succeeds, and the real
    // context's `close` is the mechanism (a context that reports
    // "not found" must not wedge the session close either).
    struct FailingContext;
    #[async_trait::async_trait]
    impl ContextHandle for FailingContext {
        fn id(&self) -> ContextId {
            ContextId::new("ctx-failing")
        }
        fn pages(&self) -> Vec<PageId> {
            Vec::new()
        }
        async fn open_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
            Err(EngineError::Terminated)
        }
        fn page(&self, _id: PageId) -> Option<Arc<dyn PageHandle>> {
            None
        }
        async fn close_page(&self, _id: PageId) -> Result<(), EngineError> {
            Err(EngineError::Terminated)
        }
        async fn set_cookies(&self, _cookies: &[Cookie]) -> Result<(), EngineError> {
            Err(EngineError::Terminated)
        }
        async fn cookies(&self) -> Result<Vec<Cookie>, EngineError> {
            Err(EngineError::Terminated)
        }
        async fn close(&self) -> Result<(), EngineError> {
            Err(EngineError::Terminated)
        }
    }

    let session = session_over(Arc::new(FailingContext));
    session.close().await;
}

#[tokio::test]
async fn close_page_removes_the_page_and_promotes_a_remaining_one() {
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));

    let (page_id, _handle) = session.active_page_for_test().await;
    session
        .execute(
            Action::Navigate {
                url: "https://example.com".to_owned(),
            },
            Origin::Human,
        )
        .await
        .expect("navigate seeds the tracked url");

    // Sessions track a second page only through recovery; inject it
    // the way `recover` would (same-module test, private fields).
    let (other_id, other_handle) = context.open_page().await.expect("second page");
    session.lock_pages().push(PageSlot {
        id: other_id.clone(),
        url: "https://other.example".to_owned(),
        handle: other_handle,
        active: false,
    });

    // Closing the active page promotes the first remaining one.
    session
        .close_page(page_id.clone())
        .await
        .expect("close the page");
    let listed = session.pages().await;
    assert_eq!(listed.len(), 1);
    assert!(listed[0].active, "the remaining page must become active");
    assert_eq!(listed[0].id, other_id);
    assert!(
        session
            .backbone()
            .replay(&SessionId::new("s-test"))
            .iter()
            .any(|envelope| matches!(envelope.event, Event::PageClosed { .. })),
        "the close lands on the event backbone"
    );
}

#[tokio::test]
async fn select_page_switches_activity_and_unknown_pages_fail() {
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));

    let (first, _) = session.active_page_for_test().await;
    let (second, second_handle) = context.open_page().await.expect("second page");
    session.lock_pages().push(PageSlot {
        id: second.clone(),
        url: "https://other.example".to_owned(),
        handle: second_handle,
        active: false,
    });

    session
        .select_page(second.clone())
        .await
        .expect("select the second page");
    let listed = session.pages().await;
    assert!(
        listed.iter().all(|info| info.active == (info.id == second)),
        "exactly the selected page is active: {listed:?}"
    );

    let missing = session.select_page(PageId::new("nope")).await;
    assert!(
        matches!(
            missing,
            Err(SessionError::Action(ActionError::NotInteractable { .. }))
        ),
        "an unknown page is not interactable, not an engine failure"
    );
    let _ = first;
}

#[tokio::test]
async fn set_cookies_requires_approval_by_the_default_policy() {
    // The default rule set requires approval for the cookies class;
    // with nobody answering within the short test window, the call
    // must time out rather than silently writing the cookies.
    let context = MockContext::new();
    let session = session_with_short_approval(Arc::new(context.clone()));
    session.active_page_for_test().await;

    let outcome = session
        .set_cookies(&[Cookie {
            name: "session".to_owned(),
            value: "42".to_owned(),
            domain: "example.com".to_owned(),
            path: None,
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        }])
        .await;
    assert!(
        matches!(
            outcome,
            Err(SessionError::Action(ActionError::ApprovalTimedOut { .. }))
        ),
        "unanswered approvals park, they do not write: {outcome:?}"
    );
    assert!(
        context.set_cookie_calls().is_empty(),
        "no cookie reached the context without a grant"
    );
}

#[tokio::test]
async fn wait_for_survives_an_absurd_budget() {
    // A u64::MAX budget overflows `Instant + Duration` unless the
    // 600 s clamp runs before the deadline is built; the mock finds
    // the needle immediately, so the happy path exercises exactly
    // that deadline construction.
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));
    let (page_id, _page) = session.active_page_for_test().await;
    let page = context
        .page_mock(page_id)
        .expect("the active page is registered in the mock context");
    page.set_found(true);

    let snapshot = tokio::time::timeout(
        Duration::from_secs(5),
        session.wait_for("needle", Duration::from_millis(u64::MAX)),
    )
    .await
    .expect("the clamped wait must stay bounded by real time here")
    .expect("the wait resolves with a snapshot");
    assert!(snapshot.to_string().contains("Ok"));
}

#[tokio::test]
async fn wait_for_timeout_names_the_requested_budget() {
    // A timeout must report the budget the caller asked for, so the
    // agent can reason about what it just waited.
    let context = MockContext::new();
    let session = session_over(Arc::new(context));
    session.active_page_for_test().await;

    let error = session
        .wait_for("needle", Duration::from_millis(150))
        .await
        .expect_err("the needle never appears");
    match &error {
        SessionError::Action(ActionError::TimedOut { phase, elapsed }) => {
            assert_eq!(*phase, rutter_core::error::WaitPhase::Settle);
            assert_eq!(*elapsed, Duration::from_millis(150));
        }
        other => panic!("expected a TimedOut error, got {other:?}"),
    }
}
