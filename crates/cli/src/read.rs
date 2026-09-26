//! `rutter read <url>`: one-shot scrape.
//!
//! Boundary: navigate once, print the page's readable content as a
//! markdown document to stdout, exit. No sessions, no recovery, no
//! policy — those belong to the serve and browse modes. stdout carries
//! only the readout; progress and errors go to stderr.

use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::engine::Engine;
use rutter_engine::supervisor::EngineLauncher;

use crate::config::Settings;
use crate::error::CliError;
use crate::launcher;

/// Runs the read mode to completion.
pub async fn run(settings: &Settings, url: &str) -> Result<(), CliError> {
    let launcher = launcher::headless_launcher(settings).await?;
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let result = read_once(engine.as_ref(), settings, url).await;
    let _ = engine.shutdown().await;
    result
}

/// Drives one navigation and readout, always shutting the engine down.
async fn read_once(engine: &dyn Engine, settings: &Settings, url: &str) -> Result<(), CliError> {
    let context = engine
        .create_context(ContextConfig {
            navigation_timeout: settings.navigation_timeout,
            ..ContextConfig::default()
        })
        .await?;
    let (_page_id, page) = context.open_page().await?;

    page.navigate(url).await?;
    let raw = page.evaluate(rutter_observe::reader_script()).await?;
    println!("{}", rutter_observe::read_from_response(&raw));
    Ok(())
}
