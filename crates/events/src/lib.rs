//! The typed event backbone: bus, per-session ring buffers, replay.
//!
//! Responsibilities:
//! - Define the [`event::Event`] vocabulary and the [`envelope::Envelope`]
//!   that carries it with a sequence number and RFC 3339 timestamp.
//! - Publish fire-and-forget on a tokio broadcast bus; publishing never
//!   blocks action execution (docs/events.md).
//! - Keep a bounded per-session ring so the dashboard can backfill
//!   history on connect through [`backbone::Backbone::replay`].
//!
//! Boundary: event data and its fan-out only. Emitting events is the
//! orchestration layer's job; screencast frames flow outside the
//! backbone (binary WebSocket frames, latest-wins backpressure).

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod backbone;
pub mod envelope;
pub mod event;

// The bus and the rings are `Backbone`'s implementation: consumers
// speak the backbone, the envelope, and the event vocabulary only.
mod bus;
mod ring;

pub use backbone::Backbone;
pub use envelope::Envelope;
pub use event::Event;
