//! Engine lifecycle against a real Chrome for Testing engine:
//! launch, health, driving one page through navigate, evaluate, input,
//! and screenshot, then closing everything down.
//!
//! Boundary: the full launcher -> engine -> context -> page path
//! through the `rutter-engine` traits only. Needs the downloaded
//! headless shell, so it is `#[ignore]`d by default.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use std::time::Duration;

use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::descriptor::EngineBackend;
use rutter_engine::input::InputEvent;
use rutter_engine::supervisor::EngineLauncher;
use rutter_engine_cdp::CdpLauncher;

use common::resolve_executable;

#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn launches_and_drives_a_page() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let descriptor = engine.descriptor();
    assert!(descriptor.capabilities.headless, "shell must be headless");
    let health = engine.health().await?;
    assert!(health.healthy);

    let context = engine
        .create_context(ContextConfig {
            navigation_timeout: Duration::from_secs(20),
            ..ContextConfig::default()
        })
        .await?;
    let (page_id, page) = context.open_page().await?;
    assert!(context.pages().contains(&page_id));

    let url = page
        .navigate("data:text/html,<title>ok</title><button>Go</button>")
        .await?;
    assert!(url.starts_with("data:text/html"), "unexpected url: {url}");

    let title = page.evaluate("document.title").await?;
    assert_eq!(title.as_str(), Some("ok"));

    // A click that lands anywhere on the page must be accepted.
    page.dispatch_input(InputEvent::MousePressed {
        x: 10.0,
        y: 10.0,
        button: rutter_engine::input::MouseButton::Left,
    })
    .await?;
    page.dispatch_input(InputEvent::MouseReleased {
        x: 10.0,
        y: 10.0,
        button: rutter_engine::input::MouseButton::Left,
    })
    .await?;

    let screenshot = page.capture_screenshot().await?;
    assert!(
        screenshot.data.len() > 100,
        "screenshot implausibly small: {} bytes",
        screenshot.data.len()
    );

    context.close_page(page_id.clone()).await?;
    assert!(!context.pages().contains(&page_id));
    engine.shutdown().await?;
    Ok(())
}
