//! Builds token-budgeted accessibility snapshots and markdown readouts
//! from serialized page content.
//!
//! Responsibilities:
//! - Expose the embedded page scripts (the serializer through
//!   [`serializer_script`], the reader through [`reader_script`]).
//! - Convert the serializer's response into a
//!   [`rutter_core::snapshot::Snapshot`] via [`snapshot_from_response`],
//!   with the pure conversion in the internal builder.
//! - Convert the reader's response into a
//!   [`rutter_core::readout::Readout`] via [`read_from_response`].
//!
//! Boundary: pure data transformation only. DOM serialization and
//! extraction (the injected scripts owned by this crate), engine
//! access, and action execution live in other crates. This crate
//! contains no async code and performs no I/O; callers inject the
//! scripts through an engine's `evaluate` and hand the JSON back here.
//! The page-side helper scripts for references, focus, select, wait,
//! and storage round-trips are re-exported below for the orchestration
//! layer.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod assets;
mod builder;
mod read;
mod resolver;
mod response;

pub use read::read_from_response;
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

/// Returns the embedded reader script for evaluation inside a page.
///
/// The script extracts the page's readable content as a markdown
/// document (site chrome and hidden elements omitted) and returns the
/// envelope described in `docs/read-format.md`. Evaluate it with a
/// page's `evaluate` and pass the result to [`read_from_response`].
pub fn reader_script() -> &'static str {
    assets::READER_JS
}
