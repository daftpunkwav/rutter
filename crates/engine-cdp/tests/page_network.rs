//! Network visibility against a real Chrome for Testing engine: a
//! `fetch` that succeeds reports its status, a request to an
//! unresolvable host reports its failure.
//!
//! Boundary: the launcher -> engine -> context -> page path through the
//! `rutter-engine` traits only. Needs the downloaded headless shell,
//! so it is `#[ignore]`d by default.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use std::time::Duration;

use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::descriptor::EngineBackend;
use rutter_engine::page::PageObservation;
use rutter_engine::supervisor::EngineLauncher;
use rutter_engine_cdp::CdpLauncher;

use common::resolve_executable;

/// Waits for the first network observation that satisfies `want`.
async fn wait_request(
    stream: &mut rutter_engine::page::ObservationStream,
    want: impl Fn(&rutter_engine::page::RequestEntry) -> bool,
) -> rutter_engine::page::RequestEntry {
    let budget = Duration::from_secs(15);
    let start = std::time::Instant::now();
    loop {
        assert!(
            start.elapsed() < budget,
            "no matching network observation arrived"
        );
        let observation = tokio::time::timeout(Duration::from_secs(5), stream.next_observation())
            .await
            .ok();
        match observation {
            Some(Some(PageObservation::RequestObserved { entry })) if want(&entry) => {
                return entry;
            }
            Some(Some(_)) => continue,
            Some(None) | None => continue,
        }
    }
}

#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn fetched_and_failed_requests_are_visible() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let context = engine.create_context(ContextConfig::default()).await?;
    let (_page_id, page) = context.open_page().await?;
    page.navigate("data:text/html,<title>net</title>").await?;
    let mut stream = page.observe().await?;

    // A fetch of a data URL succeeds: the request reports a status.
    page.evaluate("fetch('data:text/plain,x').catch(() => {})")
        .await?;
    let ok = wait_request(&mut stream, |entry| entry.status.is_some()).await;
    assert_eq!(ok.status, Some(200));
    assert_eq!(ok.method, "GET");

    // A request to an unresolvable host reports its failure.
    page.evaluate("fetch('https://rutter-invalid.invalid/x').catch(() => {})")
        .await?;
    let failed = wait_request(&mut stream, |entry| entry.error.is_some()).await;
    assert!(failed.status.is_none());
    assert!(failed.url.contains("rutter-invalid.invalid"));

    engine.shutdown().await?;
    Ok(())
}
