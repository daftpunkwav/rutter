//! `rutter open <url>`: one-shot diagnostic.
//!
//! Boundary: navigate once, print one snapshot to stdout, exit. No
//! sessions, no recovery, no policy — those belong to the serve and
//! browse modes. stdout carries only the snapshot; progress and errors
//! go to stderr.

use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::engine::Engine;
use rutter_engine::supervisor::EngineLauncher;

use crate::config::Settings;
use crate::error::CliError;
use crate::launcher;

/// Runs the open mode to completion.
pub async fn run(settings: &Settings, url: &str) -> Result<(), CliError> {
    let launcher = launcher::headless_launcher(settings).await?;
    let engine = launcher.launch(LaunchMode::Headless).await?;

    let result = open_once(engine.as_ref(), settings, url).await;
    let _ = engine.shutdown().await;
    result
}

/// Drives one navigation and snapshot, always shutting the engine down.
async fn open_once(engine: &dyn Engine, settings: &Settings, url: &str) -> Result<(), CliError> {
    let context = engine
        .create_context(ContextConfig {
            navigation_timeout: settings.navigation_timeout,
            ..ContextConfig::default()
        })
        .await?;
    let (_page_id, page) = context.open_page().await?;

    let effective_url = page.navigate(url).await?;
    let raw = page.evaluate(rutter_observe::serializer_script()).await?;
    let snapshot = rutter_observe::snapshot_from_response(&effective_url, &raw);
    println!("{snapshot}");
    Ok(())
}
