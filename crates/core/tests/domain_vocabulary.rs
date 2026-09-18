//! Functional tests for the core domain vocabulary through its public
//! API: action serde round-trips, the error taxonomy's event-convention
//! serialization, and reference identity.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::time::Duration;

use rutter_core::action::Action;
use rutter_core::error::{ActionError, WaitPhase};
use rutter_core::reference::Reference;
use rutter_core::snapshot::{Snapshot, SnapshotNode};

#[test]
fn actions_round_trip_through_json() {
    let action = Action::Navigate {
        url: "https://example.com".to_owned(),
    };
    let json = serde_json::to_string(&action).expect("serializable");
    let back: Action = serde_json::from_str(&json).expect("deserializable");
    assert_eq!(back, action);

    let click = Action::Click {
        reference: Reference::new("e17"),
    };
    let json = serde_json::to_string(&click).expect("serializable");
    let back: Action = serde_json::from_str(&json).expect("deserializable");
    assert_eq!(back, click);
}

#[test]
fn action_errors_serialize_with_the_event_convention() {
    // Events carry errors, and the event vocabulary is a "type" tag
    // with snake_case names — consumers (the dashboard) key off it.
    let error = ActionError::TimedOut {
        phase: WaitPhase::Stable,
        elapsed: Duration::from_millis(1500),
    };
    let value = serde_json::to_value(&error).expect("serializable");
    assert_eq!(value["type"], "timed_out", "tagged object: {value}");

    let error = ActionError::Internal {
        detail: "boom".to_owned(),
    };
    let value = serde_json::to_value(&error).expect("serializable");
    assert_eq!(value["type"], "internal", "tagged object: {value}");
}

#[test]
fn references_are_opaque_but_distinct() {
    let a = Reference::new("e1");
    let b = Reference::new("e2");
    assert_eq!(a, a);
    assert_ne!(a, b);
    assert_eq!(a.as_str(), "e1");
}

#[test]
fn snapshots_render_their_nodes_as_lines() {
    let snapshot = Snapshot {
        url: "https://example.com".to_owned(),
        root: SnapshotNode::leaf("button"),
        truncated: false,
    };
    let text = snapshot.to_string();
    assert!(text.contains("button"), "the node renders: {text}");
}
