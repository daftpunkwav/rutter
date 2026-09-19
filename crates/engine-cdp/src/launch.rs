//! Launcher producing CDP engines from a resolved browser executable.
//!
//! Boundary: the CDP registration point. A launcher pairs an executable
//! path with its backend identity (headless shell or full Chrome) and
//! produces engine instances; the binary decides which launcher to
//! build. Process supervision is the supervisor's concern, not this
//! factory's.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chromiumoxide::{Browser, BrowserConfig};
use futures::StreamExt;

use rutter_engine::config::LaunchMode;
use rutter_engine::descriptor::EngineBackend;
use rutter_engine::engine::Engine;
use rutter_engine::error::EngineError;
use rutter_engine::supervisor::EngineLauncher;

use crate::engine::CdpEngine;
use crate::error;

/// Builds CDP engines from one executable.
pub struct CdpLauncher {
    executable: PathBuf,
    backend: EngineBackend,
    extra_args: Vec<String>,
}

impl CdpLauncher {
    /// Creates a launcher for an executable and declares which backend
    /// it provides.
    pub fn new(executable: PathBuf, backend: EngineBackend) -> Self {
        Self {
            executable,
            backend,
            extra_args: Vec::new(),
        }
    }

    /// Passes arguments verbatim to the browser process, for example
    /// `--no-sandbox` in root containers or a proxy flag.
    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }
}

#[async_trait]
impl EngineLauncher for CdpLauncher {
    fn describe(&self) -> String {
        format!("cdp engine at {}", self.executable.display())
    }

    async fn launch(&self, mode: LaunchMode) -> Result<Arc<dyn Engine>, EngineError> {
        let mut builder = BrowserConfig::builder().chrome_executable(&self.executable);
        for argument in &self.extra_args {
            builder = builder.arg(argument.clone());
        }
        if mode == LaunchMode::Headed {
            builder = builder.with_head();
        }
        let config = builder
            .build()
            .map_err(|detail| EngineError::LaunchFailed { detail })?;

        let (browser, mut handler) = Browser::launch(config).await.map_err(error::fold)?;
        // The handler stream must be drained for any CDP traffic to flow;
        // it ends when the connection closes.
        tokio::spawn(async move { while handler.next().await.is_some() {} });

        // The version query carries a deadline like every CDP call: it
        // runs inside the supervisor's restart loop, so a browser that
        // accepts the socket but never answers would otherwise park the
        // heartbeat (and its restart mutex) forever, disabling all
        // recovery. On timeout the dropped `Browser` kills the child, so
        // the policy's next attempt starts clean.
        let version =
            error::with_deadline("engine_version", error::COMMAND_TIMEOUT, browser.version())
                .await?;
        Ok(Arc::new(CdpEngine::new(
            Arc::new(tokio::sync::Mutex::new(browser)),
            self.backend.clone(),
            version.product,
            mode,
        )))
    }
}
