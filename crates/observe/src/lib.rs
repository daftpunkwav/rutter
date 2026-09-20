//! Builds token-budgeted accessibility snapshots from serialized DOM trees.
//!
//! Responsibilities:
//! - Expose the embedded page serializer (through
//!   [`serializer_script`]).
//! - Convert its response into a [`rutter_core::snapshot::Snapshot`] via
//!   [`snapshot_from_response`], with the pure conversion in the
//!   internal builder.
//!
//! Boundary: pure data transformation only. DOM serialization (the
//! injected script owned by this crate), engine access, and action
//! execution live in other crates. This crate contains no async code and
//! performs no I/O; callers inject the script through an engine's
//! `evaluate` and hand the JSON back here. The page-side helper scripts
//! for references, focus, select, wait, and storage round-trips are
//! re-exported below for the orchestration layer.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod assets;
mod builder;
mod resolver;
mod response;

pub use resolver::{
    focus_script, resolver_script, select_script, storage_dump_script, storage_restore_script,
    wait_for_script,
};
pub use response::snapshot_from_response;

/// Returns the embedded serializer script for evaluation inside a page.
///
/// The script walks the composed DOM including shadow roots, mints
/// element references, and returns the envelope described in
/// `docs/snapshot-format.md`. Evaluate it with a page's `evaluate` and
/// pass the result to [`snapshot_from_response`].
pub fn serializer_script() -> &'static str {
    assets::SERIALIZER_JS
}
