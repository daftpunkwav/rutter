//! Keyboard events against a real Chrome for Testing engine: named keys
//! carry codes and text, so a focused input receives the characters.
//!
//! Boundary: the launcher -> engine -> context -> page path through the
//! `rutter-engine` traits only. Needs the downloaded headless shell,
//! so it is `#[ignore]`d by default.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::descriptor::EngineBackend;
use rutter_engine::input::InputEvent;
use rutter_engine::page::PageHandle;
use rutter_engine::supervisor::EngineLauncher;
use rutter_engine_cdp::CdpLauncher;

use common::resolve_executable;

async fn press(page: &dyn PageHandle, key: &str) {
    for event in [
        InputEvent::KeyPressed {
            key: key.to_owned(),
        },
        InputEvent::KeyReleased {
            key: key.to_owned(),
        },
    ] {
        page.dispatch_input(event).await.expect("key dispatches");
    }
}

#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn typed_keys_land_in_a_focused_input() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let context = engine.create_context(ContextConfig::default()).await?;
    let (_page_id, page) = context.open_page().await?;
    page.navigate("data:text/html,<input id=\"i\">").await?;
    page.evaluate("document.getElementById('i').focus()")
        .await?;

    press(page.as_ref(), "h").await;
    press(page.as_ref(), "i").await;
    let value = page.evaluate("document.getElementById('i').value").await?;
    assert_eq!(value, serde_json::json!("hi"));

    // Enter moves nothing here (a lone input), but it must not wedge
    // the page; the input keeps its value.
    press(page.as_ref(), "Enter").await;
    let value = page.evaluate("document.getElementById('i').value").await?;
    assert_eq!(value, serde_json::json!("hi"));

    engine.shutdown().await?;
    Ok(())
}
