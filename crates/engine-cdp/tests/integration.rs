//! End-to-end tests against a real Chrome for Testing engine.
//!
//! Boundary: these tests exercise the full launcher -> engine -> context
//! -> page path through the `rutter-engine` traits only. They download
//! the headless shell on first run (network + ~150 MB) and are therefore
//! `#[ignore]`d by default; the CI integration job runs them explicitly.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

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

/// Closing an already-closed context succeeds: the browser answers a
/// repeated dispose with "not found", and the handle folds that into
/// success so a session teardown can never wedge on a dead context
/// (docs/sessions.md recovery races).
#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn context_close_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let context = engine.create_context(ContextConfig::default()).await?;
    context.close().await.expect("first close");
    context
        .close()
        .await
        .expect("second close must also succeed");

    engine.shutdown().await?;
    Ok(())
}

/// A page the browser dropped on its own answers CloseTarget with an
/// error; closing it through the context must still succeed and drop
/// the registration, or the page cap would count ghost pages. The
/// vanish is simulated by closing the page twice: the first close
/// removes target and registration, the second proves a vanished page
/// never errors and never wedges the cap.
#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn close_page_accepts_an_already_vanished_target() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let context = engine.create_context(ContextConfig::default()).await?;
    let (page_id, _page) = context.open_page().await?;
    context.close_page(page_id.clone()).await?;
    context
        .close_page(page_id.clone())
        .await
        .expect("closing a vanished page twice stays a success");
    assert!(!context.pages().contains(&page_id));

    engine.shutdown().await?;
    Ok(())
}

/// History walks and reload: back/forward resolve with the effective
/// URL, an out-of-range walk fails with a message, reload succeeds.
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

/// The context's bookkeeping surface: id, per-page lookup, closing an
/// unknown page (a no-op), and the empty cookie fast path.
#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn context_exposes_ids_pages_and_cookie_edges() -> Result<(), Box<dyn std::error::Error>> {
    use rutter_core::cookie::{Cookie, SameSite};
    use rutter_core::ids::PageId;

    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let context = engine.create_context(ContextConfig::default()).await?;
    assert!(!context.id().as_str().is_empty());

    let (page_id, page) = context.open_page().await?;
    assert!(
        context.page(page_id.clone()).is_some(),
        "the page lookup finds what open_page registered"
    );
    assert!(
        context.page(PageId::new("ctx:page-999")).is_none(),
        "an unknown page is not found"
    );

    // Closing a page the map never held is a success, not an error.
    context
        .close_page(PageId::new("ctx:page-999"))
        .await
        .expect("closing an unknown page is a no-op");

    // An empty cookie batch is a fast path: no browser round-trip.
    context
        .set_cookies(&[])
        .await
        .expect("empty set is a no-op");

    // A cookie round-trips with its policy fields intact.
    let cookie = Cookie {
        name: "session".to_owned(),
        value: "42".to_owned(),
        domain: "example.com".to_owned(),
        path: Some("/".to_owned()),
        secure: false,
        http_only: true,
        same_site: Some(SameSite::Lax),
        expires: None,
    };
    context.set_cookies(std::slice::from_ref(&cookie)).await?;
    let stored = context.cookies().await?;
    let found = stored
        .iter()
        .find(|found| found.name == "session")
        .expect("the cookie is stored on the context");
    assert_eq!(found.value, "42");
    assert_eq!(found.domain, "example.com");
    assert!(found.http_only);
    assert_eq!(
        found.same_site,
        Some(SameSite::Lax),
        "the SameSite policy maps there and back"
    );

    drop(page);
    engine.shutdown().await?;
    Ok(())
}

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
    // it closes the target: the next listing reports nothing.
    context.close_page(adopted_id.clone()).await?;
    assert!(!context.pages().contains(&adopted_id));
    let relisted = context.foreign_pages().await?;
    assert!(
        relisted.is_empty(),
        "the closed popup is gone: {relisted:?}"
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
