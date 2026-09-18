//! Functional tests for the observe crate through its public API: the
//! serializer envelope → snapshot conversion (including truncation
//! flags) and the embedded page scripts' contract markers.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use rutter_observe::{resolver_script, serializer_script, snapshot_from_response};

fn envelope() -> serde_json::Value {
    serde_json::json!({
        "version": 1.0,
        "truncated": false,
        "root": {
            "role": "root",
            "children": [
                { "role": "button", "name": "Ok", "ref": "e1" },
                { "role": "textbox", "name": "Email", "ref": "e2" }
            ]
        }
    })
}

#[test]
fn well_formed_envelopes_render_actionable_snapshots() {
    let snapshot = snapshot_from_response("https://example.com", &envelope());
    assert!(!snapshot.truncated, "an intact envelope is not truncated");
    let text = snapshot.to_string();
    assert!(text.contains("button \"Ok\" [ref=e1]"), "text: {text}");
    assert!(text.contains("textbox \"Email\" [ref=e2]"), "text: {text}");
}

#[test]
fn wrong_version_or_missing_root_flags_truncation() {
    let mut future = envelope();
    future["version"] = serde_json::json!(2.0);
    let snapshot = snapshot_from_response("https://example.com", &future);
    assert!(
        snapshot.truncated,
        "an envelope newer than this parser is flagged"
    );

    let mut headless = envelope();
    headless.as_object_mut().unwrap().remove("root");
    let snapshot = snapshot_from_response("https://example.com", &headless);
    assert!(snapshot.truncated, "a missing root is flagged, not a crash");
}

#[test]
fn string_envelopes_are_parsed_like_objects() {
    let text = serde_json::to_string(&envelope()).expect("serializable");
    let snapshot = snapshot_from_response("https://example.com", &serde_json::json!(text));
    assert!(!snapshot.truncated);
    assert!(snapshot.to_string().contains("[ref=e1]"));
}

#[test]
fn embedded_scripts_carry_the_contract_markers() {
    // The page doubles in session tests dispatch on these markers, and
    // the serializer envelope contract starts at MAX_NODES.
    assert!(serializer_script().contains("var MAX_NODES = "));
    assert!(resolver_script("e17").contains("var REF = "));
}
