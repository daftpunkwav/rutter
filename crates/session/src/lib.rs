//! Session orchestration: contexts, pages, actions, auto-wait.
//!
//! Responsibilities:
//! - Own one supervised engine per process and hand out one
//!   [`session::Session`] per MCP client (blueprint §4, §7.8).
//! - Execute typed [`rutter_core::action::Action`]s through the
//!   three-phase auto-wait (visible, stable, enabled — act — settle) and
//!   return a fresh snapshot for every mutating action (blueprint §7.3,
//!   `docs/TOOL_SPEC.md` §3).
//! - Emit the event backbone's semantic events for everything a session
//!   does (blueprint §7.5).
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
pub mod resolve;
pub mod session;
pub mod storage;
pub mod wait;

pub use config::SessionConfig;
pub use error::SessionError;
pub use manager::SessionManager;
pub use session::{PageInfo, Session};
pub use storage::StorageState;
