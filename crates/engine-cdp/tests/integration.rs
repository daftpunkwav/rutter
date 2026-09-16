//! End-to-end tests against a real Chrome for Testing engine.
//!
//! Boundary: these tests exercise the full launcher -> engine -> context
//! -> page path through the `rutter-engine` traits only. They download
//! the headless shell on first run (network + ~150 MB) and are therefore
//! `#[ignore]`d by default; the CI integration job runs them explicitly.

use std::path::PathBuf;
use std::time::Duration;

use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::descriptor::EngineBackend;
use rutter_engine::download::{Product, cache_root_default, ensure};
use rutter_engine::input::InputEvent;
use rutter_engine::supervisor::EngineLauncher;
use rutter_engine_cdp::CdpLauncher;

/// Resolves a real engine binary: an explicit override via
/// `RUTTER_TEST_ENGINE`, else the shared downloader cache.
async fn resolve_executable() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Ok(path) = std::env::var("RUTTER_TEST_ENGINE") {
        return Ok(PathBuf::from(path));
    }
    let cache = cache_root_default()?;
    let installed = ensure(Product::ChromeHeadlessShell, &cache, None).await?;
    Ok(installed.executable)
}

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

#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn page_cap_is_enforced() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let context = engine
        .create_context(ContextConfig {
            max_pages: 1,
            ..ContextConfig::default()
        })
        .await?;
    let _first = context.open_page().await?;
    let second = context.open_page().await;
    assert!(
        matches!(
            second,
            Err(rutter_engine::error::EngineError::Capacity { .. })
        ),
        "second open must hit the cap"
    );
    engine.shutdown().await?;
    Ok(())
}
