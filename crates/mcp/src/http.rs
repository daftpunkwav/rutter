//! Streamable HTTP transport for the MCP server (blueprint §3:
//! "stdio first, streamable HTTP later" — this is the later).
//!
//! Boundary: transport mapping only. rmcp's `StreamableHttpService` is
//! a tower service with built-in `Host` header validation (DNS
//! rebinding, blueprint §7.7-grade checks); each MCP connection mints
//! its own rutter session through the shared manager. Binding stays on
//! the loopback address the caller passes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

use rutter_core::ids::SessionId;
use rutter_session::manager::SessionManager;

use crate::RutterMcp;

/// Serves MCP over streamable HTTP at `addr` until the process exits.
/// Each connecting client gets its own session (`http-<pid>-<n>`).
pub async fn serve_http(
    manager: Arc<SessionManager>,
    addr: std::net::SocketAddr,
) -> Result<(), String> {
    let manager_for_factory = Arc::clone(&manager);
    let counter = Arc::new(AtomicU64::new(0));
    let pid = std::process::id();
    let service = StreamableHttpService::new(
        move || {
            let serial = counter.fetch_add(1, Ordering::Relaxed);
            let session_id = SessionId::new(format!("http-{pid}-{serial}"));
            Ok(RutterMcp::new(Arc::clone(&manager_for_factory), session_id))
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );

    let app = axum::Router::new().route_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| format!("cannot bind {addr}: {error}"))?;
    eprintln!("rutter: MCP over streamable HTTP on http://{addr}/mcp");
    axum::serve(listener, app)
        .await
        .map_err(|error| format!("http server failed: {error}"))
}
