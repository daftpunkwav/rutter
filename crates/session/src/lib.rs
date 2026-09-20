//! Session orchestration: contexts, pages, actions, auto-wait.
//!
//! Responsibilities:
//! - Own one supervised engine per process and hand out one
//!   [`session::Session`] per MCP client (docs/architecture.md, docs/tool-catalog.md).
//! - Execute typed [`rutter_core::action::Action`]s through the
//!   three-phase auto-wait (visible, stable, enabled — act — settle) and
//!   return a fresh snapshot for every mutating action (docs/tool-catalog.md §3).
//! - Emit the event backbone's semantic events for everything a session
//!   does (docs/events.md).
//!
//! Boundary: the only place where engine, observation, and events meet.
//! Policy verdicts and approval flow through this crate's executor;
//! storage-state persistence and recovery replay happen here too.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod actions;
pub mod config;
pub mod error;
pub mod manager;
#[cfg(test)]
pub(crate) mod mock;
pub mod session;
pub mod storage;

// The element resolver and the polling loop are the executor's
// mechanics; the orchestration surface above never sees them.
mod resolve;
mod wait;

pub use config::SessionConfig;
pub use error::SessionError;
pub use manager::SessionManager;
pub use session::{PageInfo, Session};
pub use storage::StorageState;

// Engine types this crate's own API exposes (the manager constructor,
// `Session::screencast`/`screenshot`, `SessionError::Engine`).
// Consumers depend on `rutter-session` alone; the engine layer stays an
// implementation detail behind the orchestration surface.
pub use rutter_engine::{
    EngineError, EngineLauncher, ImageFormat, LaunchMode, ScreencastStream, Screenshot,
};
