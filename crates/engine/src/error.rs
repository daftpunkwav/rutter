//! The public error contract of the engine layer.
//!
//! Boundary: these variants describe failures of the engine process and
//! its protocol; the session layer maps them onto
//! [`rutter_core::error::ActionError`] for agents.

use std::time::Duration;

use thiserror::Error;

use rutter_core::error::TransportCause;

/// Why an engine-layer operation failed.
#[derive(Debug, Clone, Error)]
pub enum EngineError {
    /// The page could not be loaded: bad URL, DNS, TLS, or a server
    /// error. Distinguished from internal bugs so callers can treat it
    /// as input feedback rather than a defect.
    ///
    /// `cause` is classified by the backend that saw the failure. The
    /// vocabulary of network errors belongs to whoever speaks the
    /// protocol, so a caller above this crate never matches on error
    /// text (docs/architecture.md).
    #[error("navigation to '{url}' failed: {detail}")]
    NavigationFailed {
        /// URL that was requested.
        url: String,
        /// Transport-level classification of the failure.
        cause: TransportCause,
        /// Backend detail, kept for the human-readable message.
        detail: String,
    },

    /// The engine process could not be launched or connected to.
    #[error("engine launch failed: {detail}")]
    LaunchFailed {
        /// What went wrong during launch or connect.
        detail: String,
    },

    /// The engine process died; affected sessions receive
    /// `ActionError::EngineTerminated` after this surfaces.
    #[error("engine terminated")]
    Terminated,

    /// An operation exceeded its deadline; every I/O operation has one.
    #[error("operation '{operation}' timed out after {elapsed:?}")]
    Timeout {
        /// Name of the operation that timed out.
        operation: String,
        /// Time spent before giving up.
        elapsed: Duration,
    },

    /// The backend does not support the requested operation.
    #[error("operation '{operation}' is unsupported: {reason}")]
    Unsupported {
        /// Name of the requested operation.
        operation: String,
        /// Why the backend cannot perform it.
        reason: String,
    },

    /// A bug was contained at the engine boundary; never a silent pass.
    #[error("internal engine error: {detail}")]
    Internal {
        /// What went wrong, for reporting the bug.
        detail: String,
    },

    /// A resource cap was hit; the operation is refused, not queued.
    #[error("capacity exceeded: {detail}")]
    Capacity {
        /// Which cap was hit and what the caller can do about it.
        detail: String,
    },

    /// The engine binary could not be located or downloaded.
    #[error("engine download failed: {detail}")]
    DownloadFailed {
        /// What went wrong while resolving or fetching the binary.
        detail: String,
    },
}

impl EngineError {
    /// Returns an actionable, English hint for the failure; every
    /// surface that reports engine errors shows it next to the message.
    pub fn hint(&self) -> &'static str {
        match self {
            Self::NavigationFailed { .. } => {
                "re-check that the URL is spelled correctly and reachable from \
                 this machine, then retry"
            }
            Self::DownloadFailed { .. } => {
                "set --engine-executable to an existing browser binary, or \
                 --cache-dir to a writable directory and retry"
            }
            Self::LaunchFailed { .. } => {
                "verify the browser binary runs on its own; pass a different \
                 one via --engine-executable if needed"
            }
            Self::Timeout { .. } => {
                "the target page was slow; retry, or check the URL in a normal \
                 browser"
            }
            Self::Capacity { .. } => {
                "close a page or context before opening more; the caps protect \
                 the shared engine process"
            }
            // The remaining variants name process-level conditions an
            // operator investigates from the engine's own output, so
            // they share this generic pointer.
            _ => {
                "inspect the error above; most engine failures are transient and \
                 a retry is safe"
            }
        }
    }
}
