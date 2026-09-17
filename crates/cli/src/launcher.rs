//! Engine registration point of the binary.
//!
//! Boundary: selects the backend and resolves its executable (blueprint
//! §8.1). The download/cache resolution lives in `rutter-engine`; this
//! module only decides which product a mode needs and wires the CDP
//! launcher. No other module may construct launchers.

use std::path::PathBuf;

use rutter_engine::descriptor::EngineBackend;
use rutter_engine::download::{Product, discover_system_browser, ensure};
use rutter_engine::error::EngineError;
use rutter_engine_cdp::CdpLauncher;

use crate::config::Settings;

/// Headless diagnostic (`open`) and MCP serving use the headless shell.
pub async fn headless_launcher(settings: &Settings) -> Result<CdpLauncher, EngineError> {
    let executable = explicit_or(settings, Product::ChromeHeadlessShell).await?;
    Ok(CdpLauncher::new(
        executable,
        EngineBackend::ChromiumHeadlessShell,
    ))
}

/// Browse mode needs a visible window: an explicit binary wins, then a
/// system-installed browser, then the full Chrome for Testing download.
pub async fn headed_launcher(settings: &Settings) -> Result<CdpLauncher, EngineError> {
    let executable = if let Some(path) = &settings.engine_executable {
        path.clone()
    } else if let Some(path) = discover_system_browser() {
        path
    } else {
        let installed = ensure(Product::Chrome, &settings.cache_root, None).await?;
        installed.executable
    };
    Ok(CdpLauncher::new(executable, EngineBackend::Chromium))
}

/// Resolves the executable for the mode's default product, honoring an
/// explicit override.
async fn explicit_or(settings: &Settings, product: Product) -> Result<PathBuf, EngineError> {
    let installed = ensure(
        product,
        &settings.cache_root,
        settings.engine_executable.as_deref(),
    )
    .await?;
    Ok(installed.executable)
}
