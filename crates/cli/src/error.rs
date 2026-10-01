//! Error surface of the CLI with agent- and human-readable hints.
//!
//! Boundary: presentation-level error folding. Engine errors keep their
//! identities and hints (defined with the error itself in
//! `rutter-engine`); the CLI adds only the signal-handling and
//! transport cases.

use thiserror::Error;

use rutter_engine::error::EngineError;

/// Why an invocation failed.
#[derive(Debug, Error)]
pub enum CliError {
    /// The engine layer failed (download, launch, navigation, ...).
    #[error("{source}")]
    Engine {
        /// The underlying engine failure.
        #[from]
        source: EngineError,
    },

    /// Watching for the interrupt signal failed, so the mode cannot
    /// guarantee a clean shutdown.
    #[error("{mode} could not watch for the interrupt signal: {reason}")]
    SignalHandling {
        /// The entry mode that was interrupted.
        mode: String,
        /// Why watching for the signal failed.
        reason: String,
    },

    /// The stdio MCP transport failed.
    #[error("mcp stdio transport failed: {message}")]
    StdioTransport {
        /// What went wrong with the transport or service.
        message: String,
    },

    /// The streamable HTTP MCP transport failed to bind or serve.
    #[error("mcp http transport failed: {message}")]
    HttpTransport {
        /// What went wrong with the listener or the HTTP service.
        message: String,
    },

    /// A non-loopback HTTP bind was requested without `--allow-remote`.
    #[error(
        "refusing to serve MCP HTTP on {addr}: the transport drives a real \
         browser with this user's sessions and has no authentication"
    )]
    RemoteHttpBind {
        /// The address that was refused.
        addr: std::net::SocketAddr,
    },

    /// The `--policy` file could not be read or parsed; the invocation
    /// stops before any mode runs.
    #[error("policy file {} could not be loaded: {reason}", path.display())]
    PolicyFile {
        /// The policy file that failed.
        path: std::path::PathBuf,
        /// What was wrong with it (read or parse error).
        reason: String,
    },

    /// The dashboard could not start (the port is taken, the bind
    /// failed, the access hand-off had nowhere to go). `serve` stops:
    /// without the dashboard nothing can answer its approvals, so
    /// running on would park every supervised action until its window
    /// closed with nobody watching.
    #[error("dashboard failed to start: {message}")]
    Dashboard {
        /// What went wrong on the way up.
        message: String,
    },
}

impl CliError {
    /// Returns an actionable, English hint for the failure.
    pub fn hint(&self) -> String {
        match self {
            Self::Engine { source } => source.hint().to_owned(),
            Self::SignalHandling { mode, .. } => {
                format!("'{mode}' was interrupted while handling a signal; rerun the command")
            }
            Self::StdioTransport { .. } => {
                "check that stdin/stdout are connected and not owned by another process".to_owned()
            }
            Self::HttpTransport { .. } => {
                "check that the --http address is correct and the port is not \
                 already held by another process, then retry"
                    .to_owned()
            }
            Self::RemoteHttpBind { .. } => {
                "pass --allow-remote if this server really must be reachable \
                 from other machines, and put an authenticated gateway in front of it"
                    .to_owned()
            }
            Self::PolicyFile { .. } => {
                "fix the policy file (docs/policy.md has the format and the \
                 rejection rules), then rerun"
                    .to_owned()
            }
            Self::Dashboard { .. } => "check that the --dashboard port is not already held by \
                 another process (or pass --dashboard 0 to let rutter pick \
                 a free one), then retry"
                .to_owned(),
        }
    }
}
