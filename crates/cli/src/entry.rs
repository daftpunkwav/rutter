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
pub async fn run(mode: EntryMode, settings: &Settings) -> Result<(), CliError> {
    match mode {
        EntryMode::Browse => crate::browse::run(settings).await,
        EntryMode::Open { url } => crate::open::run(settings, &url).await,
        EntryMode::Serve { headed: _ } => Err(CliError::Unavailable {
            mode: "serve".to_owned(),
            reason: "the MCP tool surface lands in milestone M1".to_owned(),
        }),
    }
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

    #[tokio::test]
    async fn serve_is_honest_about_m1() {
        let settings = Settings::resolve(
            None,
            Some(std::env::temp_dir().join("rutter-test-serve")),
            Vec::new(),
        )
        .expect("settings");
        let error = run(EntryMode::Serve { headed: false }, &settings)
            .await
            .expect_err("serve must stay unavailable in M0");
        assert!(matches!(error, CliError::Unavailable { .. }));
        assert!(error.to_string().contains("M1"));
    }
}
