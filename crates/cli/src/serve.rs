//! `rutter serve`: the MCP server over stdio or streamable HTTP.
//!
//! Boundary: one connection is one session; the engine starts lazily on
//! the first tool call through [`launcher::LazyLauncher`], so the server
//! is MCP-ready before any engine I/O (blueprint §8.5, TOOL_SPEC §1).
//! stdio owns stdout — nothing else writes to it while serving.

use std::sync::Arc;

use rmcp::ServiceExt;
use rutter_core::ids::SessionId;
use rutter_engine::config::LaunchMode;
use rutter_mcp::RutterMcp;
use rutter_policy::{ApprovalBroker, RuleSet};
use rutter_session::SessionConfig;
use rutter_session::manager::SessionManager;

use crate::config::Settings;
use crate::error::CliError;
use crate::launcher;

/// Serves MCP until the client disconnects. Transports are mutually
/// exclusive: `--http ADDR` serves streamable HTTP, anything else
/// speaks stdio. The dashboard (optional port) and the policy file
/// (optional TOML) attach here.
pub async fn run(
    settings: &Settings,
    headed: bool,
    policy: Option<rutter_policy::RuleSet>,
    dashboard_port: Option<u16>,
    http_addr: Option<std::net::SocketAddr>,
) -> Result<(), CliError> {
    // Policy: the built-in conservative set unless a --policy TOML file
    // overrides it per deployment. Storage states persist under the
    // cache root (blueprint §7.4).
    let state_dir = Some(settings.cache_root.join("sessions"));
    let manager = Arc::new(SessionManager::new(
        Arc::new(launcher::LazyLauncher::new(settings.clone(), headed)),
        if headed {
            LaunchMode::Headed
        } else {
            LaunchMode::Headless
        },
        SessionConfig::default(),
        Arc::new(policy.unwrap_or_else(RuleSet::default_set)),
        Arc::new(ApprovalBroker::new()),
        state_dir,
    ));

    if let Some(port) = dashboard_port {
        let dashboard =
            rutter_dashboard::DashboardServer::new(Arc::clone(&manager), manager.broker(), port);
        tokio::spawn(async move {
            if let Err(error) = dashboard.run().await {
                eprintln!("rutter: {error}");
            }
        });
    }

    if let Some(addr) = http_addr {
        rutter_mcp::http::serve_http(manager, addr)
            .await
            .map_err(|message| CliError::Server { message })
    } else {
        let session_id = SessionId::new(format!("stdio-{}", std::process::id()));
        let server = RutterMcp::new(manager, session_id);
        let running = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| CliError::Server {
                message: error.to_string(),
            })?;
        running.waiting().await.map_err(|error| CliError::Server {
            message: error.to_string(),
        })?;
        Ok(())
    }
}
