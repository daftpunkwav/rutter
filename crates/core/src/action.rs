//! Typed operations an agent can request, plus the origin of an action.
//!
//! Boundary: the payload shapes here are the vocabulary shared by every
//! layer; the authoritative tool schema is `docs/TOOL_SPEC.md` once
//! written, and execution semantics (auto-wait, settlement, snapshots
//! after acting) live in `rutter-session`.

use serde::{Deserialize, Serialize};

use crate::reference::Reference;

/// Who initiated an action.
///
/// All origins share a single execution path and event timeline. The
/// origin decides attribution and whether approval rules apply:
/// agent-origin actions go through policy evaluation; human-origin
/// actions bypass approval and are recorded identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    /// Requested by an MCP client (the supervised path).
    Agent,
    /// Performed directly by a human (CLI diagnostics, dashboard manual
    /// control); bypasses approval, recorded on the same timeline.
    Human,
}

/// A typed operation requested against a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Action {
    /// Navigates the page to the given URL.
    Navigate {
        /// Absolute URL to load.
        url: String,
    },
    /// Goes back in the page's history.
    Back,
    /// Goes forward in the page's history.
    Forward,
    /// Reloads the current page.
    Reload,
    /// Clicks the element a snapshot reference points to.
    Click {
        /// Reference to the element to click.
        reference: Reference,
    },
    /// Hovers the element a snapshot reference points to.
    Hover {
        /// Reference to the element to hover.
        reference: Reference,
    },
    /// Types text into an element after focusing it.
    Type {
        /// Reference to the element that receives the text.
        reference: Reference,
        /// Text to type, interpreted as literal characters.
        text: String,
    },
    /// Presses a single key.
    PressKey {
        /// Key name in engine notation, for example `a`, `Enter`, `Tab`.
        key: String,
    },
    /// Selects option values on a select element.
    SelectOption {
        /// Reference to the select element.
        reference: Reference,
        /// Values of the options to select.
        values: Vec<String>,
    },
    /// Scrolls the page or an element by an amount in pixels.
    Scroll {
        /// Reference to the scroll container; `None` scrolls the page.
        reference: Option<Reference>,
        /// Direction of the scroll in page coordinates.
        direction: ScrollDirection,
        /// Distance to scroll, in pixels.
        amount: u32,
    },
}

/// Direction of an [`Action::Scroll`] operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrollDirection {
    /// Toward the top of the page.
    Up,
    /// Toward the bottom of the page.
    Down,
    /// Toward the left edge of the page.
    Left,
    /// Toward the right edge of the page.
    Right,
}
