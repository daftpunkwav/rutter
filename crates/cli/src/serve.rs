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

/// Serves MCP until the client disconnects or Ctrl-C arrives.
/// Transports are mutually exclusive: `--http ADDR` serves streamable
/// HTTP, anything else speaks stdio. The dashboard (optional port) and
/// the policy file (optional TOML) attach here. A non-loopback HTTP
/// bind must be confirmed with `--allow-remote`. Every exit path shuts
/// the manager down so the supervised browser never outlives the
/// server.
pub async fn run(
    settings: &Settings,
    headed: bool,
    policy: Option<rutter_policy::RuleSet>,
    dashboard_port: Option<u16>,
    http_addr: Option<std::net::SocketAddr>,
    allow_remote: bool,
) -> Result<(), CliError> {
    if let Some(addr) = http_addr {
        ensure_bind_is_consented(addr, allow_remote)?;
    }
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
        let dashboard = rutter_dashboard::DashboardServer::new(Arc::clone(&manager), port);
        tokio::spawn(async move {
            if let Err(error) = dashboard.run().await {
                eprintln!("rutter: {error}");
            }
        });
    }

    let result = if let Some(addr) = http_addr {
        serve_http(manager.clone(), addr).await
    } else {
        serve_stdio(manager.clone()).await
    };

    // Whether the client disconnected, Ctrl-C fired, or serving failed:
    // the engine dies with the process's last server run.
    manager.shutdown().await;
    result
}

/// Serves MCP over stdio until the client disconnects or Ctrl-C fires,
/// whichever comes first. Closing stdout (the client exiting) and the
/// interrupt both tear the service down and resolve this future.
async fn serve_stdio(manager: Arc<SessionManager>) -> Result<(), CliError> {
    let session_id = SessionId::new(format!("stdio-{}", std::process::id()));
    let server = RutterMcp::new(manager, session_id);
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|error| CliError::Transport {
            message: error.to_string(),
        })?;

    let interrupt = tokio::signal::ctrl_c();
    let wait = running.waiting();
    tokio::select! {
        result = wait => {
            result.map_err(|error| CliError::Transport {
                message: error.to_string(),
            })?;
        }
        result = interrupt => {
            result.map_err(|error| CliError::SignalHandling {
                mode: "serve".to_owned(),
                reason: format!("signal handling failed: {error}"),
            })?;
        }
    }
    Ok(())
}

/// Refuses a non-loopback HTTP bind unless the caller confirmed it:
/// the transport drives a real browser with this user's sessions and
/// has no authentication, so a silent LAN exposure is one typo away.
fn ensure_bind_is_consented(
    addr: std::net::SocketAddr,
    allow_remote: bool,
) -> Result<(), CliError> {
    if allow_remote || addr.ip().is_loopback() {
        return Ok(());
    }
    Err(CliError::RemoteHttpBind { addr })
}

/// Serves MCP over streamable HTTP until Ctrl-C fires. The interrupt
/// must be observed here because `serve_http` otherwise only returns
/// when the listener fails; without it, no Ctrl-C path could shut the
/// engine down.
async fn serve_http(
    manager: Arc<SessionManager>,
    addr: std::net::SocketAddr,
) -> Result<(), CliError> {
    let http = rutter_mcp::http::serve_http(manager, addr);
    let interrupt = tokio::signal::ctrl_c();
    tokio::select! {
        result = http => result.map_err(|message| CliError::Transport { message }),
        result = interrupt => {
            result.map_err(|error| CliError::SignalHandling {
                mode: "serve".to_owned(),
                reason: format!("signal handling failed: {error}"),
            })?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(text: &str) -> std::net::SocketAddr {
        text.parse().unwrap()
    }

    #[test]
    fn loopback_binds_need_no_confirmation() {
        ensure_bind_is_consented(addr("127.0.0.1:9800"), false)
            .expect("the default bind is loopback");
        ensure_bind_is_consented(addr("[::1]:9800"), false).expect("::1 is loopback too");
    }

    #[test]
    fn remote_binds_are_refused_without_the_flag() {
        let error = ensure_bind_is_consented(addr("192.168.1.10:9800"), false)
            .expect_err("a LAN bind is refused");
        assert!(error.to_string().contains("refusing"), "{error}");
        let error = ensure_bind_is_consented(addr("0.0.0.0:9800"), false)
            .expect_err("a wildcard bind is refused");
        assert!(error.to_string().contains("refusing"), "{error}");
    }

    #[test]
    fn remote_binds_pass_with_the_flag() {
        ensure_bind_is_consented(addr("192.168.1.10:9800"), true)
            .expect("explicit confirmation is accepted");
        ensure_bind_is_consented(addr("0.0.0.0:9800"), true)
            .expect("explicit confirmation is accepted");
    }
}
