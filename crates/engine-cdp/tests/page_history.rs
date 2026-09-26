//! Page history navigation against a real Chrome for Testing engine:
//! back and forward resolve with the effective URL, an out-of-range
//! walk fails with a message, reload succeeds.
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
async fn history_navigation_and_reload() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let context = engine.create_context(ContextConfig::default()).await?;
    let (_page_id, page) = context.open_page().await?;

    let first = page.navigate("data:text/html,<title>one</title>").await?;
    let second = page.navigate("data:text/html,<title>two</title>").await?;
    assert_ne!(first, second);

    let back = page.go_back().await?;
    assert!(
        back.starts_with("data:text/html,<title>one"),
        "back: {back}"
    );
    let forward = page.go_forward().await?;
    assert!(
        forward.starts_with("data:text/html,<title>two"),
        "forward: {forward}"
    );

    // At the newest entry there is nothing to walk forward to.
    let error = page.go_forward().await.expect_err("no entry ahead");
    assert!(
        error.to_string().contains("no history entry"),
        "unexpected error: {error}"
    );

    page.reload().await.expect("reload succeeds");

    engine.shutdown().await?;
    Ok(())
}
