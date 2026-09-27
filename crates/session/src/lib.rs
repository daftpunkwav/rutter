//! Session orchestration: contexts, pages, actions, auto-wait.
//!
//! Responsibilities:
//! - Own one supervised engine per process and hand out one
//! [`session::Session`] per MCP client.
//! - Execute typed [`rutter_core::action::Action`]s through the
//!   three-phase auto-wait (visible, stable, enabled — act — settle)
//!   and return a fresh snapshot for every
//! mutating action.
//! - Emit the event backbone's semantic events for everything a session
//! does.
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

// The element resolver, the polling loop, the page registry, and the
// observation feeds are the orchestration's mechanics; the surface above
// never sees them. Only the registry's client-facing view (`PageInfo`)
// and the feeds' entries (`Session::console_messages`) escape.
mod audit;
mod observations;
mod pages;
mod resolve;
mod wait;

pub use config::SessionConfig;
pub use error::SessionError;
pub use manager::SessionManager;
pub use pages::PageInfo;
pub use session::Session;
pub use storage::StorageState;

// Engine types this crate's own API exposes (the manager constructor,
// `Session::screencast`/`screenshot`, `SessionError::Engine`,
// `Session::console_messages`). Consumers depend on `rutter-session`
// alone; the engine layer stays an implementation detail behind the
// orchestration surface.
pub use rutter_engine::{
    ConsoleEntry, ConsoleLevel, EngineError, EngineLauncher, ImageFormat, LaunchMode, RequestEntry,
    ScreencastStream, Screenshot,
};
