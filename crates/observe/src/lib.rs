//! Builds token-budgeted accessibility snapshots from serialized DOM trees.
//!
//! Responsibilities:
//! - Convert the JSON tree reported by the page serializer into a
//!   [`rutter_core::Snapshot`] through the pure [`build`] function.
//!
//! Boundary: pure data transformation only. DOM serialization (the
//! injected script owned by this crate), engine access, and action
//! execution live in other crates. This crate contains no async code and
//! performs no I/O.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod builder;

pub use builder::build;
