//! `rutter serve`: the MCP server over stdio or streamable HTTP.
//!
//! Boundary: one connection is one session; the engine starts lazily on
//! the first tool call through [`launcher::LazyLauncher`], so the server
//! is MCP-ready before any engine I/O.
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
    // cache root.
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
        // When stderr is a pipe — the MCP client's case — the dashboard
        // hands its access URL to a per-port file under the cache root
        // instead of printing it, so the supervised process does not
        // read the token by inheritance and two rutter processes on one
        // machine never race over one hand-off file.
        let access_dir = Some(settings.cache_root.clone());
        let dashboard =
            rutter_dashboard::DashboardServer::new(Arc::clone(&manager), port, access_dir);
        // The bind is awaited here, before serving is parked on its
        // task: a dashboard that cannot start (the port is taken) fails
        // the serve instead of leaving it to answer tool calls while
        // every approval waits out its window with nobody able to
        // answer it. Only a listener failure can end the spawned task,
        // and that one stays a report.
        let bound = dashboard
            .bind()
            .await
            .map_err(|message| CliError::Dashboard { message })?;
        tokio::spawn(async move {
            if let Err(error) = bound.serve().await {
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
        .map_err(|error| CliError::StdioTransport {
            message: error.to_string(),
        })?;

    serve_with_interrupt(running.waiting(), |error| CliError::StdioTransport {
        message: error.to_string(),
    })
    .await
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

/// Serves MCP over streamable HTTP until Ctrl-C fires.
async fn serve_http(
    manager: Arc<SessionManager>,
    addr: std::net::SocketAddr,
) -> Result<(), CliError> {
    // The interrupt must be observed here because `serve_http` otherwise
    // only returns when the listener fails; without it, no Ctrl-C path
    // could shut the engine down.
    serve_with_interrupt(rutter_mcp::http::serve_http(manager, addr), |message| {
        CliError::HttpTransport { message }
    })
    .await
}

/// Parks on one transport's service future until it ends or Ctrl-C
/// fires, whichever comes first. The interrupt and its
/// [`CliError::SignalHandling`] mapping are written once, so the two
/// transports cannot grow copies that drift from the shutdown contract
/// `run` relies on: every exit path out of here ends in `run`'s
/// `manager.shutdown()`.
async fn serve_with_interrupt<S, T, E>(
    service: S,
    map_service_error: impl Fn(E) -> CliError,
) -> Result<(), CliError>
where
    S: std::future::Future<Output = Result<T, E>>,
{
    let interrupt = tokio::signal::ctrl_c();
    tokio::select! {
        result = service => {
            result.map_err(map_service_error)?;
            Ok(())
        }
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
        // The flag is unconditional consent, not an extra restriction:
        // the loopback default keeps working with it set.
        ensure_bind_is_consented(addr("127.0.0.1:9800"), true)
            .expect("explicit confirmation is accepted");
    }
}
