//! Token-budgeted accessibility-tree views of pages.
//!
//! Boundary: pure data plus its text rendering. Building a snapshot from
//! a serialized DOM tree lives in `rutter-observe`; culling and folding
//! policy is specified by `docs/snapshot-format.md`. The text
//! rendering follows the Playwright-compatible YAML style, one node per
//! line: `- button "Sign in" [ref=e17]`, children indented two spaces.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::reference::Reference;

/// A structured, token-budgeted view of one page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// URL of the page the snapshot was taken from.
    pub url: String,
    /// Root of the accessibility tree.
    pub root: SnapshotNode,
    /// Whether the snapshot was capped by a size or depth limit and
    /// therefore omits part of the page.
    pub truncated: bool,
}

/// One node of a snapshot's accessibility tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotNode {
    /// Accessible role of the element, for example `button`.
    pub role: String,
    /// Accessible name of the element, if it has one.
    pub name: Option<String>,
    /// Current value of the element, if it has one; carried as data for
    /// agents, not rendered into the text form.
    pub value: Option<String>,
    /// Stable handle to the element, present when the node is actionable.
    pub reference: Option<Reference>,
    /// Checked state, present only for elements that expose one.
    pub checked: Option<bool>,
    /// Whether the element reports itself as disabled.
    pub disabled: bool,
    /// For fold summary nodes: how many children were collapsed into
    /// this line (rendered as `× N`); `None` for regular nodes.
    pub folded_count: Option<u32>,
    /// Children of the node in tree order.
    pub children: Vec<SnapshotNode>,
}

impl SnapshotNode {
    /// Creates a leaf node with the given role.
    pub fn leaf(role: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            name: None,
            value: None,
            reference: None,
            checked: None,
            disabled: false,
            folded_count: None,
            children: Vec::new(),
        }
    }

    /// Creates a fold summary line for `count` collapsed children of
    /// the given role.
    pub fn fold_summary(role: impl Into<String>, count: u32) -> Self {
        Self {
            folded_count: Some(count),
            ..Self::leaf(role)
        }
    }
}

impl fmt::Display for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        render_node(f, &self.root, 0)
    }
}

/// Renders one node and its subtree; each node is a single line, children
/// are indented two spaces per level.
fn render_node(f: &mut fmt::Formatter<'_>, node: &SnapshotNode, depth: usize) -> fmt::Result {
    write!(f, "{:indent$}- {}", "", node.role, indent = depth * 2)?;
    if let Some(name) = &node.name {
        write!(f, " \"{}\"", escape(name))?;
    }
    if node.checked == Some(true) {
        write!(f, " [checked]")?;
    }
    if node.disabled {
        write!(f, " [disabled]")?;
    }
    if let Some(reference) = &node.reference {
        write!(f, " [ref={}]", reference)?;
    }
    if let Some(count) = node.folded_count {
        write!(f, " × {count}")?;
    }
    writeln!(f)?;
    for child in &node.children {
        render_node(f, child, depth + 1)?;
    }
    Ok(())
}

/// Escapes double quotes inside a rendered name.
fn escape(name: &str) -> String {
    name.replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(role: &str, name: Option<&str>) -> SnapshotNode {
        SnapshotNode {
            role: role.to_owned(),
            name: name.map(str::to_owned),
            value: None,
            reference: None,
            checked: None,
            disabled: false,
            folded_count: None,
            children: Vec::new(),
        }
    }

    #[test]
    fn renders_playwright_style_lines() {
        let mut logo = node("img", Some("Logo"));
        logo.reference = Some(Reference::new("e2"));
        let mut heading = node("heading", Some("Welcome"));
        heading.children = vec![logo];
        let mut button = node("button", Some("Sign in"));
        button.reference = Some(Reference::new("e17"));
        button.checked = Some(true);

        let mut disabled_button = node("button", Some("Delete"));
        disabled_button.reference = Some(Reference::new("e18"));
        disabled_button.disabled = true;

        let mut fold = node("listitem", None);
        fold.folded_count = Some(20);

        let snapshot = Snapshot {
            url: "https://example.com".to_owned(),
            root: SnapshotNode {
                children: vec![heading, button, disabled_button, fold],
                ..node("generic", None)
            },
            truncated: false,
        };

        let rendered = snapshot.to_string();
        assert_eq!(
            rendered,
            concat!(
                "- generic\n",
                "  - heading \"Welcome\"\n",
                "    - img \"Logo\" [ref=e2]\n",
                "  - button \"Sign in\" [checked] [ref=e17]\n",
                "  - button \"Delete\" [disabled] [ref=e18]\n",
                "  - listitem × 20\n"
            )
        );
    }

    #[test]
    fn fold_summary_lines_render_the_count() {
        let fold = SnapshotNode::fold_summary("listitem", 7);
        assert_eq!(fold.folded_count, Some(7));
        let snapshot = Snapshot {
            url: "https://example.com".to_owned(),
            root: fold,
            truncated: true,
        };
        assert_eq!(snapshot.to_string(), "- listitem × 7\n");
    }

    #[test]
    fn escapes_quotes_in_names() {
        let mut quoted = node("button", Some("Say \"hi\""));
        quoted.reference = Some(Reference::new("e1"));
        let snapshot = Snapshot {
            url: "https://example.com".to_owned(),
            root: quoted,
            truncated: false,
        };
        assert_eq!(
            snapshot.to_string(),
            "- button \"Say \\\"hi\\\"\" [ref=e1]\n"
        );
    }
}
