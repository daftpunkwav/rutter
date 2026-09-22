//! CDP implementation of the engine trait.
//!
//! Boundary: engine-level lifecycle over one chromiumoxide `Browser`.
//! CDP details (browser contexts, targets) are confined here and in the
//! sibling modules; callers see only `rutter-engine` types.

use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use chromiumoxide::cdp::browser_protocol::target::CreateBrowserContextParams;
use tokio::process::Child;
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

/// The browser process this engine spawned and owns. Killing on drop is
/// the backstop for abnormal teardown (a wedged browser under restart, a
/// failed post-connect check); the graceful path is [`Engine::shutdown`]'s
/// CDP `Browser.close`, which also reaches a browser whose intermediate
/// launcher process already exited. `Child` is not `Sync`, so the guard
/// parks it in a lock the drop path can take synchronously.
pub(crate) struct BrowserProcess(StdMutex<Child>);

impl BrowserProcess {
    /// Takes ownership of a freshly spawned browser process.
    pub fn new(child: Child) -> Self {
        Self(StdMutex::new(child))
    }
}

impl Drop for BrowserProcess {
    fn drop(&mut self) {
        if let Ok(mut child) = self.0.lock() {
            let _ = child.start_kill();
        }
    }
}

/// One launched CDP engine process.
pub struct CdpEngine {
    browser: SharedBrowser,
    #[allow(dead_code)] // held for its Drop side effect
    process: BrowserProcess,
    descriptor: EngineDescriptor,
    /// Whether contexts actually get browser-level isolation; flipped
    /// off the first time a context creation is refused (Electron).
    isolation: AtomicBool,
    context_counter: AtomicU64,
}

impl CdpEngine {
    /// Wraps a launched browser connection — and the process behind it —
    /// as an engine.
    pub fn new(
        browser: SharedBrowser,
        backend: EngineBackend,
        version: String,
        mode: LaunchMode,
        process: BrowserProcess,
    ) -> Self {
        let descriptor = EngineDescriptor {
            backend,
            version,
            capabilities: EngineCapabilities {
                // `headed` reports the mode this instance was launched
                // with rather than a static backend trait; headless is
                // always available to the CDP backend.
                headless: true,
                headed: mode == LaunchMode::Headed,
                screencast: true,
                per_context_isolation: true,
            },
        };
        Self {
            browser,
            process,
            descriptor,
            isolation: AtomicBool::new(true),
            context_counter: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl Engine for CdpEngine {
    fn descriptor(&self) -> EngineDescriptor {
        let mut descriptor = self.descriptor.clone();
        descriptor.capabilities.per_context_isolation = self.isolation.load(Ordering::Relaxed);
        descriptor
    }

    async fn create_context(
        &self,
        config: ContextConfig,
    ) -> Result<Arc<dyn ContextHandle>, EngineError> {
        let cdp_context_id = {
            let browser = self.browser.lock().await;
            match crate::error::with_deadline(
                "create_context",
                crate::error::COMMAND_TIMEOUT,
                browser.create_browser_context(CreateBrowserContextParams::default()),
            )
            .await
            {
                Ok(id) => Some(id),
                // Engines without browser-context support (the
                // Electron-based Rutter Browser runs everything in its
                // one visible window) fall back to the default context.
                Err(_) => {
                    self.isolation.store(false, Ordering::Relaxed);
                    None
                }
            }
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
