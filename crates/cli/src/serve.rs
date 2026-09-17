//! `rutter serve`: the MCP server over stdio.
//!
//! Boundary: one stdio connection is one session; the engine starts
//! lazily on the first tool call (TOOL_SPEC §1). Nothing on stdout is
//! written by rutter itself while serving — the protocol owns it.

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

/// Serves MCP over stdio until the client disconnects; the dashboard
/// (optional port) and the policy file (optional TOML) attach here.
pub async fn run(
    settings: &Settings,
    headed: bool,
    policy: Option<rutter_policy::RuleSet>,
    dashboard_port: Option<u16>,
) -> Result<(), CliError> {
    let launcher = if headed {
        launcher::headed_launcher(settings).await?
    } else {
        launcher::headless_launcher(settings).await?
    };
    let mode = if headed {
        LaunchMode::Headed
    } else {
        LaunchMode::Headless
    };
    // Policy: the built-in conservative set unless a --policy TOML file
    // overrides it per deployment. Storage states persist under the
    // cache root (blueprint §7.4).
    let state_dir = Some(settings.cache_root.join("sessions"));
    let manager = Arc::new(SessionManager::new(
        Arc::new(launcher),
        mode,
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

    let session_id = SessionId::new(format!("stdio-{}", std::process::id()));
    let server = RutterMcp::new(Arc::clone(&manager), session_id);
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|error| CliError::Server {
            message: error.to_string(),
        })?;
    running.waiting().await.map_err(|error| CliError::Server {
        message: error.to_string(),
    })?;
    manager.shutdown().await;
    Ok(())
}
