//! CDP implementation of context-scoped operations.
//!
//! Boundary: one CDP browser context per handle, enforcing the page cap
//! from its configuration. Storage state persistence is a session-layer
//! concern and never happens here.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use chromiumoxide::cdp::browser_protocol::browser::BrowserContextId;
use chromiumoxide::cdp::browser_protocol::target::{CloseTargetParams, CreateTargetParams};
use tokio::sync::Mutex;

use rutter_core::ids::{ContextId, PageId};
use rutter_engine::config::ContextConfig;
use rutter_engine::context::ContextHandle;
use rutter_engine::error::EngineError;
use rutter_engine::page::PageHandle;

use crate::page::CdpPage;

/// Pages of one CDP browser context, keyed by rutter page id.
type PageMap = HashMap<PageId, Arc<CdpPage>>;

/// One isolated CDP browser context.
pub struct CdpContext {
    id: ContextId,
    /// Shared browser connection handle; `Browser` itself is not
    /// `Clone`, so every layer funnels through this mutex.
    browser: Arc<Mutex<chromiumoxide::Browser>>,
    cdp_context_id: BrowserContextId,
    config: ContextConfig,
    pages: Mutex<PageMap>,
    page_counter: AtomicU64,
}

impl CdpContext {
    /// Creates the rutter-side handle for a CDP browser context.
    pub fn new(
        id: ContextId,
        browser: Arc<Mutex<chromiumoxide::Browser>>,
        cdp_context_id: BrowserContextId,
        config: ContextConfig,
    ) -> Self {
        Self {
            id,
            browser,
            cdp_context_id,
            config,
            pages: Mutex::new(HashMap::new()),
            page_counter: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl ContextHandle for CdpContext {
    fn id(&self) -> ContextId {
        self.id.clone()
    }

    fn pages(&self) -> Vec<PageId> {
        // The async mutex only guards mutations; reading the key set
        // blocks briefly and never awaits, so this stays sync per trait.
        self.pages.blocking_lock().keys().cloned().collect()
    }

    async fn open_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
        {
            let pages = self.pages.lock().await;
            if pages.len() >= self.config.max_pages {
                return Err(EngineError::Capacity {
                    detail: format!(
                        "context '{}' reached its cap of {} pages; close a page first",
                        self.id, self.config.max_pages
                    ),
                });
            }
        }

        let mut params = CreateTargetParams::new("about:blank");
        params.browser_context_id = Some(self.cdp_context_id.clone());
        let page = {
            let browser = self.browser.lock().await;
            browser.new_page(params).await.map_err(crate::error::fold)?
        };

        let serial = self.page_counter.fetch_add(1, Ordering::Relaxed);
        let page_id = PageId::new(format!("{}:page-{}", self.id, serial));
        let handle = Arc::new(CdpPage::new(page, self.config.navigation_timeout));
        self.pages
            .lock()
            .await
            .insert(page_id.clone(), Arc::clone(&handle));
        Ok((page_id, handle))
    }

    fn page(&self, id: PageId) -> Option<Arc<dyn PageHandle>> {
        self.pages
            .blocking_lock()
            .get(&id)
            .cloned()
            .map(|handle| handle as Arc<dyn PageHandle>)
    }

    async fn close_page(&self, id: PageId) -> Result<(), EngineError> {
        let handle = self.pages.lock().await.remove(&id);
        let Some(handle) = handle else {
            return Ok(());
        };
        let target_id = handle.target_id();
        let browser = self.browser.lock().await;
        browser
            .execute(CloseTargetParams::new(target_id))
            .await
            .map_err(crate::error::fold)?;
        Ok(())
    }
}
