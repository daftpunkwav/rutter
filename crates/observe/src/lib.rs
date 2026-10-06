//! Builds token-budgeted accessibility snapshots and markdown readouts
//! from serialized page content.
//!
//! Responsibilities:
//! - Expose the embedded page scripts (the serializer through
//!   [`serializer_script`], the reader through [`reader_script`]).
//! - Convert the serializer's response into a
//!   [`rutter_core::snapshot::Snapshot`] via [`snapshot_from_response`],
//!   with the pure conversion in the internal `snapshot_builder`.
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
mod read;
mod resolver;
mod response;
mod snapshot_builder;

pub use read::read_from_response;
pub use resolver::{
    element_script, files_check_script, focus_script, resolver_script, select_script,
    storage_dump_script, storage_restore_script, wait_for_script,
};
pub use response::snapshot_from_response;

/// Returns the embedded serializer script for evaluation inside a page.
///
/// The script walks the composed DOM including shadow roots, mints
/// element references, and returns the snapshot envelope. Evaluate it
/// with a page's `evaluate` and
/// pass the result to [`snapshot_from_response`].
///
/// It reads the ref scope from the minter
/// [`entropy_capture_script`] installs, and has no fallback: a page
/// evaluated without that capture reports itself truncated and hands out
/// no references. That is deliberate — a scope drawn from a source the
/// page could replace would let two documents mint the same one — so an
/// embedder that evaluates this script directly has to install the
/// capture first.
pub fn serializer_script() -> &'static str {
    assets::SERIALIZER_JS
}

/// Returns the embedded reader script for evaluation inside a page.
///
/// The script extracts the page's readable content as a markdown
/// document (site chrome and hidden elements omitted) and returns the
/// markdown envelope. Evaluate it with a page's `evaluate` and pass
/// the result to [`read_from_response`].
pub fn reader_script() -> &'static str {
    assets::READER_JS
}

/// Returns the embedded document-start entropy capture.
///
/// The serializer mints each document's ref scope from a random source,
/// and the scope is what keeps a stale reference captured on one page
/// from resolving to another page's element after a tab switch. The
/// page's own `crypto` is replaceable, so the engine installs this
/// script with `Page.addScriptToEvaluateOnNewDocument` — before the
/// document runs any script of its own — and the serializer calls the
/// scope minter it locks onto the global. A document that predates the
/// installation gets its minter from the engine's isolated world; a
/// document with no minter hands out no references.
///
/// The script is a no-op on a document without `crypto`, where the
/// serializer's store degrades instead.
pub fn entropy_capture_script() -> &'static str {
    assets::ENTROPY_JS
}
