//! The ref scope against a real Chrome for Testing engine: the entropy
//! capture the engine installs at document start survives a page that
//! replaces `crypto`, so two documents cannot be handed the same scope.
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

/// A page that replaces the generator before anything else runs, which
/// is what a hostile document does at the top of its own script.
const PATCH_CRYPTO: &str = "data:text/html,<body><button>Save</button><script>\
     crypto.getRandomValues = function (bytes) { bytes.fill(0); return bytes; }; \
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
    first.navigate(PATCH_CRYPTO).await?;
    let first_scope = scope_of(first.as_ref()).await;

    // A second document, patched the same way: with the capture in place
    // the two scopes still differ, which is what keeps a stale reference
    // from one page off the other's elements.
    let (_second_id, second) = context.open_page().await?;
    second.navigate(PATCH_CRYPTO).await?;
    let second_scope = scope_of(second.as_ref()).await;

    assert_ne!(
        first_scope, second_scope,
        "a patched crypto must not hand two documents one scope"
    );
    assert_ne!(
        first_scope, "0000",
        "the constant fill never reached the scope"
    );

    // The capture is what a page cannot undo: the property is locked and
    // still draws from the native generator.
    let probe = first
        .evaluate(
            "(function () { \
               var descriptor = Object.getOwnPropertyDescriptor(window, '__rutterGetRandomValues'); \
               var bytes = new Uint8Array(4); \
               window.__rutterGetRandomValues(bytes); \
               return { locked: descriptor.writable === false && descriptor.configurable === false, \
                        filled: Array.prototype.join.call(bytes, ',') }; \
             })()",
        )
        .await?;
    assert_eq!(
        probe["locked"],
        serde_json::json!(true),
        "the property is locked"
    );
    assert_ne!(
        probe["filled"],
        serde_json::json!("0,0,0,0"),
        "the captured generator is the native one, not the page's"
    );

    engine.shutdown().await?;
    Ok(())
}
