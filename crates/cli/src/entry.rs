//! Entry modes of the rutter binary.
//!
//! The three modes (browse, serve, open) only set defaults; engine mode
//! and the dashboard are orthogonal flags on top of them. Boundary: mode
//! dispatch only — each mode's implementation lives in its own module
//! beside this one, and the CLI owns no engine logic beyond the launcher
//! registration.

use crate::config::Settings;
use crate::error::CliError;

/// What the binary was asked to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryMode {
    /// Headed engine window operated by a human; the no-argument
    /// default. The dashboard is a `serve --dashboard` option, not part
    /// of this mode.
    Browse,
    /// MCP server; engine headless by default.
    Serve {
        /// Whether the engine runs with a visible window.
        headed: bool,
    },
    /// One-shot diagnostic: navigate, print a snapshot, exit.
    Open {
        /// URL to navigate to.
        url: String,
    },
}

impl std::fmt::Display for EntryMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Browse => f.write_str("browse"),
            Self::Serve { .. } => f.write_str("serve"),
            Self::Open { .. } => f.write_str("open"),
        }
    }
}

/// Runs an entry mode to completion.
pub async fn run(
    mode: EntryMode,
    settings: &Settings,
    policy: Option<rutter_policy::RuleSet>,
    dashboard_port: Option<u16>,
    http_addr: Option<std::net::SocketAddr>,
) -> Result<(), CliError> {
    match mode {
        EntryMode::Browse => crate::browse::run(settings).await,
        EntryMode::Open { url } => crate::open::run(settings, &url).await,
        EntryMode::Serve { headed } => {
            crate::serve::run(settings, headed, policy, dashboard_port, http_addr).await
        }
    }
}
