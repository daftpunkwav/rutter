//! Shared domain vocabulary for all rutter crates.
//!
//! Responsibilities:
//! - Define the canonical types every layer speaks: actions, snapshots,
//!   references, identifiers, and the action error taxonomy.
//! - Keep those types stable and self-contained so higher layers can be
//!   changed or replaced without touching the vocabulary.
//!
//! Boundary: pure vocabulary only. This crate performs no I/O, contains no
//! async code, and knows nothing about engines or transport protocols.
//! The glossary in `docs/glossary.md` is normative for every name
//! defined here.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod action;
pub mod cookie;
pub mod error;
pub mod ids;
pub mod reference;
pub mod snapshot;
