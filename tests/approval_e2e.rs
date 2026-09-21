//! Acceptance for approval flows (grant / deny / timeout) against the
//! real stack — an MCP client drives `rutter serve` with a policy that
//! requires approval for pointer actions, and decisions arrive through
//! the dashboard HTTP API (docs/policy.md).
//!
//! `#[ignore]`d by default: the CI integration job (or a developer)
//! runs them once the engine is in the cache.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};
use tokio::process::Command;

/// Outcome codes the raw HTTP probe cares about.
enum HttpStatus {
    Ok,
    NotFound,
    Other,
}

/// Writes a pointer-approval policy with a short window.
async fn write_policy(dir: &std::path::Path, timeout_ms: u64) -> std::path::PathBuf {
    let path = dir.join("policy.toml");
    let text = format!(
        "approval_timeout_ms = {timeout_ms}\n[[rules]]\naction_class = \"pointer\"\nverdict = \"require_approval\"\n"
    );
    std::fs::write(&path, text).expect("write policy");
    path
}

/// Boots `rutter serve` with a policy and dashboard; the token reaches
/// only the child process through its environment. The returned client
/// owns the child.
async fn connect(
    policy: &std::path::Path,
    dashboard_port: u16,
) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let mut command = Command::new(common::rutter_bin());
    command
        .arg("serve")
        .arg("--policy")
        .arg(policy)
        .arg("--dashboard")
        .arg(dashboard_port.to_string())
        .env("RUTTER_DASHBOARD_TOKEN", "test-token");
    let transport = TokioChildProcess::new(command).expect("spawn rutter serve");
    let client = ().serve(transport).await.expect("client initialization");
    // Give the dashboard listener a moment to bind.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    client
}

/// Calls one tool.
async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    arguments: Value,
) -> CallToolResult {
    let params: CallToolRequestParams = serde_json::from_value(json!({
        "name": name,
        "arguments": arguments
    }))
    .expect("well-formed params");
    client
        .call_tool(params)
        .await
        .expect("tool call succeeds on the protocol level")
}

/// First text block of a result.
fn first_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .find_map(|block| block.as_text().map(|text| text.text.clone()))
        .expect("a text content block")
}

/// Extracts the reference of the Park button from snapshot text.
fn park_reference(snapshot: &str) -> String {
    let marker = "- button \"Park\" [ref=";
    let start = snapshot
        .find(marker)
        .unwrap_or_else(|| panic!("snapshot must contain {marker}: {snapshot}"));
    let tail = &snapshot[start + marker.len()..];
    tail.chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// Posts a decision to the dashboard HTTP API with a raw request.
async fn decide(port: u16, request_id: &str, grant: bool) -> HttpStatus {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("dashboard reachable");
    let body = format!("{{\"request_id\":\"{request_id}\",\"grant\":{grant}}}");
    let request = format!(
        "POST /api/decisions?token=test-token HTTP/1.1\r\nhost: 127.0.0.1:{port}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = String::new();
    stream.read_to_string(&mut response).await.expect("read");
    if response.starts_with("HTTP/1.1 200") {
        HttpStatus::Ok
    } else if response.starts_with("HTTP/1.1 404") {
        HttpStatus::NotFound
    } else {
        HttpStatus::Other
    }
}

/// Navigates to the Park page and returns the button's reference.
async fn park_button(client: &rmcp::service::RunningService<rmcp::RoleClient, ()>) -> String {
    let page = "data:text/html,<button>Park</button>";
    let navigate = call(client, "navigate", json!({ "url": page })).await;
    assert!(!navigate.is_error.unwrap_or(false), "navigate failed");
    park_reference(&first_text(&navigate))
}

/// Waits until the dashboard serves the token (poll on a plain GET).
async fn wait_for_dashboard(port: u16) {
    // Generous on purpose: this wait only gates readiness, never
    // behavior, and an instrumented (coverage) build of the binary
    // cold-starts far slower than a normal debug build.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let probe = tokio::net::TcpStream::connect(("127.0.0.1", port)).await;
        if probe.is_ok() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "dashboard did not bind within 30 s"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Grant: a parked click proceeds once a human grants it.
#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn approval_grant_flow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let policy = write_policy(dir.path(), 30_000).await;
    let client = connect(&policy, 47901).await;
    wait_for_dashboard(47901).await;

    let reference = park_button(&client).await;
    let click = call(&client, "click", json!({ "reference": reference }));
    tokio::pin!(click);

    let result = tokio::select! {
        result = &mut click => result,
        _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => {
            assert!(
                matches!(decide(47901, "apr-1", true).await, HttpStatus::Ok),
                "the parked approval must accept the grant"
            );
            click.await
        }
    };
    assert!(
        !result.is_error.unwrap_or(false),
        "granted click must succeed: {:?}",
        first_text(&result)
    );
}

/// Deny: a parked click fails with an approval denial.
#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn approval_deny_flow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let policy = write_policy(dir.path(), 30_000).await;
    let client = connect(&policy, 47902).await;
    wait_for_dashboard(47902).await;

    let reference = park_button(&client).await;
    let click = call(&client, "click", json!({ "reference": reference }));
    tokio::pin!(click);

    let result = tokio::select! {
        result = &mut click => result,
        _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => {
            assert!(
                matches!(decide(47902, "apr-1", false).await, HttpStatus::Ok),
                "the parked approval must accept the denial"
            );
            click.await
        }
    };
    let text = first_text(&result);
    assert!(
        result.is_error.unwrap_or(false) && text.contains("approval denied"),
        "denied click must fail with the denial: {text}"
    );
}

/// Timeout: nobody answers within the policy window.
#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn approval_timeout_flow() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A 500 ms window: the parked click resolves on its own.
    let policy = write_policy(dir.path(), 500).await;
    let client = connect(&policy, 47903).await;

    let reference = park_button(&client).await;
    let result = call(&client, "click", json!({ "reference": reference })).await;
    let text = first_text(&result);
    assert!(
        result.is_error.unwrap_or(false) && text.contains("no approval decision arrived"),
        "an unanswered approval must time out: {text}"
    );
}
