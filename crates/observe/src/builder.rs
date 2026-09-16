//! Builds token-budgeted accessibility snapshots from serialized DOM trees.
//!
//! Responsibilities:
//! - Convert the JSON tree reported by the page serializer into a
//!   [`rutter_core::Snapshot`] through the pure [`build`] function.
//!   Token-budget culling and folding (spec §6) are separate steps in
//!   this file's pipeline.
//!
//! Boundary: pure data transformation only. DOM serialization (the
//! injected script owned by this crate), engine access, and action
//! execution live in other crates. This crate contains no async code and
//! performs no I/O. Arbitrary page data is untrusted input: malformed
//! fields degrade, limits are enforced, and the builder never panics.

use rutter_core::reference::Reference;
use rutter_core::snapshot::{Snapshot, SnapshotNode};
use serde_json::Value;

/// Maximum tree depth accepted from page data. A hard safety limit,
/// independent of token-budget culling (spec §6, rule 5).
const MAX_DEPTH: usize = 512;

/// Maximum number of nodes accepted from page data. A hard safety limit,
/// independent of token-budget culling (spec §6, rule 5).
const MAX_NODES: usize = 100_000;

/// Viewport and scroll position of the page a tree came from, in CSS
/// pixels. Unknown dimensions disable viewport-first culling; the
/// remaining budgets still apply.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageMeta {
    /// Viewport width, if the serializer reported one.
    pub viewport_width: Option<f64>,
    /// Viewport height, if the serializer reported one.
    pub viewport_height: Option<f64>,
    /// Horizontal scroll offset.
    pub scroll_x: f64,
    /// Vertical scroll offset.
    pub scroll_y: f64,
}

/// Build state threaded through the conversion.
struct BuildState {
    nodes: usize,
    truncated: bool,
}

/// Converts a serialized DOM tree into a snapshot for the given page URL.
pub fn build(url: impl Into<String>, meta: &PageMeta, tree: &Value) -> Snapshot {
    let _ = meta;
    let mut state = BuildState {
        nodes: 0,
        truncated: false,
    };
    let mut root = convert(tree, 0, &mut state);
    if root.role == "root" {
        // The serializer reports a synthetic root; agents see its
        // children directly under the snapshot root.
        root.role = "generic".to_owned();
    }
    Snapshot {
        url: url.into(),
        root,
        truncated: state.truncated,
    }
}

/// Converts one JSON node, degrading hostile input instead of failing.
fn convert(value: &Value, depth: usize, state: &mut BuildState) -> SnapshotNode {
    state.nodes += 1;

    let object = match value.as_object() {
        Some(object) => object,
        None => {
            state.truncated = true;
            return SnapshotNode::leaf("generic");
        }
    };

    if depth >= MAX_DEPTH {
        state.truncated = true;
        let role = role_of(object);
        return SnapshotNode::leaf(role);
    }

    let mut node = SnapshotNode {
        role: role_of(object),
        name: string_field(object, "name"),
        value: string_field(object, "value"),
        reference: string_field(object, "ref").map(Reference::new),
        checked: object.get("checked").and_then(Value::as_bool),
        disabled: object
            .get("disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        children: Vec::new(),
    };

    if let Some(children) = object.get("children") {
        match children.as_array() {
            Some(items) => {
                node.children.reserve(items.len().min(MAX_NODES));
                for item in items {
                    if state.nodes >= MAX_NODES {
                        state.truncated = true;
                        break;
                    }
                    node.children.push(convert(item, depth + 1, state));
                }
            }
            None if !children.is_null() => {
                // A `children` field that is neither an array nor null
                // means information was lost; flag it instead of guessing.
                state.truncated = true;
            }
            None => {}
        }
    }

    node
}

/// Reads the role, degrading to the generic role when absent or malformed.
fn role_of(object: &serde_json::Map<String, Value>) -> String {
    string_field(object, "role").unwrap_or_else(|| "generic".to_owned())
}

/// Reads a string field, degrading to `None` for any non-string value.
fn string_field(object: &serde_json::Map<String, Value>, field: &str) -> Option<String> {
    object.get(field).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta() -> PageMeta {
        PageMeta {
            viewport_width: Some(1280.0),
            viewport_height: Some(720.0),
            scroll_x: 0.0,
            scroll_y: 0.0,
        }
    }

    #[test]
    fn converts_well_formed_trees() {
        let tree = json!({
            "role": "root",
            "children": [
                { "role": "heading", "name": "Welcome", "ref": "e1" },
                {
                    "role": "list",
                    "children": [
                        { "role": "listitem", "name": "First" },
                        { "role": "listitem", "name": "Second" }
                    ]
                }
            ]
        });

        let snapshot = build("https://example.com", &meta(), &tree);

        assert_eq!(snapshot.url, "https://example.com");
        assert!(!snapshot.truncated);
        assert_eq!(snapshot.root.role, "generic", "synthetic root unwraps");
        assert_eq!(snapshot.root.children.len(), 2);
        assert_eq!(
            snapshot.root.children[0].reference,
            Some(Reference::new("e1"))
        );
    }

    #[test]
    fn degrades_malformed_fields() {
        let tree = json!({
            "role": 42,
            "name": false,
            "checked": "yes",
            "children": [
                { "role": "button", "name": "OK", "disabled": true }
            ]
        });

        let snapshot = build("https://example.com", &meta(), &tree);

        assert!(!snapshot.truncated);
        assert_eq!(snapshot.root.role, "generic");
        assert_eq!(snapshot.root.name, None);
        assert_eq!(snapshot.root.checked, None);
        let button = &snapshot.root.children[0];
        assert_eq!(button.role, "button");
        assert_eq!(button.name.as_deref(), Some("OK"));
        assert!(button.disabled);
    }

    #[test]
    fn non_object_input_flags_truncation() {
        let snapshot = build("https://example.com", &meta(), &json!("not a tree"));
        assert!(snapshot.truncated);
        assert_eq!(snapshot.root.role, "generic");
    }

    #[test]
    fn malformed_children_flag_truncation() {
        let tree = json!({ "role": "list", "children": "oops" });
        let snapshot = build("https://example.com", &meta(), &tree);
        assert!(snapshot.truncated);
        assert!(snapshot.root.children.is_empty());
    }

    #[test]
    fn over_deep_trees_are_cut_and_flagged() {
        let mut tree = json!({ "role": "leaf" });
        for _ in 0..(MAX_DEPTH + 10) {
            tree = json!({ "role": "branch", "children": [tree] });
        }

        let snapshot = build("https://example.com", &meta(), &tree);

        assert!(snapshot.truncated);
        // Recursion stops at the cap, so this must complete without
        // overflowing the stack.
        assert_eq!(snapshot.root.role, "branch");
    }

    #[test]
    fn oversized_trees_are_cut_and_flagged() {
        let children: Vec<Value> = (0..(MAX_NODES + 1))
            .map(|_| json!({ "role": "listitem" }))
            .collect();
        let tree = json!({ "role": "list", "children": children });

        let snapshot = build("https://example.com", &meta(), &tree);

        assert!(snapshot.truncated);
        assert_eq!(snapshot.root.children.len(), MAX_NODES - 1);
    }
}
