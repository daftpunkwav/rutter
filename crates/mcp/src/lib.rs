//! MCP tool surface for rutter.
//!
//! Boundary: the rmcp host mapping `docs/tool-catalog.md` onto the session
//! layer. One MCP connection is one session; tools return snapshots as
//! text, action failures as `isError` results with a hint (docs/tool-catalog.md
//! §2). The server performs no I/O until the first tool call needs a
//! page (lazy engine, docs/architecture.md).

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod http;
pub mod server;

pub use server::RutterMcp;
