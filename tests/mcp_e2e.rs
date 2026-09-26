//! Acceptance: e2e task flows against the real engine
//! (docs/tool-catalog.md §6). Each test drives the built `rutter serve`
//! binary over stdio with an rmcp client, so the whole protocol stack
//! runs: client -> stdio -> MCP -> session -> CDP -> engine.
//!
//! `#[ignore]`d by default: the CI e2e job (or a developer)
//! runs them once the engine is in the cache.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};
use tokio::process::Command;

/// Boots `rutter serve` as a child and connects an rmcp client.
async fn connect() -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let mut command = Command::new(common::rutter_bin());
    command.arg("serve");
    let transport = TokioChildProcess::new(command).expect("spawn rutter serve");
    ().serve(transport).await.expect("client initialization")
}

/// Calls one tool and returns its result.
async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    arguments: Value,
) -> CallToolResult {
    // The params struct is non-exhaustive, so it is built through its
    // deserializer instead of a literal.
    let params: CallToolRequestParams = serde_json::from_value(json!({
        "name": name,
        "arguments": arguments
    }))
    .expect("well-formed tool params");
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

/// Extracts the reference of an element from snapshot text by role and
/// optional accessible name.
fn reference_for(snapshot: &str, role: &str, name: Option<&str>) -> String {
    let marker = match name {
        Some(name) => format!("- {role} \"{name}\" [ref="),
        None => format!("- {role} [ref="),
    };
    let start = snapshot
        .find(&marker)
        .unwrap_or_else(|| panic!("snapshot must contain {marker}: {snapshot}"));
    let tail = &snapshot[start + marker.len()..];
    tail[..tail.find(']').expect("closing bracket")].to_owned()
}

/// Task class 1 (read): navigate, then read the page through a snapshot.
#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn read_task_navigate_and_snapshot() {
    let client = connect().await;

    let page = "data:text/html,<h1>Rutter E2E</h1><button>Go</button>";
    let result = call(&client, "navigate", json!({ "url": page })).await;
    assert!(
        !result.is_error.unwrap_or(false),
        "navigate failed: {:?}",
        first_text(&result)
    );

    let text = first_text(&result);
    assert!(text.contains("heading \"Rutter E2E\""), "snapshot: {text}");
    assert!(text.contains("- button \"Go\" [ref="), "snapshot: {text}");

    let fresh = call(&client, "snapshot", json!({})).await;
    assert!(first_text(&fresh).contains("Rutter E2E"));

    client.cancel().await.expect("shutdown");
}

/// Task class 1b (read): navigate, then read the page as markdown.
#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn read_task_read_returns_markdown() {
    let client = connect().await;

    let page = concat!(
        "data:text/html,<title>Doc</title><h1>Heading One</h1>",
        "<p>Intro with <a href=\"https://example.com/x\">a link</a>.</p>",
        "<nav>chrome</nav>"
    );
    let result = call(&client, "navigate", json!({ "url": page })).await;
    assert!(
        !result.is_error.unwrap_or(false),
        "navigate failed: {:?}",
        first_text(&result)
    );

    let read = call(&client, "read", json!({})).await;
    assert!(
        !read.is_error.unwrap_or(false),
        "read failed: {:?}",
        first_text(&read)
    );

    let text = first_text(&read);
    assert!(text.starts_with("Doc\n\n"), "title line first: {text:?}");
    assert!(text.contains("# Heading One"), "heading: {text}");
    assert!(
        text.contains("[a link](https://example.com/x)"),
        "absolute link: {text}"
    );
    assert!(!text.contains("chrome"), "site chrome is omitted: {text}");

    client.cancel().await.expect("shutdown");
}

/// Task class 2 (interact): click a button that mutates the page and
/// verify the returned snapshot reflects the change.
#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn interact_task_click_changes_the_page() {
    let client = connect().await;

    let page = concat!(
        "data:text/html,<h1 id=\"h\">before</h1>",
        "<button onclick=\"document.getElementById('h').textContent='after'\">Flip</button>"
    );
    let result = call(&client, "navigate", json!({ "url": page })).await;
    assert!(!result.is_error.unwrap_or(false), "navigate failed");

    let reference = reference_for(&first_text(&result), "button", Some("Flip"));
    let clicked = call(&client, "click", json!({ "reference": reference })).await;
    assert!(
        !clicked.is_error.unwrap_or(false),
        "click failed: {:?}",
        first_text(&clicked)
    );

    let text = first_text(&clicked);
    assert!(
        text.contains("heading \"after\""),
        "the click must be visible in the fresh snapshot: {text}"
    );

    client.cancel().await.expect("shutdown");
}

/// Task class 3 (form): type and select into a form, verify echoes, and
/// capture a screenshot.
#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn form_task_type_select_and_screenshot() {
    let client = connect().await;

    let page = concat!(
        "data:text/html,",
        "<input id=\"i\" placeholder=\"Name\" ",
        "oninput=\"document.getElementById('e').textContent=this.value\">",
        "<select id=\"s\" onchange=\"document.getElementById('s-out').textContent=this.value\">",
        "<option value=\"\"></option><option value=\"b\">Bee</option></select>",
        "<h2 id=\"e\"></h2><h2 id=\"s-out\"></h2>"
    );
    let result = call(&client, "navigate", json!({ "url": page })).await;
    assert!(!result.is_error.unwrap_or(false), "navigate failed");
    let snapshot = first_text(&result);

    let input_ref = reference_for(&snapshot, "textbox", Some("Name"));
    let typed = call(
        &client,
        "type",
        json!({ "reference": input_ref, "text": "Ada" }),
    )
    .await;
    assert!(
        !typed.is_error.unwrap_or(false),
        "type failed: {:?}",
        first_text(&typed)
    );
    assert!(
        first_text(&typed).contains("\"Ada\""),
        "the input echo must appear: {}",
        first_text(&typed)
    );

    let fresh = first_text(&call(&client, "snapshot", json!({})).await);
    let select_ref = reference_for(&fresh, "combobox", None);
    let selected = call(
        &client,
        "select_option",
        json!({ "reference": select_ref, "values": ["b"] }),
    )
    .await;
    assert!(
        !selected.is_error.unwrap_or(false),
        "select_option failed: {:?}",
        first_text(&selected)
    );

    let shot = call(&client, "screenshot", json!({})).await;
    assert!(!shot.is_error.unwrap_or(false), "screenshot failed");
    let image = shot
        .content
        .iter()
        .find_map(|block| match block {
            ContentBlock::Image(image) => Some(image),
            _ => None,
        })
        .expect("an image content block");
    assert_eq!(image.mime_type, "image/png");
    assert!(
        image.data.len() > 100,
        "screenshot implausibly small: {} chars",
        image.data.len()
    );

    client.cancel().await.expect("shutdown");
}

/// wait_for resolves once the awaited text appears, and a timeout on a
/// missing text surfaces as an isError result naming the budget.
#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn wait_for_resolves_and_times_out() {
    let client = connect().await;

    // A heading, so the text lands in the accessibility snapshot.
    let page = "data:text/html,<h1>alpha</h1>";
    let navigate = call(&client, "navigate", json!({ "url": page })).await;
    assert!(!navigate.is_error.unwrap_or(false), "navigate failed");

    // A text that is already there resolves immediately.
    let found = call(&client, "wait_for", json!({ "text": "alpha" })).await;
    assert!(
        !found.is_error.unwrap_or(false),
        "existing text must resolve: {:?}",
        first_text(&found)
    );
    assert!(first_text(&found).contains("alpha"));

    // A text that never appears times out with the requested budget.
    let missing = call(
        &client,
        "wait_for",
        json!({ "text": "never-there", "timeout_ms": 500 }),
    )
    .await;
    let text = first_text(&missing);
    assert!(
        missing.is_error.unwrap_or(false) && text.contains("timed out"),
        "a missing text must time out: {text}"
    );

    client.cancel().await.expect("shutdown");
}

/// Close is terminal for the connection: a live session closes
/// cleanly, and every later tool call on the same connection fails at
/// the protocol level naming the closed session instead of silently
/// reopening it (docs/tool-catalog.md §4).
#[tokio::test]
#[ignore = "requires the engine binary in the cache"]
async fn close_session_fails_fast_after_closing() {
    let client = connect().await;

    let page = "data:text/html,<h1>Close me</h1>";
    let navigate = call(&client, "navigate", json!({ "url": page })).await;
    assert!(!navigate.is_error.unwrap_or(false), "navigate failed");

    // List pages first: navigate must have seeded exactly one tracked
    // page and tabs_list must show it.
    let listed = call(&client, "tabs_list", json!({})).await;
    let text = first_text(&listed);
    assert!(text.contains("(active)"), "the page is listed: {text}");

    let closed = call(&client, "close_session", json!({})).await;
    assert!(
        !closed.is_error.unwrap_or(false),
        "closing a live session succeeds: {:?}",
        first_text(&closed)
    );

    // Every later tool call fails fast instead of resurrecting the
    // closed session (docs/tool-catalog.md: close is terminal for the connection).
    // The server rejects the call at the protocol level (invalid_params),
    // so the raw call result is matched against the expected message.
    let params: CallToolRequestParams = serde_json::from_value(json!({
        "name": "snapshot",
        "arguments": {}
    }))
    .expect("well-formed tool params");
    let error = client
        .call_tool(params)
        .await
        .expect_err("a call after close_session must fail on the protocol level");
    let message = error.to_string();
    assert!(
        message.contains("is closed") && message.contains("reconnect"),
        "the rejection must name the closed session: {message}"
    );

    client.cancel().await.expect("shutdown");
}
