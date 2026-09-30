//! The session error contract: agent-facing action failures,
//! engine-level failures that reach the caller unchanged, and the
//! orchestration layer's own answers (session capacity, observation
//! with no open page, contained bugs).

use rutter_core::error::ActionError;
use rutter_engine::error::EngineError;
use thiserror::Error;

/// Why a session operation failed.
#[derive(Debug, Error)]
pub enum SessionError {
    /// A typed action failure from the shared action taxonomy.
    #[error("{0}")]
    Action(#[from] ActionError),

    /// An engine-layer failure (navigation, engine death, page caps).
    #[error("{0}")]
    Engine(#[from] EngineError),

    /// The server already holds its maximum number of sessions. Unlike
    /// the engine's page and context caps, this cap is the manager's
    /// own: it bounds concurrent MCP clients, not engine targets.
    #[error("capacity exceeded: {detail}")]
    Capacity {
        /// Which cap was hit and what the caller can do about it.
        detail: String,
    },

    /// The session has no open page to observe. Distinct from an engine
    /// failure: nothing is wrong with the browser, there is simply no tab
    /// yet — and opening one would be an action, which an observation must
    /// never take (the dashboard executes none).
    #[error("no open page to observe")]
    NoOpenPage,

    /// The requested session id cannot be used by this server: it would
    /// not compose into an on-disk name for the session's state. Distinct
    /// from engine failures: nothing is wrong with the browser, the id
    /// itself is the problem.
    #[error("invalid session id: {detail}")]
    InvalidId {
        /// What is wrong with the id.
        detail: String,
    },

    /// The session's storage state could not be written to disk.
    /// Distinct from engine failures: the browser is fine, the state
    /// directory is the problem. The in-memory state is kept either way,
    /// so recovery still replays the newest capture; only the file is
    /// behind.
    #[error("storage state write failed: {detail}")]
    StorageWrite {
        /// What went wrong with the write.
        detail: String,
    },

    /// The session's storage state could not be read back. Today the
    /// one trigger is a session opened without a state directory: there
    /// is nothing to load, and that is configuration, not a bug — so it
    /// must not wear the `Internal` "report this" label.
    #[error("storage state read failed: {detail}")]
    StorageRead {
        /// What went wrong with the read.
        detail: String,
    },

    /// A bug was contained at the session boundary; never a silent pass.
    #[error("internal session error: {detail}")]
    Internal {
        /// What went wrong, for reporting the bug.
        detail: String,
    },
}

impl SessionError {
    /// Returns an actionable, English hint for the failure.
    pub fn hint(&self) -> String {
        match self {
            Self::Action(action) => action.hint(),
            Self::Engine(engine) => engine.hint().to_owned(),
            Self::Capacity { .. } => "close one of this server's sessions before opening \
                 another; the cap bounds concurrent clients, not pages"
                .to_owned(),
            Self::NoOpenPage => {
                "navigate to a URL or select a page first; observation shows what already exists"
                    .to_owned()
            }
            Self::InvalidId { .. } => "pick a session id of letters, digits, dashes, and \
                 underscores; the id names the session's on-disk state"
                .to_owned(),
            Self::StorageWrite { .. } => "check that the directory holding the session's \
                 storage file is writable, then save again; the in-memory \
                 state is kept, so nothing was lost"
                .to_owned(),
            Self::StorageRead { .. } => "this session was opened without a storage state \
                 directory, so there is nothing to load; ask for a session \
                 with persistence or keep working from the live pages"
                .to_owned(),
            Self::Internal { detail } => {
                format!("an internal bug was contained; report it, citing: {detail}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_carries_a_hint() {
        let errors = [
            SessionError::Action(ActionError::Internal {
                detail: "probe".to_owned(),
            }),
            SessionError::Engine(EngineError::Terminated),
            SessionError::Capacity {
                detail: "probe".to_owned(),
            },
            SessionError::NoOpenPage,
            SessionError::InvalidId {
                detail: "probe".to_owned(),
            },
            SessionError::StorageWrite {
                detail: "probe".to_owned(),
            },
            SessionError::StorageRead {
                detail: "probe".to_owned(),
            },
            SessionError::Internal {
                detail: "probe".to_owned(),
            },
        ];
        for error in &errors {
            assert!(!error.hint().is_empty(), "missing hint for {error}");
        }
    }
}
