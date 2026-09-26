//! Viewport override against a real Chrome for Testing engine: the
//! page reports the overridden `innerWidth`/`innerHeight` afterwards.
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
use rutter_engine::supervisor::EngineLauncher;
use rutter_engine_cdp::CdpLauncher;

use common::resolve_executable;

#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn the_page_reports_the_overridden_viewport() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let context = engine.create_context(ContextConfig::default()).await?;
    let (_page_id, page) = context.open_page().await?;
    page.navigate("data:text/html,<title>viewport</title>")
        .await?;

    page.set_viewport(640, 480).await?;
    let inner = page
        .evaluate("({ w: window.innerWidth, h: window.innerHeight })")
        .await?;
    assert_eq!(inner["w"], serde_json::json!(640));
    assert_eq!(inner["h"], serde_json::json!(480));

    engine.shutdown().await?;
    Ok(())
}
