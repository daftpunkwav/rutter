//! Markdown readouts against a real Chrome for Testing engine: the
//! reader script extracts headings, links, lists, tables, and code as
//! markdown, and omits site chrome and hidden content.
//!
//! Boundary: the launcher -> engine -> context -> page path through the
//! `rutter-engine` traits only, converting with `rutter-observe`. Needs
//! the downloaded headless shell, so it is `#[ignore]`d by default.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod common;

use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::descriptor::EngineBackend;
use rutter_engine::supervisor::EngineLauncher;
use rutter_engine_cdp::CdpLauncher;

use common::resolve_executable;

const PAGE: &str = "data:text/html,<title>Doc</title>\
     <nav>chrome</nav>\
     <h1>Heading One</h1>\
     <p>Intro with <a href=\"https://example.com/x\">a link</a>.</p>\
     <ul><li>one</li><li>two</li></ul>\
     <table><tr><th>a</th><th>b</th></tr><tr><td>1</td><td>2</td></tr></table>\
     <pre>code()</pre>\
     <div style=\"display:none\">hidden</div>";

#[tokio::test]
#[ignore = "requires a downloaded engine binary"]
async fn read_extracts_markdown_and_omits_chrome() -> Result<(), Box<dyn std::error::Error>> {
    let executable = resolve_executable().await?;
    let launcher = CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell);
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let context = engine.create_context(ContextConfig::default()).await?;
    let (_page_id, page) = context.open_page().await?;
    page.navigate(PAGE).await?;

    let raw = page.evaluate(rutter_observe::reader_script()).await?;
    let readout = rutter_observe::read_from_response(&raw);

    assert_eq!(readout.title, "Doc");
    assert!(readout.markdown.contains("# Heading One"), "{readout:?}");
    assert!(
        readout.markdown.contains("[a link](https://example.com/x)"),
        "absolute link: {readout:?}"
    );
    assert!(readout.markdown.contains("- one"), "list: {readout:?}");
    assert!(readout.markdown.contains("- two"), "list: {readout:?}");
    assert!(
        readout.markdown.contains("| a | b |") && readout.markdown.contains("| 1 | 2 |"),
        "table: {readout:?}"
    );
    assert!(readout.markdown.contains("code()"), "pre: {readout:?}");
    assert!(
        !readout.markdown.contains("chrome"),
        "site chrome is omitted: {readout:?}"
    );
    assert!(
        !readout.markdown.contains("hidden"),
        "hidden content is omitted: {readout:?}"
    );
    assert!(!readout.truncated, "{readout:?}");

    engine.shutdown().await?;
    Ok(())
}
