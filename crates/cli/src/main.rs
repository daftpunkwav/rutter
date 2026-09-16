//! Binary entry: argument parsing, config, and process lifecycle.
//!
//! Responsibilities:
//! - Parse the invocation into one of the three entry modes.
//! - Own process exit codes and top-level error reporting.
//!
//! Boundary: the CLI owns no engine or orchestration logic of its own;
//! it is the first consumer of the core crates. Engine bring-up lands
//! with milestone M0 (blueprint §10).

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod entry;

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use entry::EntryMode;

/// Headless browser orchestration for AI agents.
#[derive(Debug, Parser)]
#[command(name = "rutter", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

/// Available subcommands; no subcommand selects browse mode.
#[derive(Debug, Subcommand)]
enum Command {
    /// Start the MCP server (engine headless unless --headed).
    Serve {
        /// Run the engine with a visible window.
        #[arg(long)]
        headed: bool,
    },
    /// One-shot diagnostic: navigate, print a snapshot, exit.
    Open {
        /// URL to navigate to.
        url: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mode = match cli.command {
        None => EntryMode::Browse,
        Some(Command::Serve { headed }) => EntryMode::Serve { headed },
        Some(Command::Open { url }) => EntryMode::Open { url },
    };

    match entry::run(mode) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rutter: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocations_map_to_entry_modes() {
        let mode = |command: Option<Command>| match command {
            None => EntryMode::Browse,
            Some(Command::Serve { headed }) => EntryMode::Serve { headed },
            Some(Command::Open { url }) => EntryMode::Open { url },
        };
        assert_eq!(mode(None), EntryMode::Browse);
        assert_eq!(
            mode(Some(Command::Serve { headed: true })),
            EntryMode::Serve { headed: true }
        );
        assert_eq!(
            mode(Some(Command::Open {
                url: "https://example.com".to_owned()
            })),
            EntryMode::Open {
                url: "https://example.com".to_owned()
            }
        );
    }
}
