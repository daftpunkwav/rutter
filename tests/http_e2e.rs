//! M3 acceptance: the streamable HTTP transport (blueprint §3, §10).
//! An MCP client connects to `rutter serve --http` over HTTP, runs a
//! navigate + snapshot round-trip, and the DNS-rebinding Host check
//! rejects foreign hosts.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Value, json};
use tokio::process::Command;

/// Boots `rutter serve --http` and returns the connected client.
async fn connect(port: u16) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let mut command = Command::new(common::rutter_bin());
    command
        .arg("serve")
        .arg("--http")
        .arg(format!("127.0.0.1:{port}"))
        // Null stdio: an inherited stdout pipe would keep the test
        // harness waiting for EOF after the assertions.
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = command.spawn().expect("spawn rutter serve");
    // Keep the child running for the whole test; the assertions need
    // the server, and it is intentionally not waited on.
    std::mem::forget(child);

    wait_for_http(port).await;

    let transport = StreamableHttpClientTransport::from_uri(format!("http://127.0.0.1:{port}/mcp"));
    ().serve(transport).await.expect("client initialization")
}

/// Polls the /mcp endpoint until the server accepts requests.
async fn wait_for_http(port: u16) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let reachable = tokio::net::TcpStream::connect(("127.0.0.1", port)).await;
        if reachable.is_ok() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "http server did not bind within 5 s"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
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

/// Full round-trip over streamable HTTP: initialize, navigate, read a
/// snapshot.
#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn http_transport_round_trip() {
    let client = connect(49911).await;

    let page = "data:text/html,<h1>Http E2E</h1>";
    let result = call(&client, "navigate", json!({ "url": page })).await;
    assert!(
        !result.is_error.unwrap_or(false),
        "navigate failed: {:?}",
        first_text(&result)
    );

    let snapshot = call(&client, "snapshot", json!({})).await;
    assert!(
        first_text(&snapshot).contains("heading \"Http E2E\""),
        "snapshot over http: {}",
        first_text(&snapshot)
    );
}

/// The DNS-rebinding check: a foreign Host header is rejected before
/// any MCP processing (blueprint §7.7-grade checks on the MCP surface).
#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn http_rejects_foreign_host() {
    let _client = connect(49912).await;

    let mut stream = tokio::net::TcpStream::connect("127.0.0.1:49912")
        .await
        .expect("server reachable");
    let request = "POST /mcp HTTP/1.1\r\n\
                   host: attacker.example\r\n\
                   content-type: application/json\r\n\
                   accept: application/json, text/event-stream\r\n\
                   content-length: 2\r\n\
                   connection: close\r\n\r\n{}";
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = String::new();
    stream.read_to_string(&mut response).await.expect("read");
    assert!(
        response.starts_with("HTTP/1.1 403"),
        "foreign host must be rejected: {response}"
    );
}
