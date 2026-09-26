//! Acceptance for `rutter read`: the one-shot scrape navigates once and
//! prints the page's markdown document on stdout.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use tokio::process::Command;

/// Runs `rutter read <url>` to completion, capturing both streams.
async fn run_read(url: &str) -> std::process::Output {
    let mut command = Command::new(common::rutter_bin());
    command
        .arg("read")
        .arg(url)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    command.output().await.expect("read mode runs")
}

#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn read_prints_markdown_and_exits() {
    let output = run_read(
        "data:text/html,<title>read-e2e</title><h1>Heading One</h1>\
         <p>Body with <a href=\"https://example.com/x\">a link</a>.</p>",
    )
    .await;
    assert!(
        output.status.success(),
        "read must exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    assert!(stdout.starts_with("read-e2e\n\n"), "title line: {stdout:?}");
    assert!(stdout.contains("# Heading One"), "heading: {stdout}");
    assert!(
        stdout.contains("[a link](https://example.com/x)"),
        "absolute link: {stdout}"
    );
}
