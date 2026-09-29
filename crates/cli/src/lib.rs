//! The `rutter` CLI: argument resolution, entry modes, and process
//! lifecycle for the whole workspace.
//!
//! Responsibilities:
//! - Resolve flags and the environment into runtime [`config::Settings`].
//! - Dispatch the entry modes ([`entry`]): browse, serve, open, read.
//! - Own user-facing error presentation ([`error::CliError`] hints).
//!
//! Boundary: the CLI owns no engine or orchestration logic of its own;
//! it is the composition root that wires the other crates together and
//! the first consumer of their public APIs.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

// Only the pieces the thin binary and the tests/ targets consume are
// public; the mode implementations and the launcher wiring stay
// crate-internal (the composition root's assembly details).
pub mod config;
pub mod entry;
pub mod error;

mod browse;
mod launcher;
mod one_shot;
mod open;
mod read;
mod serve;

/// Renders an observation for the one-shot modes: the Display text plus
/// the truncation marker (the same
/// convention as the MCP surface), so a clipped one-shot output is
/// never silently trusted as complete.
pub(crate) fn render_marked(value: impl std::fmt::Display, truncated: bool) -> String {
    let mut text = value.to_string();
    if truncated {
        text.push_str("… truncated\n");
    }
    text
}

/// Loads and parses the `--policy` file. Public so the binary and the
/// tests share one path, and the error is an [`error::CliError`] like
/// every other startup failure: a hint line and the shared exit code,
/// not a special-cased exit status.
pub fn policy_file(path: &std::path::Path) -> Result<rutter_policy::RuleSet, error::CliError> {
    let text = std::fs::read_to_string(path).map_err(|reason| error::CliError::PolicyFile {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    })?;
    rutter_policy::parse_policy(&text).map_err(|error| error::CliError::PolicyFile {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::render_marked;

    #[test]
    fn a_truncated_rendering_carries_the_marker() {
        // The marker line follows the
        // content, the same convention the MCP surface applies.
        assert_eq!(render_marked("body", false), "body");
        assert_eq!(render_marked("body", true), "body… truncated\n");
    }
}
