//! CDP implementation of context-scoped operations.
//!
//! Boundary: one CDP browser context per handle, enforcing the page cap
//! from its configuration. Storage state persistence is a session-layer
//! concern and never happens here.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use chromiumoxide::cdp::browser_protocol::browser::BrowserContextId;
use chromiumoxide::cdp::browser_protocol::network::TimeSinceEpoch;
use chromiumoxide::cdp::browser_protocol::network::{CookieParam, CookieSameSite};
use chromiumoxide::cdp::browser_protocol::storage::{GetCookiesParams, SetCookiesParams};
use chromiumoxide::cdp::browser_protocol::target::{
    CloseTargetParams, CreateTargetParams, GetTargetsParams,
};
use tokio::sync::Mutex as AsyncMutex;

use rutter_core::cookie::{Cookie, SameSite};
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
    browser: Arc<AsyncMutex<chromiumoxide::Browser>>,
    cdp_context_id: BrowserContextId,
    config: ContextConfig,
    /// Sync mutex on purpose: the `ContextHandle` reads (`pages`, `page`)
    /// are synchronous per trait, and guards are never held across an
    /// await.
    pages: Mutex<PageMap>,
    page_counter: AtomicU64,
}

impl CdpContext {
    /// Creates the rutter-side handle for a CDP browser context.
    pub fn new(
        id: ContextId,
        browser: Arc<AsyncMutex<chromiumoxide::Browser>>,
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

    /// Locks the page map, recovering from poisoning: page handles stay
    /// usable even if a caller panicked while holding the lock.
    fn lock_pages(&self) -> MutexGuard<'_, PageMap> {
        self.pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl ContextHandle for CdpContext {
    fn id(&self) -> ContextId {
        self.id.clone()
    }

    fn pages(&self) -> Vec<PageId> {
        self.lock_pages().keys().cloned().collect()
    }

    async fn open_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
        {
            let pages = self.lock_pages();
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
            crate::error::with_deadline(
                "open_page",
                crate::error::COMMAND_TIMEOUT,
                browser.new_page(params),
            )
            .await?
        };

        let serial = self.page_counter.fetch_add(1, Ordering::Relaxed);
        let page_id = PageId::new(format!("{}:page-{}", self.id, serial));
        let handle = Arc::new(CdpPage::new(
            page,
            self.config.navigation_timeout,
            Some(self.config.screenshot_min_interval),
        ));

        // The cap protects the shared engine process, so it is re-checked
        // under the lock that owns registration: a concurrent open may
        // have slipped past the pre-check. The guard's scope must end
        // before the awaits below (the future must stay `Send`).
        let over_cap = {
            let mut pages = self.lock_pages();
            let over = pages.len() >= self.config.max_pages;
            if !over {
                pages.insert(page_id.clone(), Arc::clone(&handle));
            }
            over
        };
        if over_cap {
            let target_id = handle.target_id();
            let browser = self.browser.lock().await;
            let _ = browser.execute(CloseTargetParams::new(target_id)).await;
            return Err(EngineError::Capacity {
                detail: format!(
                    "context '{}' reached its cap of {} pages; close a page first",
                    self.id, self.config.max_pages
                ),
            });
        }
        Ok((page_id, handle))
    }

    fn page(&self, id: PageId) -> Option<Arc<dyn PageHandle>> {
        self.lock_pages()
            .get(&id)
            .cloned()
            .map(|handle| handle as Arc<dyn PageHandle>)
    }

    async fn close(&self) -> Result<(), EngineError> {
        // Disposing the context closes every target inside it.
        let browser = self.browser.lock().await;
        let disposed = crate::error::with_deadline(
            "close_context",
            crate::error::COMMAND_TIMEOUT,
            browser.dispose_browser_context(self.cdp_context_id.clone()),
        )
        .await;
        // Disposing an already-dead context (engine restart raced the
        // session close) is a success: the caller wants it gone. CDP
        // answers such disposals with several texts — "not found" and
        // "Failed to find context with id ..." among them — all meaning
        // the context is already gone.
        match disposed {
            Ok(()) => {}
            Err(error) => {
                let message = error.to_string().to_lowercase();
                if message.contains("not found") || message.contains("failed to find") {
                    // Already gone: treat as success.
                } else {
                    return Err(error);
                }
            }
        }
        self.lock_pages().drain();
        Ok(())
    }

    async fn close_page(&self, id: PageId) -> Result<(), EngineError> {
        // Close the target first: only a confirmed close (or a target
        // that is already gone) removes the registration, so a failed
        // close can be retried while a vanished one cannot wedge the
        // page cap with a ghost entry.
        let handle = self.lock_pages().get(&id).cloned();
        let Some(handle) = handle else {
            return Ok(());
        };
        let target_id = handle.target_id();
        {
            let browser = self.browser.lock().await;
            let closed = crate::error::with_deadline(
                "close_page",
                crate::error::COMMAND_TIMEOUT,
                browser.execute(CloseTargetParams::new(target_id.clone())),
            )
            .await;
            if let Err(error) = closed {
                // A target the browser dropped on its own (a user closed
                // the tab, or the renderer crashed) answers CloseTarget
                // with an error; the registration must still go, or the
                // page cap counts a page that no longer exists.
                if target_still_exists(&browser, target_id).await {
                    return Err(error);
                }
            }
        }
        self.lock_pages().remove(&id);
        Ok(())
    }

    async fn set_cookies(&self, cookies: &[Cookie]) -> Result<(), EngineError> {
        if cookies.is_empty() {
            return Ok(());
        }
        let params = SetCookiesParams {
            cookies: cookies
                .iter()
                .map(|cookie| CookieParam {
                    name: cookie.name.clone(),
                    value: cookie.value.clone(),
                    url: None,
                    domain: Some(cookie.domain.clone()),
                    path: cookie.path.clone(),
                    secure: Some(cookie.secure),
                    http_only: Some(cookie.http_only),
                    same_site: cookie.same_site.map(cdp_same_site),
                    expires: cookie.expires.map(TimeSinceEpoch::new),
                    priority: None,
                    same_party: None,
                    source_scheme: None,
                    source_port: None,
                    partition_key: None,
                })
                .collect(),
            browser_context_id: Some(self.cdp_context_id.clone()),
        };
        let browser = self.browser.lock().await;
        crate::error::with_deadline(
            "set_cookies",
            crate::error::COMMAND_TIMEOUT,
            browser.execute(params),
        )
        .await?;
        Ok(())
    }

    async fn cookies(&self) -> Result<Vec<Cookie>, EngineError> {
        let browser = self.browser.lock().await;
        let response = crate::error::with_deadline(
            "cookies",
            crate::error::COMMAND_TIMEOUT,
            browser.execute(GetCookiesParams {
                browser_context_id: Some(self.cdp_context_id.clone()),
            }),
        )
        .await?;
        Ok(response
            .result
            .cookies
            .into_iter()
            .map(|cookie| Cookie {
                name: cookie.name,
                value: cookie.value,
                domain: cookie.domain,
                path: Some(cookie.path),
                secure: cookie.secure,
                http_only: cookie.http_only,
                same_site: cookie.same_site.map(cdp_same_site_back),
                expires: Some(cookie.expires),
            })
            .collect())
    }
}

/// Whether the browser still knows a target; the answer defaults to
/// `true` when the probe itself fails, so an unverifiable target keeps
/// the original close error instead of being torn down on a guess.
async fn target_still_exists(
    browser: &chromiumoxide::Browser,
    target_id: chromiumoxide::cdp::browser_protocol::target::TargetId,
) -> bool {
    match browser.execute(GetTargetsParams::default()).await {
        Ok(response) => response
            .result
            .target_infos
            .iter()
            .any(|info| info.target_id == target_id),
        Err(_) => true,
    }
}

/// Maps the CDP policy back onto the protocol-neutral enum.
fn cdp_same_site_back(same_site: CookieSameSite) -> SameSite {
    match same_site {
        CookieSameSite::Strict => SameSite::Strict,
        CookieSameSite::Lax => SameSite::Lax,
        CookieSameSite::None => SameSite::None,
    }
}

/// Maps the protocol-neutral policy onto the CDP enum.
fn cdp_same_site(same_site: SameSite) -> CookieSameSite {
    match same_site {
        SameSite::Strict => CookieSameSite::Strict,
        SameSite::Lax => CookieSameSite::Lax,
        SameSite::None => CookieSameSite::None,
    }
}
