//! CDP implementation of the engine trait.
//!
//! Boundary: engine-level lifecycle over one chromiumoxide `Browser`.
//! CDP details (browser contexts, targets) are confined here and in the
//! sibling modules; callers see only `rutter-engine` types.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use chromiumoxide::cdp::browser_protocol::browser::BrowserContextId;
use chromiumoxide::cdp::browser_protocol::target::CreateBrowserContextParams;
use tokio::sync::Mutex;

use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::context::ContextHandle;
use rutter_engine::descriptor::{EngineBackend, EngineCapabilities, EngineDescriptor};
use rutter_engine::engine::Engine;
use rutter_engine::error::EngineError;
use rutter_engine::health::HealthReport;

use crate::context::CdpContext;

/// Shared browser connection; `Browser` is not `Clone`, so all handles
/// funnel through this mutex.
pub type SharedBrowser = Arc<Mutex<chromiumoxide::Browser>>;

/// One launched CDP engine process.
pub struct CdpEngine {
    browser: SharedBrowser,
    descriptor: EngineDescriptor,
    context_counter: AtomicU64,
}

impl CdpEngine {
    /// Wraps a launched browser connection as an engine.
    pub fn new(
        browser: SharedBrowser,
        backend: EngineBackend,
        version: String,
        mode: LaunchMode,
    ) -> Self {
        let descriptor = EngineDescriptor {
            backend,
            version,
            capabilities: EngineCapabilities {
                headless: true,
                headed: mode == LaunchMode::Headed,
                screencast: true,
                per_context_isolation: true,
            },
        };
        Self {
            browser,
            descriptor,
            context_counter: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl Engine for CdpEngine {
    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor.clone()
    }

    async fn create_context(
        &self,
        config: ContextConfig,
    ) -> Result<Arc<dyn ContextHandle>, EngineError> {
        let cdp_context_id: BrowserContextId = {
            let browser = self.browser.lock().await;
            crate::error::with_deadline(
                "create_context",
                crate::error::COMMAND_TIMEOUT,
                browser.create_browser_context(CreateBrowserContextParams::default()),
            )
            .await?
        };

        let serial = self.context_counter.fetch_add(1, Ordering::Relaxed);
        let id = rutter_core::ids::ContextId::new(format!("ctx-{}", serial + 1));
        Ok(Arc::new(CdpContext::new(
            id,
            Arc::clone(&self.browser),
            cdp_context_id,
            config,
        )))
    }

    async fn health(&self) -> Result<HealthReport, EngineError> {
        let version = {
            let browser = self.browser.lock().await;
            crate::error::with_deadline("health", crate::error::COMMAND_TIMEOUT, browser.version())
                .await?
        };
        Ok(HealthReport {
            healthy: true,
            backend_version: Some(version.product),
            detail: None,
        })
    }

    async fn shutdown(&self) -> Result<(), EngineError> {
        let mut browser = self.browser.lock().await;
        crate::error::with_deadline("shutdown", crate::error::COMMAND_TIMEOUT, browser.close())
            .await?;
        Ok(())
    }
}
