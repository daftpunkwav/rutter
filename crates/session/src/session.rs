//! One session: a client's context, its active page, and its events.
//!
//! Boundary: the orchestration surface the MCP tools call. The session
//! owns exactly one context (blueprint §4); it tracks the active page,
//! executes actions through the [`crate::actions`] executor, and emits
//! the semantic events of blueprint §7.5. Recovery and storage state
//! are M2 concerns.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use rutter_core::action::{Action, Origin};
use rutter_core::cookie::Cookie;
use rutter_core::error::ActionError;
use rutter_core::ids::{PageId, SessionId};
use rutter_core::snapshot::Snapshot;
use rutter_engine::context::ContextHandle;
use rutter_engine::error::EngineError;
use rutter_engine::page::{PageHandle, Screenshot};
use rutter_events::{Backbone, Event};

use crate::actions::{Executor, PageOps};
use crate::config::SessionConfig;
use crate::error::SessionError;

/// One open page as reported by `tabs_list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabInfo {
    /// Stable page identifier.
    pub page: PageId,
    /// Current URL, empty when the page did not answer.
    pub url: String,
    /// Whether this page is the session's active page.
    pub active: bool,
}

/// The client-visible state of one session.
pub struct Session {
    id: SessionId,
    context: Arc<dyn ContextHandle>,
    backbone: Arc<Backbone>,
    config: SessionConfig,
    active: Mutex<Option<ActivePage>>,
}

struct ActivePage {
    id: PageId,
    handle: Arc<dyn PageHandle>,
}

impl Session {
    /// Creates a session over a freshly created context.
    pub(crate) fn new(
        id: SessionId,
        context: Arc<dyn ContextHandle>,
        backbone: Arc<Backbone>,
        config: SessionConfig,
    ) -> Self {
        Self {
            id,
            context,
            backbone,
            config,
            active: Mutex::new(None),
        }
    }

    /// The session's identifier.
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// The event backbone, for M2 consumers that replay history.
    pub fn backbone(&self) -> Arc<Backbone> {
        Arc::clone(&self.backbone)
    }

    /// The active page, opening the first page when none exists yet.
    pub async fn active_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
        {
            let active = self.lock_active();
            if let Some(page) = active.as_ref() {
                return Ok((page.id.clone(), Arc::clone(&page.handle)));
            }
        }
        let (page_id, handle) = self.context.open_page().await?;
        *self.lock_active() = Some(ActivePage {
            id: page_id.clone(),
            handle: Arc::clone(&handle),
        });
        self.backbone.publish(
            self.id.clone(),
            Event::PageOpened {
                page: page_id.clone(),
            },
        );
        Ok((page_id, handle))
    }

    /// Executes one action with the given origin and returns the fresh
    /// snapshot; requested/completed/failed land on the event backbone.
    pub async fn execute(&self, action: Action, origin: Origin) -> Result<Snapshot, SessionError> {
        let (page_id, page) = self.active_page().await.map_err(engine_error)?;
        self.backbone.publish(
            self.id.clone(),
            Event::ActionRequested {
                page: page_id.clone(),
                origin,
                action: action.clone(),
            },
        );

        let executor = Executor {
            session: &self.id,
            page_id: &page_id,
            page: page.as_ref(),
            config: &self.config,
            backbone: &self.backbone,
        };
        match executor.run(&action).await {
            Ok(snapshot) => {
                self.backbone.publish(
                    self.id.clone(),
                    Event::ActionCompleted {
                        page: page_id,
                        origin,
                        action,
                    },
                );
                Ok(snapshot)
            }
            Err(error) => {
                self.backbone.publish(
                    self.id.clone(),
                    Event::ActionFailed {
                        page: page_id,
                        origin,
                        action,
                        error: as_action_error(&self.id, &error),
                    },
                );
                Err(error)
            }
        }
    }

    /// Renders the active page as a snapshot; no events, no auto-wait.
    pub async fn snapshot(&self) -> Result<Snapshot, SessionError> {
        let (_, page) = self.active_page().await.map_err(engine_error)?;
        PageOps {
            page: page.as_ref(),
            config: &self.config,
        }
        .snapshot()
        .await
    }

    /// Captures the active page.
    pub async fn screenshot(&self) -> Result<Screenshot, SessionError> {
        let (_, page) = self.active_page().await.map_err(engine_error)?;
        page.capture_screenshot()
            .await
            .map_err(SessionError::Engine)
    }

    /// Polls the active page until `needle` appears in its text.
    pub async fn wait_for(&self, needle: &str, budget: Duration) -> Result<Snapshot, SessionError> {
        let (_, page) = self.active_page().await.map_err(engine_error)?;
        PageOps {
            page: page.as_ref(),
            config: &self.config,
        }
        .wait_for(needle, budget)
        .await
    }

    /// Lists the session's pages with their URLs.
    pub async fn tabs(&self) -> Vec<TabInfo> {
        let active_id = self.lock_active().as_ref().map(|page| page.id.clone());
        let mut tabs = Vec::new();
        for page_id in self.context.pages() {
            // A crashed or detached page contributes an empty URL rather
            // than failing the whole listing.
            let url = match self.context.page(page_id.clone()) {
                Some(page) => page
                    .evaluate("location.href")
                    .await
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_default(),
                None => String::new(),
            };
            tabs.push(TabInfo {
                active: active_id.as_ref() == Some(&page_id),
                page: page_id,
                url,
            });
        }
        tabs
    }

    /// Makes another page active and returns its snapshot.
    pub async fn select_tab(&self, page_id: PageId) -> Result<Snapshot, SessionError> {
        let handle = self
            .context
            .page(page_id.clone())
            .ok_or_else(|| unknown_tab(&page_id))?;
        *self.lock_active() = Some(ActivePage {
            id: page_id,
            handle: Arc::clone(&handle),
        });
        PageOps {
            page: handle.as_ref(),
            config: &self.config,
        }
        .snapshot()
        .await
    }

    /// Closes a page; closing the active page leaves no active page
    /// until the next navigation opens one.
    pub async fn close_tab(&self, page_id: PageId) -> Result<String, SessionError> {
        let was_active = self
            .lock_active()
            .as_ref()
            .is_some_and(|page| page.id == page_id);
        if was_active {
            *self.lock_active() = None;
        }
        self.context
            .close_page(page_id.clone())
            .await
            .map_err(engine_error)?;
        self.backbone.publish(
            self.id.clone(),
            Event::PageClosed {
                page: page_id.clone(),
            },
        );
        Ok(format!("closed {page_id}"))
    }

    /// Sets cookies on the session's context.
    pub async fn set_cookies(&self, cookies: &[Cookie]) -> Result<(), SessionError> {
        self.context
            .set_cookies(cookies)
            .await
            .map_err(engine_error)
    }

    /// Closes every page of the session; page-close failures are
    /// tolerated so a session always closes.
    pub async fn close(&self) {
        for page_id in self.context.pages() {
            let _ = self.context.close_page(page_id).await;
        }
        *self.lock_active() = None;
    }

    fn lock_active(&self) -> std::sync::MutexGuard<'_, Option<ActivePage>> {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Maps engine failures onto the action taxonomy for event payloads.
pub(crate) fn as_action_error(session: &SessionId, error: &SessionError) -> ActionError {
    match error {
        SessionError::Action(action) => action.clone(),
        SessionError::Engine(engine) => match engine {
            EngineError::Terminated => ActionError::EngineTerminated {
                session: session.clone(),
            },
            EngineError::NavigationFailed { url, detail } => ActionError::NavigationFailed {
                url: url.clone(),
                cause: crate::actions::transport_cause(detail),
            },
            EngineError::Timeout { elapsed, .. } => ActionError::TimedOut {
                phase: rutter_core::error::WaitPhase::Act,
                elapsed: *elapsed,
            },
            other => ActionError::Internal {
                detail: other.to_string(),
            },
        },
    }
}

fn engine_error(error: EngineError) -> SessionError {
    SessionError::Engine(error)
}

fn unknown_tab(page_id: &PageId) -> SessionError {
    SessionError::Action(ActionError::NotInteractable {
        reference: rutter_core::reference::Reference::new(page_id.as_str()),
        reason: format!("no open page with id '{page_id}'"),
    })
}
