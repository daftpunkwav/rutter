//! Builds token-budgeted accessibility snapshots from serialized DOM trees.
//!
//! Responsibilities:
//! - Convert the JSON tree reported by the page serializer into a
//!   [`rutter_core::Snapshot`] through the pure [`build`] function,
//!   applying the token budget of `docs/snapshot-format.md` §6: depth
//!   budget, sibling folding, viewport-first culling, and the hard
//!   character budget, in that order.
//!
//! Boundary: pure data transformation only. DOM serialization (the
//! injected script owned by this crate), engine access, and action
//! execution live in other crates. This crate contains no async code and
//! performs no I/O. Arbitrary page data is untrusted input: malformed
//! fields degrade, oversized strings are clamped, and the builder never
//! panics.

use rutter_core::reference::Reference;
use rutter_core::snapshot::{Snapshot, SnapshotNode};
use serde_json::Value;

/// Maximum tree depth accepted from page data. A hard safety limit,
/// independent of the token budget (docs/snapshot-format.md §6, rule 5).
const MAX_DEPTH: usize = 512;

/// Maximum number of nodes accepted from page data. A hard safety limit,
/// independent of the token budget (docs/snapshot-format.md §6, rule 5).
const MAX_NODES: usize = 100_000;

/// Depth budget under the token budget (docs/snapshot-format.md §6, rule 1).
const BUDGET_DEPTH: usize = 48;

/// Default token budget in rendered characters (docs/snapshot-format.md §6).
pub const DEFAULT_BUDGET_CHARS: usize = 20_000;

/// Sibling runs of at least this many same-role nameless children fold
/// into one summary line (docs/snapshot-format.md §6, rule 2).
const FOLD_RUN_MIN: usize = 8;

/// Margin around the viewport for out-of-viewport classification
/// (docs/snapshot-format.md §6, rule 3).
const VIEWPORT_MARGIN_PX: f64 = 200.0;

/// Minimum rendered size before an out-of-viewport subtree folds
/// (docs/snapshot-format.md §6, rule 3).
const FOLD_SIZE_MIN_CHARS: usize = 400;

/// Safety clamp for hostile overlong strings (docs/snapshot-format.md §3). The serializer
/// caps names at 120 and values at 200 characters; the builder clamps
/// anything larger rather than trusting the input.
const MAX_ROLE_CHARS: usize = 64;
const MAX_REF_CHARS: usize = 64;
const MAX_TEXT_CHARS: usize = 200;

/// Viewport height of the page a tree came from, in CSS pixels. Unknown
/// dimensions disable viewport-first culling; the remaining budgets
/// still apply.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageMeta {
    /// Viewport height, if the serializer reported one.
    pub viewport_height: Option<f64>,
}

impl PageMeta {
    /// Whether a subtree spanning `[page_y, page_y + height]` in
    /// viewport-relative coordinates intersects the viewport rect
    /// expanded by its margin.
    pub fn in_viewport(&self, page_y: f64, height: f64) -> bool {
        let Some(viewport_height) = self.viewport_height else {
            return true;
        };
        page_y <= viewport_height + VIEWPORT_MARGIN_PX && page_y + height >= -VIEWPORT_MARGIN_PX
    }
}

/// Internal tree carrying layout information needed for budgeting.
#[derive(Default)]
struct RawNode {
    role: String,
    name: Option<String>,
    value: Option<String>,
    reference: Option<Reference>,
    checked: Option<bool>,
    disabled: bool,
    folded_count: Option<u32>,
    page_y: f64,
    height: f64,
    children: Vec<RawNode>,
}

/// Build state threaded through the pipeline.
struct BuildState {
    nodes: usize,
    truncated: bool,
}

/// Converts a serialized DOM tree into a snapshot for the given page URL.
pub fn build(url: impl Into<String>, meta: &PageMeta, tree: &Value) -> Snapshot {
    let mut state = BuildState {
        nodes: 0,
        truncated: false,
    };
    let mut raw = convert(tree, 0, &mut state);
    if raw.role == "root" {
        // The serializer reports a synthetic root; agents see its
        // children directly under the snapshot root. Renamed before the
        // budget pipeline so size estimates match the final rendering.
        raw.role = "generic".to_owned();
    }
    raw = cut_depth(raw, BUDGET_DEPTH, &mut state);
    raw = fold_sibling_runs(raw, &mut state);
    fit(&mut raw, 0, DEFAULT_BUDGET_CHARS, meta, &mut state);

    Snapshot {
        url: url.into(),
        root: into_snapshot_node(raw),
        truncated: state.truncated,
    }
}

/// Converts one JSON node, degrading hostile input instead of failing.
fn convert(value: &Value, depth: usize, state: &mut BuildState) -> RawNode {
    state.nodes += 1;

    let object = match value.as_object() {
        Some(object) => object,
        None => {
            state.truncated = true;
            return leaf_raw("generic");
        }
    };

    if depth >= MAX_DEPTH {
        state.truncated = true;
        return leaf_raw(&role_of(object));
    }

    let (page_y, height) = object
        .get("rect")
        .and_then(Value::as_object)
        .map(|rect| {
            (
                rect.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                rect.get("height").and_then(Value::as_f64).unwrap_or(0.0),
            )
        })
        .unwrap_or((0.0, 0.0));

    let mut node = RawNode {
        role: clamp_string(role_of(object), MAX_ROLE_CHARS, state),
        name: string_field(object, "name").map(|name| clamp_string(name, MAX_TEXT_CHARS, state)),
        value: string_field(object, "value")
            .map(|value| clamp_string(value, MAX_TEXT_CHARS, state)),
        reference: string_field(object, "ref")
            .map(|reference| clamp_string(reference, MAX_REF_CHARS, state))
            .map(Reference::new),
        checked: object.get("checked").and_then(Value::as_bool),
        disabled: object
            .get("disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        folded_count: None,
        page_y,
        height,
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

/// Rule 1: cuts every branch deeper than `max_depth` below the root.
fn cut_depth(mut node: RawNode, max_depth: usize, state: &mut BuildState) -> RawNode {
    if max_depth == 0 {
        if !node.children.is_empty() {
            state.truncated = true;
        }
        node.children.clear();
        return node;
    }
    for child in &mut node.children {
        *child = cut_depth(std::mem::take(child), max_depth - 1, state);
    }
    node
}

/// Rule 2: folds runs of at least [`FOLD_RUN_MIN`] consecutive same-role
/// nameless, reference-less children into one summary line.
fn fold_sibling_runs(mut node: RawNode, state: &mut BuildState) -> RawNode {
    fn foldable(node: &RawNode) -> bool {
        node.name.is_none() && node.reference.is_none()
    }

    let children = std::mem::take(&mut node.children);
    let mut processed: Vec<RawNode> = Vec::with_capacity(children.len());
    let mut run: Vec<RawNode> = Vec::new();

    for child in children {
        let child = fold_sibling_runs(child, state);
        let joins_run = foldable(&child) && run.last().is_some_and(|head| head.role == child.role);
        if !joins_run && !run.is_empty() {
            flush_run(&mut run, &mut processed, state);
        }
        if !foldable(&child) {
            processed.push(child);
        } else {
            run.push(child);
        }
    }
    flush_run(&mut run, &mut processed, state);
    node.children = processed;
    node
}

/// Emits a finished run: folded when long enough, kept verbatim when not.
fn flush_run(run: &mut Vec<RawNode>, out: &mut Vec<RawNode>, state: &mut BuildState) {
    if run.is_empty() {
        return;
    }
    if run.len() >= FOLD_RUN_MIN {
        let head = &run[0];
        let summary = RawNode {
            role: head.role.clone(),
            page_y: head.page_y,
            height: run.iter().fold(0.0f64, |max, node| max.max(node.height)),
            folded_count: Some(run.len() as u32),
            ..leaf_raw("")
        };
        out.push(summary);
        state.truncated = true;
    } else {
        out.append(run);
    }
    run.clear();
}

/// Rules 3 and 4: spends the character budget in document order. A
/// subtree that no longer fits folds when it is entirely outside the
/// viewport (beyond the minimum fold size) or shrinks recursively; a
/// leaf or summary line that does not fit at all is dropped.
fn fit(
    node: &mut RawNode,
    depth: usize,
    budget: usize,
    meta: &PageMeta,
    state: &mut BuildState,
) -> usize {
    let own = line_len(node, depth);
    if own >= budget {
        if !node.children.is_empty() {
            node.children.clear();
            state.truncated = true;
        }
        return own;
    }

    // Rule 3: entirely out of the viewport and large enough to matter.
    if !node.children.is_empty()
        && !meta.in_viewport(node.page_y, node.height)
        && subtree_size(node, depth) > FOLD_SIZE_MIN_CHARS
    {
        fold_subtree(node, state);
        return own;
    }

    let children = std::mem::take(&mut node.children);
    let mut kept: Vec<RawNode> = Vec::with_capacity(children.len());
    let mut remaining = budget - own;

    for mut child in children {
        let child_size = subtree_size(&child, depth + 1);
        let child_line = line_len(&child, depth + 1);
        let out_of_view = !meta.in_viewport(child.page_y, child.height);

        // Rule 3: entirely out of the viewport and large enough to
        // matter — folds regardless of the remaining budget.
        if out_of_view && child_size > FOLD_SIZE_MIN_CHARS && !child.children.is_empty() {
            fold_subtree(&mut child, state);
            // Folding adds the `× N` suffix; measure after the fold.
            let summary_line = line_len(&child, depth + 1);
            if summary_line <= remaining {
                remaining -= summary_line;
                kept.push(child);
                continue;
            }
            state.truncated = true;
            continue;
        }

        if child_size <= remaining {
            remaining -= child_size;
            kept.push(child);
            continue;
        }

        if child.children.is_empty() || child_line >= remaining {
            state.truncated = true;
            continue;
        }
        let used = fit(&mut child, depth + 1, remaining, meta, state);
        if used > remaining {
            state.truncated = true;
            continue;
        }
        remaining -= used;
        kept.push(child);
    }

    node.children = kept;
    subtree_size(node, depth)
}

/// Replaces a subtree with its fold summary (`× N` counts the folded
/// nodes, excluding the summary line itself).
fn fold_subtree(node: &mut RawNode, state: &mut BuildState) {
    let count = count_nodes(node) - 1;
    node.children.clear();
    node.folded_count = Some(count as u32);
    state.truncated = true;
}

fn count_nodes(node: &RawNode) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

/// Estimated rendered size of one subtree in characters, matching the
/// `Display` implementation of `Snapshot` (char counts, not bytes).
fn subtree_size(node: &RawNode, depth: usize) -> usize {
    line_len(node, depth)
        + node
            .children
            .iter()
            .map(|child| subtree_size(child, depth + 1))
            .sum::<usize>()
}

/// Rendered length of one node's line, matching `Display` in
/// `rutter-core`.
fn line_len(node: &RawNode, depth: usize) -> usize {
    let mut len = 2 * depth + 2 + node.role.chars().count();
    if let Some(name) = &node.name {
        len += 3 + name.chars().count() + name.matches('"').count();
    }
    if node.checked == Some(true) {
        len += " [checked]".len();
    }
    if node.disabled {
        len += " [disabled]".len();
    }
    if let Some(reference) = &node.reference {
        len += " [ref=".len() + reference.as_str().chars().count() + 1;
    }
    if let Some(count) = node.folded_count {
        len += 3 + count.to_string().len();
    }
    len + 1 // newline
}

/// Converts the finished raw tree into the public snapshot node type.
fn into_snapshot_node(node: RawNode) -> SnapshotNode {
    SnapshotNode {
        role: node.role,
        name: node.name,
        value: node.value,
        reference: node.reference,
        checked: node.checked,
        disabled: node.disabled,
        folded_count: node.folded_count,
        children: node.children.into_iter().map(into_snapshot_node).collect(),
    }
}

/// Reads the role, degrading to the generic role when absent or malformed.
fn role_of(object: &serde_json::Map<String, Value>) -> String {
    string_field(object, "role").unwrap_or_else(|| "generic".to_owned())
}

/// Reads a string field, degrading to `None` for any non-string value.
fn string_field(object: &serde_json::Map<String, Value>, field: &str) -> Option<String> {
    object.get(field).and_then(Value::as_str).map(str::to_owned)
}

/// Clamps a hostile string to `limit` characters, flagging truncation.
fn clamp_string(value: String, limit: usize, state: &mut BuildState) -> String {
    if value.chars().count() <= limit {
        return value;
    }
    state.truncated = true;
    value.chars().take(limit).collect()
}

fn leaf_raw(role: &str) -> RawNode {
    RawNode {
        role: role.to_owned(),
        name: None,
        value: None,
        reference: None,
        checked: None,
        disabled: false,
        folded_count: None,
        page_y: 0.0,
        height: 0.0,
        children: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
