//! Error surface of the CLI with agent- and human-readable hints.
//!
//! Boundary: presentation-level error folding. Engine errors keep their
//! identities and hints (defined with the error itself in
//! `rutter-engine`); the CLI adds only the mode-unavailable case.

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

    /// The requested mode is not available in this build yet.
    #[error("{mode} is not available yet: {reason}")]
    Unavailable {
        /// The entry mode that was requested.
        mode: String,
        /// What is missing and when it is expected.
        reason: String,
    },
}

impl CliError {
    /// Returns an actionable, English hint for the failure.
    pub fn hint(&self) -> String {
        match self {
            Self::Engine { source } => source.hint().to_owned(),
            Self::Unavailable { .. } => "track milestone M1 for the MCP server surface".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_carries_a_hint() {
        let errors = [
            CliError::Engine {
                source: EngineError::Terminated,
            },
            CliError::Unavailable {
                mode: "serve".to_owned(),
                reason: "partially shipped: stdio serves in M1".to_owned(),
            },
        ];
        for error in &errors {
            assert!(!error.hint().is_empty(), "missing hint for {error}");
        }
    }
}
