//! The public error contract of the engine layer.
//!
//! Boundary: these variants describe failures of the engine process and
//! its protocol; the session layer maps them onto
//! [`rutter_core::ActionError`] for agents.

use std::time::Duration;

use thiserror::Error;

/// Why an engine-layer operation failed.
#[derive(Debug, Clone, Error)]
pub enum EngineError {
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
