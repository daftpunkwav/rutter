//! Functional tests for the event backbone through its public API:
//! ordered publish/subscribe, per-session replay, and ring isolation
//! between sessions.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use rutter_core::ids::SessionId;
use rutter_events::{Backbone, Event};

#[tokio::test]
async fn subscribers_receive_published_events_in_order() {
    let backbone = Backbone::new();
    let session = SessionId::new("s1");
    let mut subscriber = backbone.subscribe();

    backbone.publish(session.clone(), Event::SessionStarted);
    backbone.publish(session.clone(), Event::SessionClosed);

    let first = subscriber.recv().await.expect("first envelope");
    assert!(matches!(first.event, Event::SessionStarted));
    assert_eq!(first.session, session);
    let second = subscriber.recv().await.expect("second envelope");
    assert!(matches!(second.event, Event::SessionClosed));
    assert!(second.seq > first.seq, "sequence numbers advance");
}

#[tokio::test]
async fn replay_returns_a_sessions_history_oldest_first() {
    let backbone = Backbone::new();
    let session = SessionId::new("s1");

    backbone.publish(session.clone(), Event::SessionStarted);
    backbone.publish(
        session.clone(),
        Event::PageOpened {
            page: rutter_core::ids::PageId::new("p1"),
        },
    );
    backbone.publish(session.clone(), Event::SessionClosed);

    let replayed = backbone.replay(&session);
    assert_eq!(replayed.len(), 3, "every semantic event is kept");
    let seqs: Vec<u64> = replayed.iter().map(|envelope| envelope.seq).collect();
    assert!(
        seqs.windows(2).all(|pair| pair[0] < pair[1]),
        "replay is ordered by sequence: {seqs:?}"
    );
}

#[tokio::test]
async fn sessions_keep_separate_histories() {
    let backbone = Backbone::new();
    let first = SessionId::new("s1");
    let second = SessionId::new("s2");

    backbone.publish(first.clone(), Event::SessionStarted);
    backbone.publish(second.clone(), Event::SessionClosed);

    assert_eq!(backbone.replay(&first).len(), 1);
    assert_eq!(backbone.replay(&second).len(), 1);
    assert!(backbone.replay(&SessionId::new("s3")).is_empty());
}
