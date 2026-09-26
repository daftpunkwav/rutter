//! Context page bookkeeping against a real Chrome for Testing engine:
//! the page cap, idempotent context close, closing a target that
//! vanished on its own, and the lookup and cookie edges.
//!
//! Boundary: the full launcher -> engine -> context path through the
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
