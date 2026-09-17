//! Serializer envelope parsing: page JSON to snapshot conversion entry.
//!
//! Boundary: adapts the in-page serializer's envelope (spec section 2)
//! to the pure builder. The caller evaluates `serializer_script()` in a
//! page and hands the returned JSON here; the engine stays out of this
//! crate entirely.

use serde_json::Value;

use rutter_core::snapshot::Snapshot;

use crate::builder::{self, PageMeta};

/// Serializer envelope format version accepted by this build.
const ENVELOPE_VERSION: f64 = 1.0;

/// Converts a serializer response into a snapshot for `url`.
///
/// The response may be the envelope object itself or a JSON string
/// containing it. Missing or malformed envelope parts degrade: an
/// unreadable response yields an empty, truncated snapshot rather than
/// an error; an unknown viewport disables viewport-first culling.
pub fn snapshot_from_response(url: &str, response: &Value) -> Snapshot {
    let envelope = match response {
        Value::Object(_) => response.clone(),
        Value::String(text) => serde_json::from_str::<Value>(text).unwrap_or(Value::Null),
        _ => Value::Null,
    };

    let empty = Value::Object(Default::default());
    let object = envelope.as_object();

    let version_ok = object
        .and_then(|object| object.get("version"))
        .and_then(Value::as_f64)
        .is_none_or(|version| version <= ENVELOPE_VERSION);
    let serializer_flagged = object
        .and_then(|object| object.get("truncated"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let meta = PageMeta {
        viewport_width: object
            .and_then(|object| object.get("viewport"))
            .and_then(|viewport| viewport.get("width"))
            .and_then(Value::as_f64)
            .map(|width| width.max(0.0)),
        viewport_height: object
            .and_then(|object| object.get("viewport"))
            .and_then(|viewport| viewport.get("height"))
            .and_then(Value::as_f64)
            .map(|height| height.max(0.0)),
    };

    let root_present = object
        .and_then(|object| object.get("root"))
        .is_some_and(Value::is_object);
    let root = object
        .and_then(|object| object.get("root"))
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| empty.clone());

    let mut snapshot = builder::build(url, &meta, &root);
    snapshot.truncated = snapshot.truncated || serializer_flagged || !version_ok || !root_present;
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_envelope_object() {
        let response = json!({
            "version": 1,
            "truncated": false,
            "viewport": { "width": 800, "height": 600 },
            "scroll": { "x": 0, "y": 120 },
            "root": {
                "role": "root",
                "children": [{ "role": "button", "name": "Go", "ref": "e1" }]
            }
        });

        let snapshot = snapshot_from_response("https://example.com", &response);
        assert!(!snapshot.truncated);
        assert_eq!(snapshot.root.role, "generic", "synthetic root unwraps");
        assert_eq!(snapshot.root.children[0].name.as_deref(), Some("Go"));
    }

    #[test]
    fn accepts_json_string_envelope() {
        let response = Value::String(
            r#"{"version":1,"truncated":false,"root":{"role":"root","children":[]}}"#.to_owned(),
        );
        let snapshot = snapshot_from_response("https://example.com", &response);
        assert!(!snapshot.truncated);
        assert_eq!(snapshot.root.role, "generic", "synthetic root unwraps");
    }

    #[test]
    fn garbage_degrades_to_empty_truncated_snapshot() {
        let snapshot = snapshot_from_response("https://example.com", &json!(42));
        assert!(snapshot.truncated);
        assert_eq!(snapshot.root.role, "generic");
        assert!(snapshot.root.children.is_empty());
    }

    #[test]
    fn serializer_truncation_flag_propagates() {
        let response = json!({
            "version": 1,
            "truncated": true,
            "root": { "role": "root", "children": [] }
        });
        let snapshot = snapshot_from_response("https://example.com", &response);
        assert!(snapshot.truncated);
    }

    #[test]
    fn future_version_flags_truncation() {
        let response = json!({
            "version": 99,
            "truncated": false,
            "root": { "role": "root", "children": [] }
        });
        let snapshot = snapshot_from_response("https://example.com", &response);
        assert!(
            snapshot.truncated,
            "unknown envelope version must be flagged"
        );
    }
}
