//! Acceptance for the streamable HTTP transport (docs/architecture.md, docs/tool-catalog.md).
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

/// A running `rutter serve --http` child, its port, and the connected
/// client. Dropping this tears the server's whole process tree down,
/// so no serve (or browser grandchild) can outlive the test that
/// started it: a leaked child would keep the cargo-test pipes open
/// (`cargo test | grep` never sees EOF) and a later run could meet a
/// stale server.
struct Server {
    client: rmcp::service::RunningService<rmcp::RoleClient, ()>,
    port: u16,
    /// The serve process id, for the Windows-only tree teardown in
    /// `Drop`; elsewhere kill-on-drop is the only guard and the field
    /// is never read.
    #[cfg_attr(not(windows), expect(dead_code))]
    pid: Option<u32>,
    /// Ownership guard: kill-on-drop is the fallback that never leaves
    /// the serve child itself behind.
    _child: tokio::process::Child,
}

impl Drop for Server {
    fn drop(&mut self) {
        // On Windows, spawning a child copies *every* inheritable
        // handle into it (bInheritHandles), not just the standard
        // streams — so the browser grandchildren of `serve` end up
        // holding the test harness's pipe handles. Killing the serve
        // child alone (kill-on-drop) orphans them, and an orphaned
        // chrome-headless-shell keeps `cargo test | grep` hanging
        // forever. `taskkill /T` tears down the whole tree instead.
        #[cfg(windows)]
        if let Some(pid) = self.pid {
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID"])
                .arg(pid.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        // Off Windows children do not inherit unrelated handles (fds
        // are close-on-exec), so the kill-on-drop guard suffices.
    }
}

/// Grabs a free loopback port by binding and releasing an ephemeral
/// listener. The OS hands out a port that is free right now, so
/// parallel tests and consecutive runs never collide on a fixed port.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral listener")
        .local_addr()
        .expect("local address")
        .port()
}

/// Boots `rutter serve --http` on a fresh port and returns the server
/// plus the connected client.
async fn connect() -> Server {
    let port = free_port();
    let mut command = Command::new(common::rutter_bin());
    command
        .arg("serve")
        .arg("--http")
        .arg(format!("127.0.0.1:{port}"))
        // Null stdio: an inherited stdout pipe would keep the test
        // harness waiting for EOF after the assertions.
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let child = command.spawn().expect("spawn rutter serve");
    let pid = child.id();

    wait_for_http(port).await;

    let transport = StreamableHttpClientTransport::from_uri(format!("http://127.0.0.1:{port}/mcp"));
    let client = ().serve(transport).await.expect("client initialization");
    Server {
        client,
        port,
        pid,
        _child: child,
    }
}

/// Polls the /mcp endpoint until the server accepts requests.
async fn wait_for_http(port: u16) {
    // Generous on purpose: readiness only, never behavior — an
    // instrumented (coverage) build of the binary cold-starts far
    // slower than a normal debug build.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let reachable = tokio::net::TcpStream::connect(("127.0.0.1", port)).await;
        if reachable.is_ok() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "http server did not bind within 30 s"
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
    let server = connect().await;

    let page = "data:text/html,<h1>Http E2E</h1>";
    let result = call(&server.client, "navigate", json!({ "url": page })).await;
    assert!(
        !result.is_error.unwrap_or(false),
        "navigate failed: {:?}",
        first_text(&result)
    );

    let snapshot = call(&server.client, "snapshot", json!({})).await;
    assert!(
        first_text(&snapshot).contains("heading \"Http E2E\""),
        "snapshot over http: {}",
        first_text(&snapshot)
    );
}

/// The DNS-rebinding check: a foreign Host header is rejected before
/// any MCP processing (dashboard-grade checks; docs/dashboard.md).
#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn http_rejects_foreign_host() {
    let server = connect().await;

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", server.port))
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

/// Browser-origin requests are rejected: browsers attach an Origin
/// header and MCP clients never do, so a hostile web page driving the
/// local browser over the transport is refused regardless of CORS.
#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn http_rejects_browser_origin() {
    let server = connect().await;

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", server.port))
        .await
        .expect("server reachable");
    let request = format!(
        "POST /mcp HTTP/1.1\r\n\
         host: 127.0.0.1:{}\r\n\
         origin: http://attacker.example\r\n\
         content-type: application/json\r\n\
         accept: application/json, text/event-stream\r\n\
         content-length: 2\r\n\
         connection: close\r\n\r\n{{}}",
        server.port
    );
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = String::new();
    stream.read_to_string(&mut response).await.expect("read");
    assert!(
        response.starts_with("HTTP/1.1 403"),
        "browser-origin requests must be rejected: {response}"
    );
}
