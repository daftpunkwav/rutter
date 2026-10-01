//! Observation-surface tests: the read-only probes (wait, screenshot,
//! dialog, console, network, cookies) that must never mutate a session.

use super::*;
#[tokio::test]
async fn wait_for_survives_an_absurd_budget() {
    // A u64::MAX budget overflows `Instant + Duration` unless the
    // 600 s clamp runs before the deadline is built; the mock finds
    // the needle immediately, so the happy path exercises exactly
    // that deadline construction.
    let context = MockContext::new();
    let session = default_session(Arc::new(context.clone()));
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
    let session = default_session(Arc::new(context));
    session.active_page_for_test().await;

    let error = session
        .wait_for("needle", Duration::from_millis(150))
        .await
        .expect_err("the needle never appears");
    match &error {
        SessionError::Action(ActionError::TimedOut { phase, elapsed }) => {
            assert_eq!(*phase, rutter_core::error::WaitPhase::Poll);
            assert_eq!(*elapsed, Duration::from_millis(150));
        }
        other => panic!("expected a TimedOut error, got {other:?}"),
    }
}

#[tokio::test]
async fn screenshot_capture_errors_map_to_the_internal_taxonomy() {
    // capture errors surface as `ActionError::Internal` —
    // the taxonomy every action failure uses — not as a raw engine
    // error.
    let session = default_session(Arc::new(SinglePageContext(Arc::new(BrokenLens))));
    let error = session
        .screenshot()
        .await
        .expect_err("the broken lens never captures");
    assert!(
        matches!(error, SessionError::Action(ActionError::Internal { .. })),
        "capture errors use the action taxonomy: {error:?}"
    );
}

#[tokio::test]
async fn watching_a_page_never_opens_one() {
    // The dashboard promises observation without effect. This used to run
    // through the same lookup an action uses, which opens a tab when the
    // session has none — so asking to watch created the thing watched.
    let context = MockContext::new();
    let session = default_session(Arc::new(context.clone()));

    assert!(
        matches!(session.screencast().await, Err(SessionError::NoOpenPage)),
        "the refusal must name the real cause, not an engine failure"
    );
    assert!(
        context.pages().is_empty(),
        "a refused observation opened a tab anyway"
    );
    assert!(
        session.pages().await.is_empty(),
        "the session gained a page it never had"
    );
}

#[tokio::test]
async fn a_page_dialog_is_dismissed_and_lands_on_the_timeline() {
    let context = MockContext::new();
    let session = default_session(Arc::new(context.clone()));
    let (page_id, _) = session.active_page_for_test().await;
    let mock = context.page_mock(page_id.clone()).expect("mock page");

    // The feed starts with the page; wait for its claim so the emitted
    // observation cannot race the feed's start-up.
    crate::wait::poll_until(
        || async { mock.feed_claimed().then_some(()) },
        Duration::from_secs(5),
        Duration::from_millis(20),
    )
    .await
    .expect("the feed claims the page's stream");

    mock.emit(rutter_engine::PageObservation::DialogOpened {
        kind: rutter_engine::DialogKind::Confirm,
        message: "leave?".to_owned(),
    });

    crate::wait::poll_until(
        || async { (!mock.dialog_answers().is_empty()).then_some(()) },
        Duration::from_secs(5),
        Duration::from_millis(20),
    )
    .await
    .expect("the dialog is answered");
    assert_eq!(mock.dialog_answers(), vec![(false, None)]);
    assert!(
        session
            .backbone()
            .replay(&SessionId::new("s-test"))
            .iter()
            .any(|envelope| matches!(
                envelope.event,
                Event::DialogAutoDismissed { ref kind, ref message, .. }
                    if kind == "confirm" && message == "leave?"
            )),
        "the dismissal is announced on the backbone"
    );
}

#[tokio::test]
async fn console_messages_reports_the_active_pages_entries() {
    let context = MockContext::new();
    let session = default_session(Arc::new(context.clone()));
    let (page_id, _) = session.active_page_for_test().await;
    let mock = context.page_mock(page_id.clone()).expect("mock page");

    assert!(
        session.console_messages().is_empty(),
        "a page with no console output reports none"
    );

    crate::wait::poll_until(
        || async { mock.feed_claimed().then_some(()) },
        Duration::from_secs(5),
        Duration::from_millis(20),
    )
    .await
    .expect("the feed claims the page's stream");

    mock.emit(rutter_engine::PageObservation::ConsoleEmitted {
        level: rutter_engine::ConsoleLevel::Warning,
        text: "careful".to_owned(),
    });
    mock.emit(rutter_engine::PageObservation::UncaughtException {
        text: "boom".to_owned(),
    });

    crate::wait::poll_until(
        || async { (!session.console_messages().is_empty()).then_some(()) },
        Duration::from_secs(5),
        Duration::from_millis(20),
    )
    .await
    .expect("the entries land");
    let entries = session.console_messages();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].level, rutter_engine::ConsoleLevel::Warning);
    assert_eq!(entries[0].text, "careful");
    assert_eq!(entries[1].level, rutter_engine::ConsoleLevel::Error);
    assert_eq!(entries[1].text, "boom");

    // A closed page's entries go with it.
    session.close_page(page_id).await.expect("page closes");
    assert!(session.console_messages().is_empty());
}

#[tokio::test]
async fn get_cookies_reads_the_context_without_opening_a_page() {
    let session = default_session(Arc::new(MockContext::new()));
    let cookies = session.cookies().await.expect("the context answers");
    assert!(cookies.is_empty(), "the fresh context has no cookies");
    assert!(
        session.registry().list().is_empty(),
        "a cookie read is observation and never opens a page"
    );
}

#[tokio::test]
async fn network_requests_reports_the_active_pages_entries() {
    let context = MockContext::new();
    let session = default_session(Arc::new(context.clone()));
    let (page_id, _) = session.active_page_for_test().await;
    let mock = context.page_mock(page_id.clone()).expect("mock page");

    assert!(session.network_requests().is_empty());

    crate::wait::poll_until(
        || async { mock.feed_claimed().then_some(()) },
        Duration::from_secs(5),
        Duration::from_millis(20),
    )
    .await
    .expect("the feed claims the page's stream");

    mock.emit(rutter_engine::PageObservation::RequestObserved {
        entry: rutter_engine::RequestEntry {
            method: "GET".to_owned(),
            url: "https://a.example/x".to_owned(),
            status: Some(200),
            resource_type: Some("fetch".to_owned()),
            error: None,
        },
    });

    crate::wait::poll_until(
        || async { (!session.network_requests().is_empty()).then_some(()) },
        Duration::from_secs(5),
        Duration::from_millis(20),
    )
    .await
    .expect("the request lands");
    let requests = session.network_requests();
    assert_eq!(
        requests[0].to_string(),
        "GET https://a.example/x -> 200 [fetch]"
    );

    // A closed page's requests go with it.
    session.close_page(page_id).await.expect("page closes");
    assert!(session.network_requests().is_empty());
}
