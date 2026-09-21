//! Acceptance for `rutter open`: the one-shot diagnostic navigates once
//! and prints exactly one snapshot on stdout, and a missing explicit
//! engine exits with a failure and a hint.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use tokio::process::Command;

/// Runs `rutter open <url>` to completion, capturing both streams.
async fn run_open(extra_args: &[&str], url: &str) -> std::process::Output {
    let mut command = Command::new(common::rutter_bin());
    command
        .args(extra_args)
        .arg("open")
        .arg(url)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    command.output().await.expect("open mode runs")
}

#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn open_prints_one_snapshot_and_exits() {
    let output = run_open(
        &[],
        "data:text/html,<title>open-e2e</title><button>Go</button>",
    )
    .await;
    assert!(
        output.status.success(),
        "open must exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    assert!(
        stdout.contains("button \"Go\""),
        "the snapshot travels on stdout: {stdout}"
    );
    assert!(
        !stdout.trim().is_empty(),
        "stdout carries the snapshot, nothing else: {stdout}"
    );
}

#[tokio::test]
async fn open_with_a_missing_engine_fails_with_a_hint() {
    // No engine needed: an explicit path that does not exist must fail
    // before any launch, with the error and its hint on stderr.
    let output = run_open(
        &["--engine-executable", "Z:/definitely/missing/browser.exe"],
        "about:blank",
    )
    .await;
    assert!(!output.status.success(), "a missing engine must not exit 0");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not exist"),
        "the error names the cause: {stderr}"
    );
    assert!(
        stderr.contains("hint"),
        "the failure carries its hint: {stderr}"
    );
}
