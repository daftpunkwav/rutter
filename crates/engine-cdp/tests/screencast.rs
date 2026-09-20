//! Screencast acceptance: the live view contract of docs/dashboard.md —
//! JPEG frames with a correct ack loop, on-demand start/stop, and
//! restart across navigations.
//!
//! `#[ignore]`d by default: requires the engine binary in the cache.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::path::PathBuf;
use std::time::Duration;

use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::descriptor::EngineBackend;
use rutter_engine::download::{Product, cache_root_default, ensure};
use rutter_engine::engine::Engine;
use std::sync::Arc;

use rutter_engine::page::{PageHandle, ScreencastStream};
use rutter_engine::supervisor::EngineLauncher;
use rutter_engine_cdp::CdpLauncher;

/// Resolves a real engine binary via the shared cache.
async fn resolve_executable() -> PathBuf {
    if let Ok(path) = std::env::var("RUTTER_TEST_ENGINE") {
        return PathBuf::from(path);
    }
    let cache = cache_root_default().expect("cache root");
    let installed = ensure(Product::ChromeHeadlessShell, &cache, None)
        .await
        .expect("engine download");
    installed.executable
}

/// The live engine, kept alive by the test for the whole flow.
type LiveEngine = Arc<dyn Engine>;

/// Launches an engine with one page showing a static header. The engine
/// must stay alive for the duration: dropping it kills the browser.
async fn open_page() -> (LiveEngine, ScreencastStream, Arc<dyn PageHandle>) {
    let executable = resolve_executable().await;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await.expect("launch");
    let context = engine
        .create_context(ContextConfig {
            navigation_timeout: Duration::from_secs(20),
            ..ContextConfig::default()
        })
        .await
        .expect("context");
    let (_page_id, page) = context.open_page().await.expect("page");
    page.navigate("data:text/html,<h1>Live</h1>")
        .await
        .expect("navigate");

    let stream = page.start_screencast().await.expect("screencast");
    (engine, stream, page)
}

/// First frame must be a real JPEG (magic FF D8 FF) within a deadline.
#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn screencast_delivers_jpeg_frames() {
    let (_engine, mut stream, _page) = open_page().await;

    let frame = tokio::time::timeout(Duration::from_secs(10), stream.next_frame())
        .await
        .expect("a frame within 10 s")
        .expect("stream open");
    assert!(
        frame.jpeg.len() > 100,
        "frame implausibly small: {} bytes",
        frame.jpeg.len()
    );
    assert_eq!(&frame.jpeg[..3], &[0xFF, 0xD8, 0xFF], "frames must be JPEG");
}

/// Dropping the stream stops the capture: a fresh page handle can
/// start another screencast afterwards (on-demand lifecycle).
#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn screencast_stops_when_dropped() {
    let (_engine, stream, page) = open_page().await;
    drop(stream);
    // Give the stop command a beat to land.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // A second start must succeed; a stuck capture would starve it.
    let mut second = tokio::time::timeout(Duration::from_secs(10), page.start_screencast())
        .await
        .expect("restart within 10 s")
        .expect("second screencast");
    let frame = tokio::time::timeout(Duration::from_secs(10), second.next_frame())
        .await
        .expect("a frame within 10 s")
        .expect("stream open");
    assert_eq!(&frame.jpeg[..2], &[0xFF, 0xD8]);
}

/// The capture keeps flowing across navigations (docs/dashboard.md: CDP
/// stops screencasts on navigation and the backend restarts them).
#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn screencast_survives_navigation() {
    let (_engine, mut stream, page) = open_page().await;

    let first = tokio::time::timeout(Duration::from_secs(10), stream.next_frame())
        .await
        .expect("first frame")
        .expect("stream open");
    assert_eq!(&first.jpeg[..2], &[0xFF, 0xD8]);

    page.navigate("data:text/html,<h1>Moved</h1>")
        .await
        .expect("navigate");

    let second = tokio::time::timeout(Duration::from_secs(10), stream.next_frame())
        .await
        .expect("frame after navigation")
        .expect("stream open after navigation");
    assert_eq!(&second.jpeg[..2], &[0xFF, 0xD8]);
}
