//! Spec-pinning tests for the snapshot pipeline: golden snapshots
//! (insta), property tests (proptest), and per-rule unit tests.

use super::*;
use serde_json::json;

fn meta() -> PageMeta {
    PageMeta {
        viewport_height: Some(720.0),
    }
}

fn items(count: usize, name: Option<&str>) -> Vec<Value> {
    (0..count)
        .map(|_| json!({ "role": "listitem", "name": name, "rect": { "y": 10, "height": 20 } }))
        .collect()
}

#[test]
fn viewport_classification_uses_rect_extents_alone() {
    // Rects are viewport-relative (docs/snapshot-format.md §3); classification must not
    // depend on any scroll offset.
    let meta = meta();

    assert!(meta.in_viewport(100.0, 40.0), "in-view content stays");
    assert!(meta.in_viewport(880.0, 40.0), "bottom margin stays");
    assert!(meta.in_viewport(-180.0, 40.0), "top margin stays");
    assert!(meta.in_viewport(-220.0, 40.0), "partial overlap stays");
    assert!(!meta.in_viewport(-260.0, 50.0), "fully above folds");
    assert!(!meta.in_viewport(5000.0, 40.0), "far below folds");
    assert!(!meta.in_viewport(-900.0, 40.0), "far above folds");
    assert!(
        PageMeta::default().in_viewport(999_999.0, 40.0),
        "no viewport data disables culling"
    );
}

#[test]
fn converts_well_formed_trees() {
    let tree = json!({
        "role": "root",
        "children": [
            { "role": "heading", "name": "Welcome", "ref": "e1", "rect": { "y": 0, "height": 40 } },
            { "role": "list", "rect": { "y": 50, "height": 100 }, "children": items(2, Some("First")) }
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
            { "role": "button", "name": "OK", "disabled": true, "rect": { "y": 0, "height": 10 } }
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
    assert_eq!(
        snapshot.root.children.len(),
        1,
        "the oversized run collapses to one summary line"
    );
}

#[test]
fn long_nameless_sibling_runs_fold() {
    let tree = json!({
        "role": "root",
        "rect": { "y": 0, "height": 700 },
        "children": [
            { "role": "list", "rect": { "y": 10, "height": 500 }, "children": items(10, None) }
        ]
    });

    let snapshot = build("https://example.com", &meta(), &tree);
    let list = &snapshot.root.children[0];

    assert!(snapshot.truncated, "folding is a budget-induced change");
    assert_eq!(list.children.len(), 1, "the run collapses to one line");
    assert_eq!(list.children[0].role, "listitem");
    assert_eq!(list.children[0].folded_count, Some(10));
    assert!(list.children[0].children.is_empty());
}

#[test]
fn short_runs_and_named_items_stay_unfolded() {
    let tree = json!({
        "role": "root",
        "rect": { "y": 0, "height": 700 },
        "children": [
            { "role": "list", "rect": { "y": 10, "height": 500 }, "children": items(3, Some("Item")) }
        ]
    });

    let snapshot = build("https://example.com", &meta(), &tree);
    let list = &snapshot.root.children[0];

    assert!(!snapshot.truncated);
    assert_eq!(list.children.len(), 3);
}

#[test]
fn out_of_viewport_subtrees_fold() {
    // Named items do not collapse via sibling folding, so the whole
    // subtree stays large enough for viewport folding to matter.
    let children: Vec<Value> = (0..50)
        .map(|i| {
            json!({
                "role": "button", "name": format!("Unique button {i}"),
                "rect": { "y": 5000, "height": 20 }
            })
        })
        .collect();
    let tree = json!({
        "role": "root",
        "rect": { "y": 0, "height": 3000 },
        "children": [
            { "role": "section", "rect": { "y": 5000, "height": 2000 },
              "children": children }
        ]
    });

    let snapshot = build("https://example.com", &meta(), &tree);
    let section = &snapshot.root.children[0];

    assert!(snapshot.truncated);
    assert_eq!(section.children.len(), 0, "subtree folds below the fold");
    assert!(section.folded_count.is_some());
}

#[test]
fn in_viewport_content_survives_a_large_budget() {
    let tree = json!({
        "role": "root",
        "rect": { "y": 0, "height": 600 },
        "children": [
            { "role": "heading", "name": "Visible", "rect": { "y": 10, "height": 30 } }
        ]
    });

    let snapshot = build("https://example.com", &meta(), &tree);

    assert!(!snapshot.truncated);
    assert!(snapshot.to_string().contains("heading \"Visible\""));
}

#[test]
fn hard_budget_bounds_the_rendered_size() {
    // A wide in-viewport tree far beyond the 20k character budget.
    let children: Vec<Value> = (0..2000)
        .map(|i| {
            json!({
                "role": "button", "name": format!("Button number {i} in a long list"),
                "rect": { "y": i * 30, "height": 30 }
            })
        })
        .collect();
    let tree =
        json!({ "role": "root", "rect": { "y": 0, "height": 999999 }, "children": children });

    let snapshot = build("https://example.com", &meta(), &tree);

    assert!(snapshot.truncated);
    assert!(
        snapshot.to_string().chars().count() <= DEFAULT_BUDGET_CHARS,
        "rendered size must respect the budget"
    );
}

#[test]
fn hostile_strings_are_clamped() {
    let huge = "x".repeat(10_000);
    let tree = json!({
        "role": huge, "name": huge, "rect": { "y": 0, "height": 10 }
    });

    let snapshot = build("https://example.com", &meta(), &tree);

    assert!(snapshot.truncated);
    assert_eq!(snapshot.root.role.chars().count(), MAX_ROLE_CHARS);
    assert_eq!(
        snapshot.root.name.as_ref().map(|n| n.chars().count()),
        Some(MAX_TEXT_CHARS)
    );
}

// --- Golden tests: pinned YAML text form (insta inline snapshots). ---

#[test]
fn golden_folded_list_rendering() {
    let tree = json!({
        "role": "root",
        "rect": { "y": 0, "height": 700 },
        "children": [
            { "role": "list", "rect": { "y": 10, "height": 500 },
              "children": items(10, None) },
            { "role": "button", "name": "Sign in", "ref": "e17",
              "checked": true, "rect": { "y": 20, "height": 30 } }
        ]
    });

    let snapshot = build("https://example.com", &meta(), &tree);
    insta::assert_snapshot!(snapshot.to_string(), @r###"
    - generic
      - list
        - listitem × 10
      - button "Sign in" [checked] [ref=e17]
    "###);
}

#[test]
fn golden_escaping_and_flags() {
    let tree = json!({
        "role": "root",
        "rect": { "y": 0, "height": 700 },
        "children": [
            { "role": "button", "name": "Say \"hi\"", "ref": "e1", "rect": { "y": 0, "height": 20 } },
            { "role": "button", "name": "Delete", "ref": "e2", "disabled": true, "rect": { "y": 30, "height": 20 } }
        ]
    });

    let snapshot = build("https://example.com", &meta(), &tree);
    insta::assert_snapshot!(snapshot.to_string(), @r###"
    - generic
      - button "Say \"hi\"" [ref=e1]
      - button "Delete" [disabled] [ref=e2]
    "###);
}

// --- Property tests: hostile input invariants. ---

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    fn role_name() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("listitem".to_owned()),
            Just("button".to_owned()),
            "[a-z]{1,10}"
        ]
    }

    /// Arbitrary JSON node strategy: deep and wide enough that the
    /// rendered text regularly exceeds the 20k budget, so the size
    /// property is exercised against real folding pressure.
    fn arb_tree() -> impl Strategy<Value = Value> {
        let leaf = (role_name(), prop::option::of("[^\"]{0,160}"), 0.0f64..5000.0)
            .prop_map(|(role, name, y)| {
                json!({ "role": role, "name": name, "rect": { "y": y, "height": 10 } })
            });
        leaf.prop_recursive(6, 400, 24, |inner| {
            prop::collection::vec(inner, 4..40).prop_map(|children| {
                json!({ "role": "list", "rect": { "y": 0, "height": 10 }, "children": children })
            })
        })
    }

    proptest! {
        #[test]
        fn rendered_size_never_exceeds_budget(trees in prop::collection::vec(arb_tree(), 0..12)) {
            let meta = PageMeta {
                viewport_height: Some(720.0),
            };
            let tree = json!({ "role": "root", "rect": { "y": 0, "height": 5000 }, "children": trees });
            let snapshot = build("https://example.com", &meta, &tree);
            let rendered = snapshot.to_string();
            prop_assert!(
                rendered.chars().count() <= DEFAULT_BUDGET_CHARS,
                "rendered size {} exceeded the budget",
                rendered.chars().count()
            );
        }

        #[test]
        fn builder_survives_arbitrary_json(input in ".*") {
            let value: Value = serde_json::from_str(&input).unwrap_or(Value::String(input));
            let snapshot = build("https://example.com", &PageMeta::default(), &value);
            prop_assert!(!snapshot.to_string().is_empty());
        }
    }
}

// --- fit() branch pins: the 20k default budget makes these unreachable
// through `build`, so the unit under test is driven directly. ---

fn named(role: &str, name: &str, page_y: f64, height: f64) -> RawNode {
    RawNode {
        role: role.to_owned(),
        name: Some(name.to_owned()),
        page_y,
        height,
        ..RawNode::default()
    }
}

fn fresh_state() -> BuildState {
    BuildState {
        nodes: 0,
        truncated: false,
    }
}

#[test]
fn null_children_are_not_truncation() {
    // A null `children` reads as "no children", not as lost
    // information.
    let tree = json!({ "role": "button", "children": null });
    let snapshot = build("https://example.com", &meta(), &tree);
    assert!(!snapshot.truncated);
}

#[test]
fn fit_clears_children_when_the_own_line_exceeds_the_budget() {
    let mut state = fresh_state();
    let node = named("list", &"x".repeat(MAX_TEXT_CHARS), 0.0, 20.0);
    let mut node = RawNode {
        children: vec![RawNode::default()],
        ..node
    };
    let own = line_len(&node, 0);

    let used = fit(&mut node, 0, own - 1, &meta(), &mut state);

    assert_eq!(used, own, "the caller learns the true cost");
    assert!(node.children.is_empty(), "children over budget are cut");
    assert!(state.truncated);
}

#[test]
fn fit_folds_a_large_out_of_viewport_subtree_at_entry() {
    let mut state = fresh_state();
    // Deep below the viewport: a subtree over the minimum fold size
    // folds at the fit entry, whatever the budget says.
    let mut node = RawNode {
        children: vec![
            named("listitem", &"y".repeat(MAX_TEXT_CHARS), 5000.0, 30.0),
            named("listitem", &"z".repeat(MAX_TEXT_CHARS), 5000.0, 30.0),
        ],
        ..named("list", "wrap", 5000.0, 60.0)
    };
    assert!(
        !meta().in_viewport(5000.0, 60.0),
        "fixture must sit outside the viewport"
    );

    fit(&mut node, 0, DEFAULT_BUDGET_CHARS, &meta(), &mut state);

    assert_eq!(node.folded_count, Some(2), "the subtree folds to a summary");
    assert!(node.children.is_empty());
    assert!(state.truncated);
}

#[test]
fn fit_keeps_a_folded_child_that_still_fits() {
    let mut state = fresh_state();
    // Parent in view; one child far below with a subtree over the fold
    // minimum: the child folds, and the summary line is cheap enough to
    // keep.
    let child = RawNode {
        children: vec![
            named("listitem", &"y".repeat(MAX_TEXT_CHARS), 5000.0, 30.0),
            named("listitem", &"z".repeat(MAX_TEXT_CHARS), 5000.0, 30.0),
        ],
        ..named("list", "child", 5000.0, 60.0)
    };
    let mut parent = RawNode {
        children: vec![child],
        ..named("root", "parent", 0.0, 40.0)
    };

    fit(&mut parent, 0, DEFAULT_BUDGET_CHARS, &meta(), &mut state);

    assert_eq!(parent.children.len(), 1, "the folded child stays");
    assert_eq!(parent.children[0].folded_count, Some(2));
    assert!(state.truncated);
}
