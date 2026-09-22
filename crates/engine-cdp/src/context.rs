//! CDP implementation of context-scoped operations.
//!
//! Boundary: at most one CDP browser context per handle (engines
//! without context support run every page in the default one),
//! enforcing the page cap from its configuration. Storage state
//! persistence is a session-layer concern and never happens here.

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
    CloseTargetParams, CreateTargetParams, GetTargetsParams, TargetId, TargetInfo,
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
    /// `None` on engines without browser-context support (the
    /// Electron-based Rutter Browser): every page lives in the default
    /// context.
    cdp_context_id: Option<BrowserContextId>,
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
        cdp_context_id: Option<BrowserContextId>,
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
        params.browser_context_id = self.cdp_context_id.clone();
        let (page, created) = {
            let browser = self.browser.lock().await;
            match crate::error::with_deadline(
                "open_page",
                crate::error::COMMAND_TIMEOUT,
                browser.new_page(params),
            )
            .await
            {
                Ok(page) => (page, true),
                // Engines without target creation fall back to the page
                // surface they already have: the Electron-based Rutter
                // Browser answers Target.createTarget with "Not
                // supported" because its one visible window is the
                // surface, and driving that is the point.
                Err(error) if is_not_supported(&error) => (
                    attach_existing_surface(&browser).await.map_err(|_| error)?,
                    false,
                ),
                Err(error) => return Err(error),
            }
        };

        let serial = self.page_counter.fetch_add(1, Ordering::Relaxed);
        let page_id = PageId::new(format!("{}:page-{}", self.id, serial));
        let handle = Arc::new(if created {
            CdpPage::new(
                page,
                self.config.navigation_timeout,
                Some(self.config.screenshot_min_interval),
            )
        } else {
            CdpPage::attach(
                page,
                self.config.navigation_timeout,
                Some(self.config.screenshot_min_interval),
            )
        });

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
            // An attached surface belongs to the engine's UI and is
            // never closed; see [`CdpPage::attach`].
            if handle.closable() {
                let target_id = handle.target_id();
                let browser = self.browser.lock().await;
                // Same wedged-handler threat model as every call here:
                // the cleanup must be bounded or a hung browser would
                // park `open_page` forever while holding the browser
                // mutex.
                let _ = crate::error::with_deadline(
                    "close_overflow_page",
                    crate::error::COMMAND_TIMEOUT,
                    browser.execute(CloseTargetParams::new(target_id)),
                )
                .await;
            }
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
        // Disposing the context closes every target inside it. The
        // default context (context-less engines) cannot be disposed —
        // nothing to do there.
        let Some(context_id) = self.cdp_context_id.clone() else {
            self.lock_pages().drain();
            return Ok(());
        };
        let browser = self.browser.lock().await;
        let disposed = crate::error::with_deadline(
            "close_context",
            crate::error::COMMAND_TIMEOUT,
            browser.dispose_browser_context(context_id),
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
        // An attached surface is the engine's own window content and
        // outlives the handle: closing its target would blank the
        // browser's UI, so the registration alone goes.
        if handle.closable() {
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
            browser_context_id: self.cdp_context_id.clone(),
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
                browser_context_id: self.cdp_context_id.clone(),
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

/// Whether the browser refused the command itself rather than failing
/// it: the Electron engine's CDP endpoint answers commands it does not
/// implement (target creation, browser contexts) with the CDP server
/// error "Not supported", which [`crate::error::fold`] keeps verbatim
/// in the detail text.
pub(crate) fn is_not_supported(error: &EngineError) -> bool {
    matches!(error, EngineError::Internal { detail }
        if detail.to_lowercase().contains("not supported"))
}

/// Whether a target URL is the shell's own toolbar document — the
/// window chrome that `browser/main.js` loads, so renaming the file
/// there must update this matcher. The toolbar is loaded from the app
/// directory, so its URL is always a
/// `file://` one; anchoring the match there keeps a web page that
/// navigates itself to `https://evil.com/toolbar.html` from being
/// mistaken for shell UI (which would fail the attach fail-closed) —
/// web content cannot initiate `file://` navigations.
fn is_toolbar_document(url: &str) -> bool {
    url.starts_with("file://") && url.ends_with("/toolbar.html")
}

/// Picks the target a page handle may drive: the first `page`-typed
/// target that is not the shell's own toolbar document. Pure so the
/// attach rules are unit-testable without a browser connection.
fn pick_drivable_target(infos: &[TargetInfo]) -> Option<TargetId> {
    infos
        .iter()
        .filter(|info| info.r#type == "page")
        .find(|info| !is_toolbar_document(&info.url))
        .map(|info| info.target_id.clone())
}

/// The browser's own page surface, for engines that cannot create
/// targets: lists the targets and attaches to the one
/// [`pick_drivable_target`] selects. Fails when the browser offers
/// nothing drivable, which keeps the original "Not supported" error
/// the caller holds.
async fn attach_existing_surface(
    browser: &chromiumoxide::Browser,
) -> Result<chromiumoxide::Page, EngineError> {
    let response = crate::error::with_deadline(
        "list_targets",
        crate::error::COMMAND_TIMEOUT,
        browser.execute(GetTargetsParams::default()),
    )
    .await?;
    let Some(target_id) = pick_drivable_target(&response.result.target_infos) else {
        return Err(EngineError::Internal {
            detail: "the engine exposes no page surface to attach to".to_owned(),
        });
    };
    crate::error::with_deadline(
        "attach_surface",
        crate::error::COMMAND_TIMEOUT,
        browser.get_page(target_id),
    )
    .await
}

/// Whether the browser still knows a target; the answer defaults to
/// `true` when the probe itself fails, so an unverifiable target keeps
/// the original close error instead of being torn down on a guess. The
/// probe carries a deadline like every call here: on a wedged handler
/// the honest answer is "unverifiable", not an unbounded wait that pins
/// the shared browser mutex (and `close_page`) forever.
async fn target_still_exists(
    browser: &chromiumoxide::Browser,
    target_id: chromiumoxide::cdp::browser_protocol::target::TargetId,
) -> bool {
    match crate::error::with_deadline(
        "close_page_probe",
        crate::error::COMMAND_TIMEOUT,
        browser.execute(GetTargetsParams::default()),
    )
    .await
    {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::fold;
    use chromiumoxide::error::CdpError;
    use std::time::Duration;

    #[test]
    fn only_the_app_directory_toolbar_document_is_excluded() {
        // The shell loads the toolbar from its own directory.
        assert!(is_toolbar_document("file:///C:/apps/rutter/toolbar.html"));
        assert!(is_toolbar_document("file:///home/u/rutter/toolbar.html"));
        // A web page that names itself toolbar.html is drivable, not
        // shell UI.
        assert!(!is_toolbar_document("https://evil.com/toolbar.html"));
        assert!(!is_toolbar_document("http://localhost/toolbar.html"));
        // Other file documents stay drivable too.
        assert!(!is_toolbar_document("file:///C:/apps/rutter/start.html"));
    }

    /// Builds a target listing entry the way `Target.getTargets`
    /// reports one.
    fn target(id: &str, kind: &str, url: &str) -> TargetInfo {
        TargetInfo::builder()
            .target_id(TargetId::new(id))
            .r#type(kind)
            .title(id)
            .url(url)
            .attached(false)
            .can_access_opener(false)
            .build()
            .unwrap()
    }

    #[test]
    fn attach_picks_the_first_drivable_page_skipping_shell_ui() {
        // The toolbar window typically lists first; the drivable
        // surface sits behind it. Non-page targets (workers, iframes)
        // are never drivable no matter where they list.
        let infos = [
            target("toolbar", "page", "file:///C:/apps/rutter/toolbar.html"),
            target("worker", "service_worker", "https://example.com/sw.js"),
            target("frame", "iframe", "https://example.com/"),
            target("start", "page", "file:///C:/apps/rutter/start.html"),
        ];
        assert_eq!(pick_drivable_target(&infos), Some(TargetId::new("start")));
    }

    #[test]
    fn attach_fails_closed_when_only_shell_ui_is_exposed() {
        // Driving the toolbar would navigate the shell's own UI away,
        // so nothing drivable means the caller keeps its refusal error
        // instead of attaching to the wrong surface.
        let infos = [target(
            "toolbar",
            "page",
            "file:///C:/apps/rutter/toolbar.html",
        )];
        assert_eq!(pick_drivable_target(&infos), None);
        assert_eq!(pick_drivable_target(&[]), None);
    }

    #[test]
    fn a_cdp_refusal_is_recognized_as_not_supported() {
        // The exact shape a real refusal takes: the CDP server error
        // rides through `fold` into an Internal detail. The match is
        // case-insensitive because the wording belongs to the engine.
        let refusal = fold(CdpError::Chrome(chromiumoxide::types::Error {
            code: -32000,
            message: "Not supported".to_owned(),
        }));
        assert!(
            is_not_supported(&refusal),
            "real refusal shape must match: {refusal}"
        );
        assert!(is_not_supported(&EngineError::Internal {
            detail: "cdp command failed: Error -32000: not supported".to_owned(),
        }));
    }

    #[test]
    fn a_wedged_or_unrelated_engine_error_never_counts_as_refusal() {
        // A timeout or transport failure must surface unchanged: one
        // wedged call must not silently strip per-context isolation for
        // the engine's lifetime.
        assert!(!is_not_supported(&EngineError::Timeout {
            operation: "create_context".to_owned(),
            elapsed: Duration::from_secs(30),
        }));
        assert!(!is_not_supported(&EngineError::Internal {
            detail: "cdp layer failure: connection closed before a response".to_owned(),
        }));
        assert!(!is_not_supported(&EngineError::Capacity {
            detail: "cap reached".to_owned(),
        }));
    }
}
