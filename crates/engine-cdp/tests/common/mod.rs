//! Shared fixtures for the real-engine integration binaries: the
//! engine binary resolver every suite starts from.

// Each test target compiles this module and uses a subset of it.
#![allow(dead_code)]

use std::path::PathBuf;

/// Resolves a real engine binary: an explicit override via
/// `RUTTER_TEST_ENGINE`, else the shared downloader cache.
pub async fn resolve_executable() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Ok(path) = std::env::var("RUTTER_TEST_ENGINE") {
        return Ok(PathBuf::from(path));
    }
    let cache = rutter_engine::download::cache_root_default()?;
    let installed = rutter_engine::download::ensure(
        rutter_engine::download::Product::ChromeHeadlessShell,
        &cache,
        None,
    )
    .await?;
    Ok(installed.executable)
}
