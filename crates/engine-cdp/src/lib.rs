//! CDP engine backend for rutter, built on chromiumoxide.
//!
//! Boundary: the only crate in the workspace permitted to speak CDP
//! (docs/architecture.md). Everything above `rutter-engine`'s traits stays
//! protocol-agnostic; swapping or adding engines is confined to this
//! crate plus a registration point in the CLI. Chromiumoxide failures
//! are folded into [`rutter_engine::EngineError`] here; no CDP type
//! leaks across the trait boundary.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

// The implementation modules stay private on purpose: their items take
// chromiumoxide types, and the crate boundary is `EngineLauncher` alone
// (docs/architecture.md: no CDP type crosses a public signature).
mod context;
mod engine;
mod error;
mod launch;
mod page;

pub use launch::CdpLauncher;
