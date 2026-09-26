//! Context-scoped engine operations.
//!
//! Boundary: one isolated cookie/storage unit inside an engine. Storage
//! state persistence and replay across restarts live in `rutter-session`;
//! a context only manages the pages that exist inside it.

use std::sync::Arc;

use async_trait::async_trait;

use rutter_core::cookie::Cookie;
use rutter_core::ids::{ContextId, PageId};

use crate::error::EngineError;
use crate::page::PageHandle;

/// A page surface that exists in the engine but was not opened through
/// the context reporting it: a window the page itself opened
/// (`target=_blank`), a window the human opened, or an engine whose one
/// visible surface predates every session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignPage {
    /// Stable identifier that names this surface to
    /// [`ContextHandle::adopt_page`] and stays valid while the window
    /// lives.
    pub id: PageId,
    /// The surface's current URL.
    pub url: String,
}

/// Lifecycle of pages inside one context.
#[async_trait]
pub trait ContextHandle: Send + Sync {
    /// Identifier of this context.
    fn id(&self) -> ContextId;

    /// Pages currently open in this context.
    fn pages(&self) -> Vec<PageId>;

    /// Opens a new page, respecting the context's page cap.
    async fn open_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), EngineError>;

    /// Returns the handle for an open page, if it is still open.
    fn page(&self, id: PageId) -> Option<Arc<dyn PageHandle>>;

    /// Closes a page; closing the last page may close the context,
    /// depending on backend semantics.
    async fn close_page(&self, id: PageId) -> Result<(), EngineError>;

    /// The page surfaces the engine already shows that this context did
    /// not open (docs/tool-catalog.md: `tabs_list` discovery). Engines
    /// that cannot enumerate surfaces report none.
    async fn foreign_pages(&self) -> Result<Vec<ForeignPage>, EngineError> {
        Ok(Vec::new())
    }

    /// Brings a [`ContextHandle::foreign_pages`] surface under this
    /// context's tracking, returning its handle. Adopting registers an
    /// existing surface; it obeys the page cap's counting but never the
    /// cap itself, because the surface exists whether or not it is
    /// tracked. Fails for an id the engine no longer reports;
    /// re-adopting an already-tracked id is idempotent and hands back
    /// the registered handle.
    async fn adopt_page(&self, id: &PageId) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
        let _ = id;
        Err(EngineError::Unsupported {
            operation: "adopt_page".to_owned(),
            reason: "this engine cannot adopt pages it did not open".to_owned(),
        })
    }

    /// Sets cookies scoped to this context, replacing nothing: each
    /// cookie is written by name/domain/path per backend semantics.
    async fn set_cookies(&self, cookies: &[Cookie]) -> Result<(), EngineError>;

    /// Reads every cookie scoped to this context (storage state
    /// capture, docs/sessions.md).
    async fn cookies(&self) -> Result<Vec<Cookie>, EngineError>;

    /// Closes the context and every page inside it. Idempotent: closing
    /// an already-closed context succeeds, so a session can always be
    /// torn down regardless of what happened before.
    async fn close(&self) -> Result<(), EngineError>;
}
