//! Page observations against a real Chrome for Testing engine: the
//! dialog feed reports an `alert` and answering it unblocks the page;
//! console calls and uncaught exceptions arrive on the same feed.
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
use rutter_engine::page::{ObservationStream, PageObservation};
use rutter_engine::supervisor::EngineLauncher;
use rutter_engine_cdp::CdpLauncher;

use common::resolve_executable;

/// The feed's task runs on its own schedule; observations get a generous
/// budget to arrive.
async fn next_observation(stream: &mut ObservationStream) -> PageObservation {
    tokio::time::timeout(Duration::from_secs(10), stream.next_observation())
        .await
        .expect("observation arrives within the budget")
        .expect("the feed stays open")
}

#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn dialogs_console_and_exceptions_flow_through_the_feed()
-> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let context = engine.create_context(ContextConfig::default()).await?;
    let (_page_id, page) = context.open_page().await?;
    page.navigate("data:text/html,<title>feed</title>").await?;

    let mut stream = page.observe().await?;

    // A console call and an uncaught exception, both delivered.
    page.evaluate("console.warn('careful', 41)").await?;
    match next_observation(&mut stream).await {
        PageObservation::ConsoleEmitted { level, text } => {
            assert_eq!(level.as_str(), "warning");
            assert_eq!(text, "careful 41");
        }
        other => panic!("expected a console observation, got {other:?}"),
    }
    page.evaluate("setTimeout(() => { throw new Error('boom') }, 0)")
        .await?;
    match next_observation(&mut stream).await {
        PageObservation::UncaughtException { text } => {
            assert!(text.contains("boom"), "unexpected text: {text}");
        }
        other => panic!("expected an exception observation, got {other:?}"),
    }

    // An alert wedges the page until the dialog is answered; the feed
    // reports it and the answer unblocks evaluation.
    page.evaluate("setTimeout(() => alert('blocked'), 0)")
        .await?;
    match next_observation(&mut stream).await {
        PageObservation::DialogOpened { kind, message } => {
            assert_eq!(kind.as_str(), "alert");
            assert_eq!(message, "blocked");
        }
        other => panic!("expected a dialog observation, got {other:?}"),
    }
    page.handle_dialog(false, None).await?;
    let unblocked = page.evaluate("1 + 1").await?;
    assert_eq!(unblocked, serde_json::json!(2));

    // The feed is single-consumer: a second observe fails instead of
    // silently splitting the observations.
    let second = page.observe().await;
    assert!(second.is_err(), "the feed must not be handed out twice");

    engine.shutdown().await?;
    Ok(())
}
