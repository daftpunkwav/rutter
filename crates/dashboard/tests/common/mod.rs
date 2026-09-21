//! Shared fixtures for the dashboard's functional tests: a scriptable
//! engine double chain (launcher → engine → context → page) so the real
//! HTTP routes and WebSocket loop run without a browser.

// Each test target compiles this module and uses a subset of it.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use serde_json::{Value, json};

use rutter_core::cookie::Cookie;
use rutter_core::ids::{ContextId, PageId};
use rutter_engine::config::{ContextConfig, LaunchMode};
use rutter_engine::context::ContextHandle;
use rutter_engine::descriptor::{EngineBackend, EngineCapabilities, EngineDescriptor};
use rutter_engine::engine::Engine;
use rutter_engine::error::EngineError;
use rutter_engine::health::HealthReport;
use rutter_engine::input::InputEvent;
use rutter_engine::page::{ImageFormat, PageHandle, ScreencastStream, Screenshot};
use rutter_engine::supervisor::EngineLauncher;

fn lock<T, R>(mutex: &Mutex<T>, f: impl FnOnce(&mut T) -> R) -> R {
    let mut guard = mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

struct PageInner {
    url: Mutex<String>,
}

/// A page double answering the few scripts the dashboard flow touches
/// (`location.href` for snapshots, nothing else).
#[derive(Clone, Default)]
pub struct FlowPage {
    inner: Arc<PageInner>,
}

impl PageInner {
    fn new() -> Self {
        Self {
            url: Mutex::new(String::new()),
        }
    }
}

#[async_trait]
impl PageHandle for FlowPage {
    async fn navigate(&self, url: &str) -> Result<String, EngineError> {
        lock(&self.inner.url, |current| *current = url.to_owned());
        Ok(url.to_owned())
    }

    async fn reload(&self) -> Result<(), EngineError> {
        Ok(())
    }

    async fn go_back(&self) -> Result<String, EngineError> {
        Ok(lock(&self.inner.url, |url| url.clone()))
    }

    async fn go_forward(&self) -> Result<String, EngineError> {
        Ok(lock(&self.inner.url, |url| url.clone()))
    }

    async fn evaluate(&self, expression: &str) -> Result<Value, EngineError> {
        if expression.contains("location.href") {
            Ok(json!(lock(&self.inner.url, |url| url.clone())))
        } else {
            Ok(Value::Null)
        }
    }

    async fn dispatch_input(&self, _event: InputEvent) -> Result<(), EngineError> {
        Ok(())
    }

    async fn capture_screenshot(&self) -> Result<Screenshot, EngineError> {
        Ok(Screenshot {
            format: ImageFormat::Png,
            data: vec![0; 1024],
        })
    }

    async fn start_screencast(&self) -> Result<ScreencastStream, EngineError> {
        let (_sender, receiver) = tokio::sync::mpsc::channel(1);
        Ok(ScreencastStream::new(receiver))
    }
}

impl Default for PageInner {
    fn default() -> Self {
        Self::new()
    }
}

struct ContextInner {
    id: ContextId,
    pages: Mutex<VecDeque<(PageId, Arc<FlowPage>)>>,
    counter: AtomicU64,
    closed: AtomicBool,
    set_cookie_calls: Mutex<Vec<Vec<Cookie>>>,
}

/// A context double whose pages are [`FlowPage`]s; clones share state.
#[derive(Clone)]
pub struct FlowContext {
    inner: Arc<ContextInner>,
}

impl FlowContext {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ContextInner {
                id: ContextId::new("ctx-flow"),
                pages: Mutex::new(VecDeque::new()),
                counter: AtomicU64::new(0),
                closed: AtomicBool::new(false),
                set_cookie_calls: Mutex::new(Vec::new()),
            }),
        }
    }
}

impl Default for FlowContext {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ContextHandle for FlowContext {
    fn id(&self) -> ContextId {
        self.inner.id.clone()
    }

    fn pages(&self) -> Vec<PageId> {
        lock(&self.inner.pages, |pages| {
            pages.iter().map(|(id, _)| id.clone()).collect()
        })
    }

    async fn open_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(EngineError::Terminated);
        }
        let serial = self.inner.counter.fetch_add(1, Ordering::SeqCst);
        let id = PageId::new(format!("{}:page-{}", self.inner.id, serial));
        let page = Arc::new(FlowPage::default());
        lock(&self.inner.pages, |pages| {
            pages.push_back((id.clone(), Arc::clone(&page)))
        });
        Ok((id, page))
    }

    fn page(&self, id: PageId) -> Option<Arc<dyn PageHandle>> {
        lock(&self.inner.pages, |pages| {
            pages
                .iter()
                .find(|(page_id, _)| *page_id == id)
                .map(|(_, handle)| Arc::clone(handle) as Arc<dyn PageHandle>)
        })
    }

    async fn close_page(&self, id: PageId) -> Result<(), EngineError> {
        lock(&self.inner.pages, |pages| {
            pages.retain(|(page_id, _)| *page_id != id)
        });
        Ok(())
    }

    async fn set_cookies(&self, cookies: &[Cookie]) -> Result<(), EngineError> {
        lock(&self.inner.set_cookie_calls, |calls| {
            calls.push(cookies.to_vec())
        });
        Ok(())
    }

    async fn cookies(&self) -> Result<Vec<Cookie>, EngineError> {
        Ok(Vec::new())
    }

    async fn close(&self) -> Result<(), EngineError> {
        self.inner.closed.store(true, Ordering::SeqCst);
        lock(&self.inner.pages, |pages| pages.clear());
        Ok(())
    }
}

/// An engine handing out [`FlowContext`]s.
#[derive(Default)]
pub struct FlowEngine {
    contexts: Mutex<Vec<FlowContext>>,
}

impl FlowEngine {
    /// The context created by the `serial`th `create_context` call.
    pub fn context(&self, serial: usize) -> FlowContext {
        lock(&self.contexts, |contexts| {
            contexts.get(serial).cloned().expect("context with serial")
        })
    }
}

#[async_trait]
impl Engine for FlowEngine {
    fn descriptor(&self) -> EngineDescriptor {
        EngineDescriptor {
            backend: EngineBackend::ChromiumHeadlessShell,
            version: "flow".to_owned(),
            capabilities: EngineCapabilities {
                headless: true,
                headed: false,
                screencast: false,
                per_context_isolation: true,
            },
        }
    }

    async fn create_context(
        &self,
        _config: ContextConfig,
    ) -> Result<Arc<dyn ContextHandle>, EngineError> {
        let context = FlowContext::new();
        lock(&self.contexts, |contexts| contexts.push(context.clone()));
        Ok(Arc::new(context))
    }

    async fn health(&self) -> Result<HealthReport, EngineError> {
        Ok(HealthReport {
            healthy: true,
            backend_version: Some("flow".to_owned()),
            detail: None,
        })
    }

    async fn shutdown(&self) -> Result<(), EngineError> {
        Ok(())
    }
}

/// A launcher producing [`FlowEngine`]s, keeping typed handles so tests
/// can reach the engines without downcasts.
#[derive(Default)]
pub struct FlowLauncher {
    engines: Mutex<Vec<Arc<FlowEngine>>>,
}

impl FlowLauncher {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait]
impl EngineLauncher for FlowLauncher {
    fn describe(&self) -> String {
        "flow".to_owned()
    }

    async fn launch(&self, _mode: LaunchMode) -> Result<Arc<dyn Engine>, EngineError> {
        let engine = Arc::new(FlowEngine::default());
        lock(&self.engines, |engines| engines.push(Arc::clone(&engine)));
        Ok(engine)
    }
}
