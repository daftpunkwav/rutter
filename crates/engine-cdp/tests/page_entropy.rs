//! The ref scope against a real Chrome for Testing engine: the engine
//! gives every document a scope the document cannot choose — at document
//! start for the documents it loads, and from an isolated world for the
//! one that is already there — so two pages cannot be handed the same
//! scope and a stale reference cannot cross between them.
//!
//! Boundary: the launcher -> engine -> context -> page path through the
//! `rutter-engine` traits only. Needs the downloaded headless shell,
//! so it is `#[ignore]`d by default.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use std::sync::Arc;
use std::time::Duration;

use rutter_core::ids::PageId;
use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::context::ContextHandle;
use rutter_engine::descriptor::EngineBackend;
use rutter_engine::page::PageHandle;
use rutter_engine::supervisor::EngineLauncher;
use rutter_engine_cdp::CdpLauncher;
use rutter_observe::serializer_script;

use common::resolve_executable;

/// A page that replaces everything a scope could be drawn from before
/// anything else runs, which is what a hostile document does at the top
/// of its own script: the generator, and the prototypes a conversion to
/// base36 digits would go through.
const PATCH_EVERYTHING: &str = "data:text/html,<body><button>Save</button><script>\
     crypto.getRandomValues = function (bytes) { bytes.fill(0); return bytes; }; \
     Number.prototype.toString = function () { return '000000'; }; \
     String.prototype.charAt = function () { return '0'; }; \
     Uint8Array.prototype[Symbol.iterator] = function* () { yield 0; }; \
     </script></body>";

/// The scope the page's reference store carries after one snapshot.
async fn scope_of(page: &dyn PageHandle) -> String {
    let raw = page
        .evaluate(serializer_script())
        .await
        .expect("serializer runs");
    let text = raw.to_string();
    assert!(
        text.contains("\"truncated\":false"),
        "the store must exist for the scope to be read: {text}"
    );
    let marker = "\"ref\":\"e1-";
    let start = text.find(marker).expect("the button carries a reference") + marker.len();
    let rest = &text[start..];
    let end = rest.find('"').expect("the reference is quoted");
    rest[..end].to_owned()
}

/// Opens a window the engine's document-start registration cannot
/// cover: a popup has no CDP session of its own, so its document runs
/// with no registration at all -- exactly the state of a target that
/// was already loaded when rutter attached. The opener writes the
/// document synchronously (`document.write` runs the script before
/// `open` returns), so the popup is fully prepared by the time it
/// lists, and adoption cannot race the page's own script. The document
/// carries a button, the reference anchor every snapshot below needs.
///
/// The popup reference is kept on the opener (`window.__rutterTestPopup`)
/// so a test can replace the document later: the opener is same-origin
/// with the about:blank popup it opened, and the popup is unreachable
/// through the engine until adopted.
///
/// The expression answers `null` on purpose: `window.open` itself
/// returns a Window remote object no CDP transport will serialize.
async fn open_unregistered_window(page: &dyn PageHandle, script: &str) {
    // JSON's string literal is a valid JavaScript string literal, so
    // the script rides inside the expression whatever quotes it uses.
    let literal = serde_json::to_string(script).expect("the script serializes");
    page.evaluate(&format!(
        "(function () {{ \
           var popup = window.open(); \
           window.__rutterTestPopup = popup; \
           popup.document.write('<body><button>Save</button></body>' + \
             '<script>' + {literal} + '<\\/script>'); \
           popup.document.close(); \
         }})(); null"
    ))
    .await
    .expect("the opener can fill its own popup");
}

#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn a_page_cannot_choose_its_own_ref_scope() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;
    let context = engine.create_context(ContextConfig::default()).await?;

    let (_first_id, first) = context.open_page().await?;
    first.navigate(PATCH_EVERYTHING).await?;
    let first_scope = scope_of(first.as_ref()).await;

    // A second document, patched the same way: with the minter in place
    // the two scopes still differ, which is what keeps a stale reference
    // from one page off the other's elements.
    //
    // Two draws from the same generator can land on one scope, at about
    // one in 36^6 per pair -- past two billion to one. That is the
    // property the design promises -- independent draws -- and the
    // assertion states it; the failure it would report is a real
    // collision, not a broken minter.
    let (_second_id, second) = context.open_page().await?;
    second.navigate(PATCH_EVERYTHING).await?;
    let second_scope = scope_of(second.as_ref()).await;

    assert_ne!(
        first_scope, second_scope,
        "a patched page must not hand two documents one scope"
    );
    assert_ne!(
        first_scope, "000000",
        "neither the generator nor the conversion reached the scope"
    );

    // The minter is what a page cannot undo: the property is locked, and
    // it still draws from the platform.
    let probe = first
        .evaluate(
            "(function () { \
               var descriptor = Object.getOwnPropertyDescriptor(window, '__rutterRefScope'); \
               return { locked: descriptor.writable === false && descriptor.configurable === false, \
                        scope: window.__rutterRefScope() }; \
             })()",
        )
        .await?;
    assert_eq!(
        probe["locked"],
        serde_json::json!(true),
        "the property is locked"
    );
    assert_ne!(probe["scope"], serde_json::json!("000000"));

    engine.shutdown().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn the_document_that_is_already_there_gets_a_scope_too()
-> Result<(), Box<dyn std::error::Error>> {
    // The document-start registration only reaches documents loaded after
    // it, so a page that is already there -- `about:blank` of a page this
    // context just opened, or a target rutter attached to -- is covered by
    // the engine minting its scope in an isolated world. Without that,
    // this snapshot would come back truncated with no references.
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;
    let context = engine.create_context(ContextConfig::default()).await?;

    let (_page_id, page) = context.open_page().await?;
    // The same document, filled in without navigating: a navigation would
    // hand the job to the document-start registration instead.
    page.evaluate("document.body.innerHTML = '<button>Save</button>'")
        .await?;
    let scope = scope_of(page.as_ref()).await;
    assert_ne!(scope, "000000");

    let raw = page.evaluate(serializer_script()).await?;
    assert!(
        raw.to_string().contains(&format!("\"ref\":\"e1-{scope}\"")),
        "the scope stays the one the document was given"
    );

    engine.shutdown().await?;
    Ok(())
}

/// Waits for the popup to list and returns its id. The listing can beat
/// the popup's commit, so the poll is what keeps this deterministic,
/// exactly like in `foreign_pages_are_adoptable_and_closable`.
async fn single_foreign_surface(
    context: &Arc<dyn ContextHandle>,
) -> Result<PageId, Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let foreign = context.foreign_pages().await?;
        if let Some(surface) = foreign.first() {
            assert!(
                foreign.len() == 1,
                "exactly the popup is foreign: {foreign:?}"
            );
            return Ok(surface.id.clone());
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the popup never listed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Polls the foreign listing until a surface's URL satisfies `wanted`,
/// and returns its id. The listing can beat a popup's navigation
/// commit, so the poll is what keeps an adoption from racing it.
async fn poll_foreign_url(
    context: &Arc<dyn ContextHandle>,
    wanted: impl Fn(&str) -> bool,
) -> Result<PageId, Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let foreign = context.foreign_pages().await?;
        if let Some(listed) = foreign.iter().find(|listed| wanted(&listed.url)) {
            return Ok(listed.id.clone());
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no popup ever listed with the wanted URL: {foreign:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A document that replaced its main-world entropy before rutter ever
/// attached: adopting it must mint its scope in the isolated world,
/// which the page's replacement cannot reach. An implementation that
/// minted in the page's main world answers `000000` here, and one that
/// trusted the document's own state hands out no reference at all.
#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn an_already_loaded_page_gets_a_trusted_scope() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;
    let context = engine.create_context(ContextConfig::default()).await?;
    let (_opener_id, opener) = context.open_page().await?;

    open_unregistered_window(
        opener.as_ref(),
        "crypto.getRandomValues = function (bytes) { bytes.fill(0); return bytes; };",
    )
    .await;
    let surface = single_foreign_surface(&context).await?;
    let (_page_id, page) = context.adopt_page(&surface).await?;

    let scope = scope_of(page.as_ref()).await;
    assert_ne!(
        scope, "000000",
        "the replaced entropy must not reach the scope"
    );

    // The registration adopted with the page covers its next document
    // too: the fresh document carries a minter without a new adoption
    // doing anything.
    page.navigate("data:text/html,<body><button>Save</button></body>")
        .await?;
    assert_eq!(
        page.evaluate("typeof window.__rutterRefScope === 'function'")
            .await?,
        serde_json::json!(true),
        "the registration covers the documents loaded after the adoption"
    );

    engine.shutdown().await?;
    Ok(())
}

/// A document that did not stop at replacing entropy but planted its own
/// locked minter cannot be adopted at all: no engine install can take a
/// non-configurable property back, so the honest answer is the error --
/// never a snapshot whose scope the page chose. An implementation that
/// recognizes any installed minter as the engine's would adopt here, and
/// every reference it hands out would resolve wherever the page wants.
/// The failed adoption leaves the target alone: it was never this
/// context's to close.
#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn a_page_that_planted_its_own_scope_is_never_adopted()
-> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;
    let context = engine.create_context(ContextConfig::default()).await?;
    let (_opener_id, opener) = context.open_page().await?;

    open_unregistered_window(
        opener.as_ref(),
        "crypto.getRandomValues = function (bytes) { bytes.fill(0); return bytes; };          Object.defineProperty(window, '__rutterRefScope', {            value: function () { return 'zzzz'; },            writable: false, configurable: false, enumerable: false });",
    )
    .await;
    let surface = single_foreign_surface(&context).await?;
    let adopted = context.adopt_page(&surface).await;
    assert!(
        adopted.is_err(),
        "a page-chosen scope must fail the adoption"
    );

    let still_there = context.foreign_pages().await?;
    assert!(
        still_there.iter().any(|listed| listed.id == surface),
        "the failed adoption must not close the target: {still_there:?}"
    );

    engine.shutdown().await?;
    Ok(())
}

/// An adoption that failed is not a target that can never be adopted:
/// once the page's own document is gone, nothing stands in the way of a
/// fresh mint. The opener replaces the popup's document -- same target,
/// new document, none of the planted property -- and the second adoption
/// prepares it like any other. An implementation whose preparation gate
/// stayed shut after a failure would fail here forever.
#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn an_adoption_that_failed_can_recover() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;
    let context = engine.create_context(ContextConfig::default()).await?;
    let (_opener_id, opener) = context.open_page().await?;

    open_unregistered_window(
        opener.as_ref(),
        "Object.defineProperty(window, '__rutterRefScope', {            value: function () { return 'zzzz'; },            writable: false, configurable: false, enumerable: false });",
    )
    .await;
    // Mark the planted document in its URL, so the listing can tell the
    // two documents apart.
    opener
        .evaluate("window.__rutterTestPopup.location.hash = 'plant'; null")
        .await?;
    let surface = poll_foreign_url(&context, |url| url.ends_with("#plant")).await?;
    let first_attempt = context.adopt_page(&surface).await;
    assert!(
        first_attempt.is_err(),
        "the planted scope must fail the first adoption"
    );

    // A real navigation, not `document.open`: replacing the document
    // that way keeps the same Window global, and the planted property
    // with it. Navigating away re-creates the global, so the new
    // document has nothing planted. The listing reports the committed
    // URL, so the poll is what keeps the re-adoption from racing the
    // navigation.
    opener
        .evaluate("window.__rutterTestPopup.location.href = 'about:blank'; null")
        .await?;
    let surface = poll_foreign_url(&context, |url| url == "about:blank").await?;
    let (_page_id, page) = context.adopt_page(&surface).await?;
    page.evaluate("document.body.innerHTML = '<button>Save</button>'")
        .await?;
    let scope = scope_of(page.as_ref()).await;
    assert_ne!(
        scope, "zzzz",
        "the new document must carry an engine-chosen scope"
    );

    engine.shutdown().await?;
    Ok(())
}
