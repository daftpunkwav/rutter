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
        // Once one creation was refused the engine cannot isolate, and
        // every later call would fail the same way, so the flag also
        // short-circuits the command.
        let cdp_context_id = if self.isolation.load(Ordering::Relaxed) {
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
                // Only a refusal by the engine itself takes that path: a
                // timeout or transport failure must surface, or one
                // wedged call would silently strip isolation — and the
                // reported capability — for the engine's lifetime.
                Err(error) if crate::context::is_not_supported(&error) => {
                    self.isolation.store(false, Ordering::Relaxed);
                    None
                }
                Err(error) => return Err(error),
            }
        } else {
            None
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_tungstenite::tungstenite::Message;
    use futures::StreamExt;
    use std::net::SocketAddr;
    use std::process::Stdio;

    /// A fake CDP endpoint that mimics the Electron engine's refusals:
    /// `Target.createBrowserContext` is answered with the CDP server
    /// error "Not supported" (and counted), everything else with an
    /// empty success. Returns the address to dial and the refusal
    /// count.
    async fn spawn_refusing_endpoint() -> (SocketAddr, Arc<AtomicU64>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let refusals = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&refusals);
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = async_tungstenite::tokio::accept_async(stream)
                .await
                .unwrap();
            while let Some(Ok(message)) = ws.next().await {
                let Message::Text(text) = message else {
                    continue;
                };
                let request: serde_json::Value = serde_json::from_str(&text).unwrap();
                let id = request["id"].clone();
                let response = if request["method"] == "Target.createBrowserContext" {
                    counted.fetch_add(1, Ordering::Relaxed);
                    serde_json::json!({
                        "id": id,
                        "error": {"code": -32000, "message": "Not supported"}
                    })
                } else {
                    serde_json::json!({ "id": id, "result": {} })
                };
                ws.send(Message::text(response.to_string())).await.unwrap();
            }
        });
        (addr, refusals)
    }

    /// A stand-in child for the drop guard: the test exercises the CDP
    /// layer, so any process the guard can kill on drop works.
    fn dummy_child() -> Child {
        #[cfg(windows)]
        let mut command = {
            let mut command = tokio::process::Command::new("ping");
            command.args(["-n", "60", "127.0.0.1"]);
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = tokio::process::Command::new("sleep");
            command.arg("60");
            command
        };
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// The isolation fallback: when the engine refuses browser-context
    /// creation the way the Electron-based Rutter Browser does, the
    /// context still opens (everything runs in the default context),
    /// the first refusal flips the reported capability off, and later
    /// calls stop asking (the refusal short-circuits on the flag).
    #[tokio::test]
    async fn refused_context_creation_falls_back_to_the_default_context() {
        let (addr, refusals) = spawn_refusing_endpoint().await;
        let (browser, mut handler) = chromiumoxide::Browser::connect(format!("ws://{addr}"))
            .await
            .unwrap();
        // The handler stream must drain for command responses to flow.
        tokio::spawn(async move { while handler.next().await.is_some() {} });

        let engine = CdpEngine::new(
            Arc::new(Mutex::new(browser)),
            EngineBackend::Chromium,
            "fake/1.0".to_owned(),
            LaunchMode::Headless,
            BrowserProcess::new(dummy_child()),
        );
        assert!(
            engine.descriptor().capabilities.per_context_isolation,
            "an untried engine reports isolation"
        );

        let first = engine
            .create_context(ContextConfig::default())
            .await
            .expect("the refusal must fall back, not fail the caller");
        let second = engine
            .create_context(ContextConfig::default())
            .await
            .unwrap();
        assert_ne!(first.id(), second.id());
        assert!(
            !engine.descriptor().capabilities.per_context_isolation,
            "the refusal must flip the capability off"
        );
        assert_eq!(
            refusals.load(Ordering::Relaxed),
            1,
            "the second create_context must short-circuit on the flipped flag"
        );
    }
}
