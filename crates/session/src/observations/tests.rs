//! Feed mechanics: the bounded buffer, per-page isolation, teardown,
//! and full passes of a dialog and console lines through a spawned feed
//! over the crate's mock page.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use rutter_core::ids::SessionId;
use rutter_engine::context::ContextHandle;
use rutter_engine::page::{ConsoleEntry, ConsoleLevel, DialogKind, PageObservation, RequestEntry};
use rutter_events::Backbone;

use super::{FEED_CAPACITY, ObservationFeeds, push};
use crate::mock::MockContext;

#[test]
fn the_buffer_keeps_the_newest_capacity_entries() {
    let mut entries = VecDeque::new();
    for index in 0..FEED_CAPACITY + 10 {
        push(
            &mut entries,
            ConsoleEntry {
                level: ConsoleLevel::Log,
                text: format!("line-{index}"),
            },
        );
    }
    assert_eq!(entries.len(), FEED_CAPACITY);
    assert_eq!(
        entries.front().map(|entry| entry.text.as_str()),
        Some("line-10")
    );
    assert_eq!(
        entries.back().map(|entry| entry.text.as_str()),
        Some(format!("line-{}", FEED_CAPACITY + 9).as_str())
    );
}

fn session_id() -> SessionId {
    SessionId::new("s-feed")
}

/// Waits until `condition` holds or the budget runs out; the feed's
/// task runs on its own schedule.
async fn wait_for(condition: impl Fn() -> bool) {
    crate::wait::poll_until(
        || async { if condition() { Some(()) } else { None } },
        Duration::from_secs(5),
        Duration::from_millis(20),
    )
    .await
    .expect("condition holds within the budget");
}

#[tokio::test(flavor = "current_thread")]
async fn a_dialog_is_dismissed_and_recorded() {
    let context = MockContext::new();
    let (page_id, _handle) = context.open_page().await.expect("mock open");
    let mock = context.page_mock(page_id.clone()).expect("mock page");

    let feeds = Arc::new(ObservationFeeds::default());
    feeds.spawn(
        session_id(),
        page_id.clone(),
        context.page_mock(page_id.clone()).expect("mock page"),
        Arc::new(Backbone::new()),
    );
    wait_for(|| mock.feed_claimed()).await;

    mock.emit(PageObservation::DialogOpened {
        kind: DialogKind::Confirm,
        message: "leave?".to_owned(),
    });

    wait_for(|| !mock.dialog_answers().is_empty()).await;
    assert_eq!(
        mock.dialog_answers(),
        vec![(false, None)],
        "dialogs are dismissed, prompts keep their default"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn console_lines_land_in_the_page_buffer_and_clear_stops_the_feed() {
    let context = MockContext::new();
    let (page_id, _handle) = context.open_page().await.expect("mock open");
    let mock = context.page_mock(page_id.clone()).expect("mock page");

    let feeds = Arc::new(ObservationFeeds::default());
    feeds.spawn(
        session_id(),
        page_id.clone(),
        context.page_mock(page_id.clone()).expect("mock page"),
        Arc::new(Backbone::new()),
    );
    wait_for(|| mock.feed_claimed()).await;

    mock.emit(PageObservation::ConsoleEmitted {
        level: ConsoleLevel::Warning,
        text: "careful".to_owned(),
    });
    mock.emit(PageObservation::UncaughtException {
        text: "boom".to_owned(),
    });

    wait_for(|| feeds.entries(&page_id).len() == 2).await;
    let entries = feeds.entries(&page_id);
    assert_eq!(entries[0].level, ConsoleLevel::Warning);
    assert_eq!(entries[0].text, "careful");
    assert_eq!(
        entries[1].level,
        ConsoleLevel::Error,
        "exceptions record as errors"
    );
    assert_eq!(entries[1].text, "boom");

    feeds.clear();
    assert!(feeds.entries(&page_id).is_empty());
    // A cleared feed no longer records: its task is stopped.
    mock.emit(PageObservation::ConsoleEmitted {
        level: ConsoleLevel::Log,
        text: "after clear".to_owned(),
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(feeds.entries(&page_id).is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn feeds_are_isolated_per_page_and_removable() {
    let context = MockContext::new();
    let (first, _handle) = context.open_page().await.expect("mock open");
    let (second, _handle) = context.open_page().await.expect("mock open");
    let first_mock = context.page_mock(first.clone()).expect("mock page");
    let second_mock = context.page_mock(second.clone()).expect("mock page");

    let feeds = Arc::new(ObservationFeeds::default());
    feeds.spawn(
        session_id(),
        first.clone(),
        context.page_mock(first.clone()).expect("mock page"),
        Arc::new(Backbone::new()),
    );
    feeds.spawn(
        session_id(),
        second.clone(),
        context.page_mock(second.clone()).expect("mock page"),
        Arc::new(Backbone::new()),
    );
    wait_for(|| first_mock.feed_claimed() && second_mock.feed_claimed()).await;

    first_mock.emit(PageObservation::ConsoleEmitted {
        level: ConsoleLevel::Info,
        text: "one".to_owned(),
    });
    second_mock.emit(PageObservation::ConsoleEmitted {
        level: ConsoleLevel::Error,
        text: "two".to_owned(),
    });

    wait_for(|| feeds.entries(&first).len() == 1 && feeds.entries(&second).len() == 1).await;
    assert_eq!(feeds.entries(&first)[0].text, "one");

    feeds.remove(&first);
    assert!(feeds.entries(&first).is_empty());
    first_mock.emit(PageObservation::ConsoleEmitted {
        level: ConsoleLevel::Log,
        text: "after removal".to_owned(),
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        feeds.entries(&first).is_empty(),
        "a removed feed stays dead"
    );
    assert_eq!(
        feeds.entries(&second).len(),
        1,
        "the other page keeps its feed"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn network_requests_land_in_the_page_buffer() {
    let context = MockContext::new();
    let (page_id, _handle) = context.open_page().await.expect("mock open");
    let mock = context.page_mock(page_id.clone()).expect("mock page");

    let feeds = Arc::new(ObservationFeeds::default());
    feeds.spawn(
        session_id(),
        page_id.clone(),
        context.page_mock(page_id.clone()).expect("mock page"),
        Arc::new(Backbone::new()),
    );
    wait_for(|| mock.feed_claimed()).await;

    mock.emit(PageObservation::RequestObserved {
        entry: RequestEntry {
            method: "GET".to_owned(),
            url: "https://a.example/x".to_owned(),
            status: Some(200),
            resource_type: Some("fetch".to_owned()),
            error: None,
        },
    });
    mock.emit(PageObservation::RequestObserved {
        entry: RequestEntry {
            method: "GET".to_owned(),
            url: "https://a.example/missing".to_owned(),
            status: None,
            resource_type: Some("fetch".to_owned()),
            error: Some("net::ERR_NAME_NOT_RESOLVED".to_owned()),
        },
    });

    wait_for(|| feeds.requests(&page_id).len() == 2).await;
    let requests = feeds.requests(&page_id);
    assert_eq!(requests[0].status, Some(200));
    assert_eq!(
        requests[1].error.as_deref(),
        Some("net::ERR_NAME_NOT_RESOLVED")
    );
    assert!(feeds.entries(&page_id).is_empty(), "rings stay separate");
}
