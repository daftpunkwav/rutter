//! File upload against a real Chrome for Testing engine: a snapshot
//! reference resolves to the file input, `set_input_files` hands real
//! files to it, and a stale reference fails with `ReferenceExpired`.
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

/// Runs the serializer so the page's reference store exists, then
/// returns the reference of the page's only actionable element.
async fn only_reference(page: &dyn PageHandle) -> String {
    let raw = page
        .evaluate(serializer_script())
        .await
        .expect("serializer runs");
    let text = raw.to_string();
    let marker = "\"ref\":\"";
    let start = text
        .find(marker)
        .expect("the single input carries a reference")
        + marker.len();
    let rest = &text[start..];
    let end = rest.find('"').expect("reference is quoted");
    rest[..end].to_owned()
}

#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn files_reach_the_input_and_stale_references_fail() -> Result<(), Box<dyn std::error::Error>>
{
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let context = engine.create_context(ContextConfig::default()).await?;
    let (_page_id, page) = context.open_page().await?;
    page.navigate("data:text/html,<input type=\"file\" multiple>")
        .await?;

    // No snapshot yet: the reference store does not exist, so the
    // upload fails with the shared expired-reference error.
    let file = std::env::temp_dir().join("rutter-upload-integration.txt");
    std::fs::write(&file, b"payload")?;
    let stale = page
        .set_input_files("e1", &[file.to_string_lossy().into_owned()])
        .await;
    assert!(
        matches!(
            stale,
            Err(rutter_engine::error::EngineError::ReferenceExpired { .. })
        ),
        "a reference without a snapshot must fail: {stale:?}"
    );

    // After a snapshot the reference resolves and the input receives
    // the file.
    let reference = only_reference(page.as_ref()).await;
    page.set_input_files(&reference, &[file.to_string_lossy().into_owned()])
        .await?;
    let seen = page
        .evaluate(
            "(function () { var input = document.querySelector('input'); \
              return { count: input.files.length, name: input.files[0].name }; })()",
        )
        .await?;
    assert_eq!(seen["count"], serde_json::json!(1));
    assert_eq!(
        seen["name"],
        serde_json::json!("rutter-upload-integration.txt")
    );

    let _ = std::fs::remove_file(&file);
    engine.shutdown().await?;
    Ok(())
}
