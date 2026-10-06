//! Discovery of windows rutter did not open, against a real Chrome for
//! Testing engine: `foreign_pages` reports the surfaces `open_page`
//! never created, `adopt_page` brings them under the context, and the
//! visibility rules keep another session's isolation out of the
//! listing.
//!
//! Boundary: the engine -> context path through the `rutter-engine`
//! traits only. Needs the downloaded headless shell, so it is
//! `#[ignore]`d by default.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use std::time::Duration;

use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::descriptor::EngineBackend;
use rutter_engine::supervisor::EngineLauncher;
use rutter_engine_cdp::CdpLauncher;

use common::resolve_executable;

/// A window a page opened (`window.open` — the `target=_blank` case)
/// exists in the session's own context with nothing tracking it. It
/// must still be drivable: `foreign_pages` reports it, `adopt_page`
/// brings it under the context, and because it lives in the session's
/// own isolation, closing it closes the window for real.
#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn foreign_pages_are_adoptable_and_closable() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let context = engine.create_context(ContextConfig::default()).await?;
    let (_page_id, page) = context.open_page().await?;
    page.navigate("data:text/html,<title>opener</title>")
        .await?;
    // Popup blocking is off in the automation defaults, so a script can
    // open a window without a user gesture.
    // The expression answers `null` on purpose: `window.open` itself
    // returns a Window remote object no CDP transport will serialize.
    page.evaluate("window.open('data:text/html,<title>popup</title>'); null")
        .await?;

    // The popup lists as foreign — a real window in this context that
    // `open_page` did not create — while the tracked opener does not.
    let foreign = context.foreign_pages().await?;
    assert_eq!(
        foreign.len(),
        1,
        "exactly the popup is foreign: {foreign:?}"
    );
    let surface = &foreign[0];
    // The listing can beat the popup's navigation commit, so the URL
    // may still be blank; the target id prefix is what identifies it.
    assert!(surface.id.as_str().starts_with("target:"));

    let (adopted_id, adopted) = context.adopt_page(&surface.id).await?;
    assert_eq!(adopted_id, surface.id);
    assert!(context.pages().contains(&adopted_id));
    // Re-adopting is idempotent: the same id comes back and the
    // context still tracks exactly one page for it.
    let (again_id, _) = context.adopt_page(&surface.id).await?;
    assert_eq!(again_id, adopted_id);
    assert_eq!(
        context
            .pages()
            .iter()
            .filter(|id| **id == adopted_id)
            .count(),
        1
    );

    // The adopted window is drivable like any page.
    adopted
        .navigate("data:text/html,<title>adopted</title>")
        .await?;
    assert_eq!(
        adopted.evaluate("document.title").await?.as_str(),
        Some("adopted")
    );

    // This popup sits inside the session's own isolation, so closing
    // it closes the target. Target destruction races the protocol
    // round-trip, so poll the listing instead of assuming the first
    // query already observes the teardown.
    context.close_page(adopted_id.clone()).await?;
    assert!(!context.pages().contains(&adopted_id));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let relisted = loop {
        let listing = context.foreign_pages().await?;
        if listing.is_empty() || tokio::time::Instant::now() >= deadline {
            break listing;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(
        relisted.is_empty(),
        "the closed popup is gone: {relisted:?}"
    );

    engine.shutdown().await?;
    Ok(())
}

/// Two adoptions of one foreign surface may be in flight at once. Both
/// must come back with it, and the target must survive them: preparing a
/// page defines a locked scope minter, so a second attempt meets the
/// first and throws — and a failure that closed the shared target would
/// take the other caller's page with it.
#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn concurrent_adoptions_of_one_surface_both_succeed() -> Result<(), Box<dyn std::error::Error>>
{
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let context = engine.create_context(ContextConfig::default()).await?;
    let (_page_id, page) = context.open_page().await?;
    page.navigate("data:text/html,<title>opener</title>")
        .await?;
    page.evaluate("window.open('data:text/html,<title>popup</title>'); null")
        .await?;
    let surface = context
        .foreign_pages()
        .await?
        .first()
        .expect("the popup is listed")
        .id
        .clone();

    // Both calls pass the "already tracked" check before either has
    // registered anything, which is what makes them prepare the same
    // target.
    let (first, second) = tokio::join!(context.adopt_page(&surface), context.adopt_page(&surface));
    let (first_id, first_handle) = first?;
    let (second_id, second_handle) = second?;
    assert_eq!(first_id, surface);
    assert_eq!(second_id, surface);

    assert_eq!(
        context.pages().iter().filter(|id| **id == surface).count(),
        1,
        "one surface, one registration"
    );

    // The target still answers through both handles: neither adoption
    // closed what the other drives.
    first_handle
        .navigate("data:text/html,<title>adopted</title>")
        .await?;
    assert_eq!(
        second_handle.evaluate("document.title").await?.as_str(),
        Some("adopted")
    );

    engine.shutdown().await?;
    Ok(())
}

/// Another session's isolated context is nobody's discovery: targets
/// inside a foreign browser context are never reported.
#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn other_contexts_targets_are_never_foreign() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let mine = engine.create_context(ContextConfig::default()).await?;
    let theirs = engine.create_context(ContextConfig::default()).await?;
    let (_their_page, their_handle) = theirs.open_page().await?;
    their_handle
        .navigate("data:text/html,<title>secret</title>")
        .await?;

    let foreign = mine.foreign_pages().await?;
    assert!(
        foreign
            .iter()
            .all(|surface| !surface.url.contains("secret")),
        "another session's tab must not be reported: {foreign:?}"
    );

    engine.shutdown().await?;
    Ok(())
}
