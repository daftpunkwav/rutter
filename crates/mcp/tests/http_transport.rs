//! In-process acceptance for the streamable HTTP transport: the real
//! `serve_http` serves a real MCP client over loopback, each connection
//! gets its own session, and the browser/Host guards refuse strangers.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use std::sync::Arc;
use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Value, json};

use rutter_engine::config::LaunchMode;
use rutter_mcp::http::serve_http;
use rutter_policy::{ApprovalBroker, RuleSet};
use rutter_session::config::SessionConfig;
use rutter_session::manager::SessionManager;

use common::FlowLauncher;

/// Grabs a free loopback port by binding and releasing an ephemeral
/// listener.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral listener")
        .local_addr()
        .expect("local address")
        .port()
}

/// Boots the in-process HTTP server and connects one MCP client.
async fn connect() -> (
    rmcp::service::RunningService<rmcp::RoleClient, ()>,
    u16,
    Arc<SessionManager>,
) {
    let port = free_port();
    let manager = Arc::new(SessionManager::new(
        FlowLauncher::new(),
        LaunchMode::Headless,
        SessionConfig::default(),
        Arc::new(RuleSet::default_set()),
        Arc::new(ApprovalBroker::new()),
        None,
    ));
    tokio::spawn({
        let manager = Arc::clone(&manager);
        async move {
            let _ = serve_http(manager, ([127, 0, 0, 1], port).into()).await;
        }
    });
    wait_up(port).await;

    let transport = StreamableHttpClientTransport::from_uri(format!("http://127.0.0.1:{port}/mcp"));
    let client = ().serve(transport).await.expect("client initialization");
    (client, port, manager)
}

/// Polls the port until the server accepts connections.
async fn wait_up(port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "http server did not bind within 10 s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
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

#[tokio::test]
async fn http_transport_round_trip_in_process() {
    let (client, _port, manager) = connect().await;

    let page = "data:text/html,<h1>Http In Process</h1>";
    let result = call(&client, "navigate", json!({ "url": page })).await;
    assert!(
        !result.is_error.unwrap_or(false),
        "navigate failed: {:?}",
        first_text(&result)
    );

    let snapshot = call(&client, "snapshot", json!({})).await;
    assert!(
        first_text(&snapshot).contains("button \"Ok\""),
        "snapshot over http: {}",
        first_text(&snapshot)
    );

    // The connection's session is one of the manager's own, created
    // through the shared factory (one MCP connection is one session).
    let sessions = manager.session_ids().await;
    assert_eq!(sessions.len(), 1, "one connection mints one session");
    assert!(
        sessions
            .first()
            .expect("one session")
            .as_str()
            .starts_with("http-"),
        "HTTP connections get http-prefixed session ids: {sessions:?}"
    );

    // Closing through the tool surface ends that session on the manager.
    let closed = call(&client, "close_session", json!({})).await;
    assert!(
        !closed.is_error.unwrap_or(false),
        "close_session failed: {:?}",
        first_text(&closed)
    );
    assert!(
        manager.session_ids().await.is_empty(),
        "the closed session leaves the manager"
    );
}

#[tokio::test]
async fn http_rejects_browser_origin() {
    let (_client, port, _manager) = connect().await;

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
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
        port
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

#[tokio::test]
async fn http_rejects_foreign_host() {
    let (_client, port, _manager) = connect().await;

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
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
