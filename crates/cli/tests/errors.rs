//! Tests for `CliError` presentation: every variant carries an
//! actionable hint and a readable message.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use rutter_cli::error::CliError;
use rutter_engine::error::EngineError;

#[test]
fn every_error_carries_a_hint() {
    let errors = [
        CliError::Engine {
            source: EngineError::Terminated,
        },
        CliError::SignalHandling {
            mode: "browse".to_owned(),
            reason: "signal handling failed".to_owned(),
        },
        CliError::StdioTransport {
            message: "stdio closed".to_owned(),
        },
        CliError::HttpTransport {
            message: "cannot bind 127.0.0.1:9800".to_owned(),
        },
        CliError::RemoteHttpBind {
            addr: "192.168.1.10:9800".parse().expect("a socket address"),
        },
    ];
    for error in &errors {
        assert!(!error.hint().is_empty(), "missing hint for {error}");
        assert!(!error.to_string().is_empty(), "empty display for {error:?}");
    }
}

#[test]
fn signal_handling_names_the_mode() {
    let error = CliError::SignalHandling {
        mode: "serve".to_owned(),
        reason: "signal handling failed".to_owned(),
    };
    let message = error.to_string();
    assert!(
        message.contains("serve") && message.contains("interrupt"),
        "the message names the interrupted mode: {message}"
    );
}

#[test]
fn engine_errors_keep_their_identity() {
    let error = CliError::from(EngineError::Terminated);
    let hint = error.hint();
    // The engine's own hint flows through unchanged (presentation only).
    assert_eq!(hint, EngineError::Terminated.hint());
}
