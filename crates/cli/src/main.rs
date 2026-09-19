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

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use rutter_cli::config::Settings;
use rutter_cli::entry::EntryMode;

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

    /// Policy TOML file for serve mode (blueprint §7.6).
    #[arg(long, global = true, value_name = "FILE")]
    policy: Option<std::path::PathBuf>,

    /// Enable the supervision dashboard on 127.0.0.1:PORT (serve mode).
    #[arg(long, global = true, value_name = "PORT")]
    dashboard: Option<u16>,

    /// Serve MCP over streamable HTTP at ADDR instead of stdio
    /// (serve mode), for example 127.0.0.1:9800.
    #[arg(long, global = true, value_name = "ADDR")]
    http: Option<std::net::SocketAddr>,

    /// Confirm a non-loopback --http bind: the transport drives a real
    /// browser with this user's sessions and has no authentication.
    #[arg(long, global = true)]
    allow_remote: bool,
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

    let policy = cli.policy.map(|path| {
        match std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| {
                rutter_policy::parse_policy(&text)
                    .map_err(|error| error.to_string())
                    .map(|rules| (rules, text))
            }) {
            Ok((rules, _text)) => rules,
            Err(error) => {
                eprintln!("rutter: {path:?}: {error}");
                std::process::exit(2);
            }
        }
    });

    match runtime.block_on(rutter_cli::entry::run(
        mode,
        &settings,
        policy,
        cli.dashboard,
        cli.http,
        cli.allow_remote,
    )) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rutter: {error}");
            eprintln!("rutter: hint — {}", error.hint());
            ExitCode::FAILURE
        }
    }
}
