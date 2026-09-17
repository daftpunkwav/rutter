//! Builds token-budgeted accessibility snapshots from serialized DOM trees.
//!
//! Responsibilities:
//! - Expose the embedded page serializer ([`assets::SERIALIZER_JS`]).
//! - Convert its response into a [`Snapshot`] via
//!   [`response::snapshot_from_response`], with the pure conversion in
//!   [`builder`].
//!
//! Boundary: pure data transformation only. DOM serialization (the
//! injected script owned by this crate), engine access, and action
//! execution live in other crates. This crate contains no async code and
//! performs no I/O; callers inject the script through an engine's
//! `evaluate` and hand the JSON back here.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod assets;
pub mod builder;
pub mod resolver;
pub mod response;

pub use builder::PageMeta;
pub use resolver::{focus_script, resolver_script, select_script, wait_for_script};
pub use response::snapshot_from_response;

/// Returns the embedded serializer script for evaluation inside a page.
///
/// The script walks the composed DOM including shadow roots, mints
/// element references, and returns the envelope described in
/// `docs/SNAPSHOT_SPEC.md`. Evaluate it with a page's `evaluate` and
/// pass the result to [`snapshot_from_response`].
pub fn serializer_script() -> &'static str {
    assets::SERIALIZER_JS
}
