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

use rutter_engine::config::{ContextConfig, LaunchMode};
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
     Number.prototype.toString = function () { return '0000'; }; \
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
    // one in 36^4 per pair. That is the property the design promises --
    // independent draws -- and the assertion states it; the failure it
    // would report is a real collision, not a broken minter.
    let (_second_id, second) = context.open_page().await?;
    second.navigate(PATCH_EVERYTHING).await?;
    let second_scope = scope_of(second.as_ref()).await;

    assert_ne!(
        first_scope, second_scope,
        "a patched page must not hand two documents one scope"
    );
    assert_ne!(
        first_scope, "0000",
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
    assert_ne!(probe["scope"], serde_json::json!("0000"));

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
    assert_ne!(scope, "0000");

    let raw = page.evaluate(serializer_script()).await?;
    assert!(
        raw.to_string().contains(&format!("\"ref\":\"e1-{scope}\"")),
        "the scope stays the one the document was given"
    );

    engine.shutdown().await?;
    Ok(())
}
