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

    /// Sets cookies scoped to this context, replacing nothing: each
    /// cookie is written by name/domain/path per backend semantics.
    async fn set_cookies(&self, cookies: &[Cookie]) -> Result<(), EngineError>;
}
