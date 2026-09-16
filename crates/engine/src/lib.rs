//! The engine abstraction: traits and lifecycle types every browser
//! backend implements.
//!
//! Responsibilities:
//! - Define the [`Engine`], [`ContextHandle`], and [`PageHandle`]
//!   contracts, the only engine abstraction in the codebase.
//! - Carry launch configuration, engine descriptors, and health reports.
//! - Acquire engine binaries (downloader) and keep them alive
//!   (supervisor: heartbeat, restart with capped backoff, breaker).
//!
//! Boundary: no protocol-specific knowledge lives here; CDP details are
//! confined to `rutter-engine-cdp`. Session state restoration across
//! restarts is a session-layer concern and never happens here.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod backoff;
pub mod config;
pub mod context;
pub mod descriptor;
pub mod download;
pub mod engine;
pub mod error;
pub mod health;
pub mod input;
pub mod page;
pub mod supervisor;

pub use backoff::Backoff;
pub use config::{ContextConfig, LaunchMode};
pub use context::ContextHandle;
pub use descriptor::{EngineBackend, EngineCapabilities, EngineDescriptor};
pub use engine::Engine;
pub use error::EngineError;
pub use health::HealthReport;
pub use input::{InputEvent, MouseButton};
pub use page::{ImageFormat, PageHandle, Screenshot};
pub use supervisor::policy::RestartPolicy;
pub use supervisor::{EngineLauncher, Supervisor};
