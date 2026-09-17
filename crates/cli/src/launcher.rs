//! Engine registration point of the binary.
//!
//! Boundary: selects the backend and resolves its executable (blueprint
//! §8.1). The download/cache resolution lives in `rutter-engine`; this
//! module only decides which product a mode needs and wires the CDP
//! launcher. No other module may construct launchers. Serving modes use
//! [`LazyLauncher`] so the process starts before any engine I/O happens
//! (blueprint §8.5: engine lazy, startup < 100 ms).

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use rutter_engine::config::LaunchMode;
use rutter_engine::descriptor::EngineBackend;
use rutter_engine::download::{Product, discover_system_browser, ensure};
use rutter_engine::engine::Engine;
use rutter_engine::error::EngineError;
use rutter_engine::supervisor::EngineLauncher;
use rutter_engine_cdp::CdpLauncher;

use crate::config::Settings;

/// Headless diagnostic (`open`) resolves eagerly: the engine is needed
/// immediately.
pub async fn headless_launcher(settings: &Settings) -> Result<CdpLauncher, EngineError> {
    let executable = explicit_or(settings, Product::ChromeHeadlessShell).await?;
    Ok(
        CdpLauncher::new(executable, EngineBackend::ChromiumHeadlessShell)
            .with_extra_args(settings.extra_engine_args.clone()),
    )
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
    Ok(CdpLauncher::new(executable, EngineBackend::Chromium)
        .with_extra_args(settings.extra_engine_args.clone()))
}

/// A launcher that defers binary resolution to the first launch, so
/// `rutter serve` reaches MCP-ready before any download or disk probe
/// (blueprint §8.5: startup < 100 ms, engine lazy).
pub struct LazyLauncher {
    settings: Settings,
    headed: bool,
}

impl LazyLauncher {
    /// Wraps settings; `headed` selects the full browser (system
    /// install or download) over the headless shell at launch time.
    pub fn new(settings: Settings, headed: bool) -> Self {
        Self { settings, headed }
    }
}

#[async_trait]
impl EngineLauncher for LazyLauncher {
    fn describe(&self) -> String {
        if self.headed {
            "lazy cdp engine (headed: system browser or chrome)".to_owned()
        } else {
            "lazy cdp engine (chrome-headless-shell)".to_owned()
        }
    }

    async fn launch(&self, mode: LaunchMode) -> Result<Arc<dyn Engine>, EngineError> {
        let launcher = if self.headed {
            headed_launcher(&self.settings).await?
        } else {
            headless_launcher(&self.settings).await?
        };
        launcher.launch(mode).await
    }
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
