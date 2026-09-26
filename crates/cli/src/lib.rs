//! The `rutter` CLI: argument resolution, entry modes, and process
//! lifecycle for the whole workspace.
//!
//! Responsibilities:
//! - Resolve flags and the environment into runtime [`config::Settings`].
//! - Dispatch the entry modes ([`entry`]): browse, serve, open, read.
//! - Own user-facing error presentation ([`error::CliError`] hints).
//!
//! Boundary: the CLI owns no engine or orchestration logic of its own;
//! it is the composition root that wires the other crates together and
//! the first consumer of their public APIs.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

// Only the pieces the thin binary and the tests/ targets consume are
// public; the mode implementations and the launcher wiring stay
// crate-internal (the composition root's assembly details).
pub mod config;
pub mod entry;
pub mod error;

mod browse;
mod launcher;
mod open;
mod read;
mod serve;
