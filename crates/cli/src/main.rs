//! Binary entry: argument parsing, config, and process lifecycle.
//!
//! Responsibilities:
//! - Parse the invocation into one of the three entry modes.
//! - Own process exit codes and top-level error reporting.
//!
//! Boundary: the CLI owns no engine or orchestration logic of its own;
//! it is the first consumer of the core crates. The tokio runtime here
//! is process init: one runtime for the whole invocation.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod browse;
mod config;
mod entry;
mod error;
mod launcher;
mod open;
mod serve;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use config::Settings;
use entry::EntryMode;

/// Headless browser orchestration for AI agents.
#[derive(Debug, Parser)]
#[command(name = "rutter", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Use this browser binary instead of downloading or caching one.
    #[arg(long, global = true, value_name = "PATH")]
    engine_executable: Option<std::path::PathBuf>,

    /// Engine cache directory (default: OS cache dir + rutter).
    #[arg(long, global = true, value_name = "DIR", env = "RUTTER_CACHE_DIR")]
    cache_dir: Option<std::path::PathBuf>,

    /// Extra argument passed to the engine process (repeatable).
    #[arg(long = "engine-arg", global = true, value_name = "ARG")]
    engine_args: Vec<String>,
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

    let settings = match Settings::resolve(cli.engine_executable, cli.cache_dir, cli.engine_args) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("rutter: {error}");
            eprintln!("rutter: hint - set RUTTER_CACHE_DIR to a writable path");
            return ExitCode::FAILURE;
        }
    };

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("rutter: cannot start the async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(entry::run(mode, &settings)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rutter: {error}");
            eprintln!("rutter: hint — {}", error.hint());
            ExitCode::FAILURE
        }
    }
}
