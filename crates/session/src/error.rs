//! The session error contract: agent-facing action failures plus
//! engine-level failures that reach the caller unchanged.

use rutter_core::error::ActionError;
use rutter_engine::error::EngineError;
use thiserror::Error;

/// Why a session operation failed.
#[derive(Debug, Error)]
pub enum SessionError {
    /// A typed action failure from the blueprint §7.3 taxonomy.
    #[error("{0}")]
    Action(#[from] ActionError),

    /// An engine-layer failure (navigation, capacity, engine death).
    #[error("{0}")]
    Engine(#[from] EngineError),
}

impl SessionError {
    /// Returns an actionable, English hint for the failure.
    pub fn hint(&self) -> String {
        match self {
            Self::Action(action) => action.hint(),
            Self::Engine(engine) => engine.hint().to_owned(),
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
        ];
        for error in &errors {
            assert!(!error.hint().is_empty(), "missing hint for {error}");
        }
    }
}
