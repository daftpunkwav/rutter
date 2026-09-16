//! Entry modes of the rutter binary.
//!
//! The three modes (browse, serve, open) only set defaults; engine mode
//! and the dashboard are orthogonal flags on top of them. Boundary: mode
//! dispatch hands an entry mode to the component that implements it.
//! Until engine bring-up lands (milestone M0), every mode fails with a
//! clear message instead of pretending to work.

use std::fmt;

use thiserror::Error;

/// What the binary was asked to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryMode {
    /// Headed engine window operated by a human, dashboard enabled; the
    /// no-argument default.
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

impl fmt::Display for EntryMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Browse => f.write_str("browse"),
            Self::Serve { .. } => f.write_str("serve"),
            Self::Open { .. } => f.write_str("open"),
        }
    }
}

/// Why an entry mode could not run.
#[derive(Debug, Error)]
#[error("the '{mode}' mode requires the engine backend, which is not wired into this build yet")]
pub struct EntryError {
    /// The mode that was requested.
    mode: EntryMode,
}

/// Runs an entry mode to completion.
pub fn run(mode: EntryMode) -> Result<(), EntryError> {
    Err(EntryError { mode })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_display_by_name() {
        assert_eq!(EntryMode::Browse.to_string(), "browse");
        assert_eq!(EntryMode::Serve { headed: false }.to_string(), "serve");
        assert_eq!(
            EntryMode::Open {
                url: "https://example.com".to_owned()
            }
            .to_string(),
            "open"
        );
    }

    #[test]
    fn run_reports_missing_engine_backend() {
        let error = run(EntryMode::Browse).expect_err("skeleton must not fake success");
        assert!(error.to_string().contains("engine backend"));
    }
}
