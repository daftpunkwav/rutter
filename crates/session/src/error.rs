//! The session error contract: agent-facing action failures plus
//! engine-level failures that reach the caller unchanged.

use rutter_core::error::ActionError;
use rutter_engine::error::EngineError;
use thiserror::Error;

/// Why a session operation failed.
#[derive(Debug, Error)]
pub enum SessionError {
    /// A typed action failure from the docs/tool-catalog.md §3 taxonomy.
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
    /// never take (docs/architecture.md: the dashboard executes none).
    #[error("no open page to observe")]
    NoOpenPage,

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
            SessionError::Internal {
                detail: "probe".to_owned(),
            },
        ];
        for error in &errors {
            assert!(!error.hint().is_empty(), "missing hint for {error}");
        }
    }
}
