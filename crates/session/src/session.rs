//! One session: a client's context, its pages, and its events.
//!
//! Boundary: the orchestration surface the MCP tools call. The session
//! owns exactly one context (blueprint §4), tracks its open pages and
//! their URLs, executes actions through the [`crate::actions`]
//! executor, enforces the policy with approvals (§7.6), and persists
//! its storage state after every change so a supervisor restart can
//! rebuild it (§7.4). Recovery swaps in a fresh context and replays.

use std::path::PathBuf;
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
use rutter_engine::page::{PageHandle, ScreencastStream, Screenshot};
use rutter_events::{Backbone, Event};
use rutter_policy::{ActionClass, ApprovalBroker, ApprovalOutcome, RuleSet, Verdict};

use crate::actions::{Executor, PageOps};
use crate::config::SessionConfig;
use crate::error::SessionError;
use crate::storage::StorageState;

/// One open page as reported by `tabs_list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabInfo {
    /// Stable page identifier.
    pub page: PageId,
    /// Last known URL.
    pub url: String,
    /// Whether this page is the session's active page.
    pub active: bool,
}

/// One tracked page with its last known URL.
struct PageSlot {
    id: PageId,
    url: String,
    handle: Arc<dyn PageHandle>,
    active: bool,
}

/// The client-visible state of one session.
pub struct Session {
    id: SessionId,
    backbone: Arc<Backbone>,
    config: SessionConfig,
    policy: Arc<RuleSet>,
    broker: Arc<ApprovalBroker>,
    /// Swappable so recovery can install a fresh context after an
    /// engine restart (blueprint §7.4).
    context: tokio::sync::RwLock<Arc<dyn ContextHandle>>,
    pages: Mutex<Vec<PageSlot>>,
    /// Where the storage state persists; `None` disables persistence.
    state_path: Option<PathBuf>,
    last_storage: Mutex<StorageState>,
}

impl Session {
    /// Creates a session over a freshly created context.
    pub(crate) fn new(
        id: SessionId,
        context: Arc<dyn ContextHandle>,
        backbone: Arc<Backbone>,
        config: SessionConfig,
        policy: Arc<RuleSet>,
        broker: Arc<ApprovalBroker>,
        state_path: Option<PathBuf>,
    ) -> Self {
        Self {
            id,
            backbone,
            config,
            policy,
            broker,
            context: tokio::sync::RwLock::new(context),
            pages: Mutex::new(Vec::new()),
            state_path,
            last_storage: Mutex::new(StorageState::default()),
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

    /// The approval broker decisions are submitted to.
    pub fn broker(&self) -> Arc<ApprovalBroker> {
        Arc::clone(&self.broker)
    }

    /// Enforces the policy for one operation against `url`: `Allow`
    /// passes, `Deny` fails with `ApprovalDenied`, and
    /// `RequireApproval` publishes the request and parks until a human
    /// answers or the window closes (blueprint §7.6).
    async fn enforce_policy(
        &self,
        class: ActionClass,
        url: &str,
        page_id: &PageId,
        action: &Action,
    ) -> Result<(), SessionError> {
        match self.policy.evaluate(class, url) {
            Verdict::Allow => return Ok(()),
            Verdict::Deny => {
                return Err(SessionError::Action(ActionError::ApprovalDenied {
                    reference: action_reference(action),
                }));
            }
            Verdict::RequireApproval => {}
        }

        let denied = || {
            SessionError::Action(ActionError::ApprovalDenied {
                reference: action_reference(action),
            })
        };

        let (request_id, receiver) = self.broker.open();
        self.backbone.publish(
            self.id.clone(),
            Event::ApprovalRequested {
                request_id: request_id.as_str().to_owned(),
                page: page_id.clone(),
                action: action.clone(),
            },
        );
        let outcome = self
            .broker
            .wait(&request_id, receiver, self.policy.approval_timeout())
            .await;
        let granted = outcome == ApprovalOutcome::Granted;
        self.backbone.publish(
            self.id.clone(),
            Event::ApprovalResolved {
                request_id: request_id.as_str().to_owned(),
                granted,
            },
        );
        match outcome {
            ApprovalOutcome::Granted => Ok(()),
            ApprovalOutcome::Denied => Err(denied()),
            ApprovalOutcome::TimedOut => Err(SessionError::Action(ActionError::ApprovalTimedOut {
                waited: self.policy.approval_timeout(),
            })),
        }
    }

    /// The active page, opening the first page when none exists yet.
    async fn active_page(&self) -> Result<Arc<dyn PageHandle>, EngineError> {
        {
            let pages = self.lock_pages();
            if let Some(slot) = pages.iter().find(|slot| slot.active) {
                return Ok(Arc::clone(&slot.handle));
            }
        }
        let context = self.context.read().await.clone();
        let (page_id, handle) = context.open_page().await?;
        // Decide under the lock without awaiting: either register this
        // page as the active one, or note that a racing caller already
        // opened one (its page wins; this call's page is closed after
        // the lock is released — guards never cross an await).
        let raced = {
            let mut pages = self.lock_pages();
            let existing = pages
                .iter()
                .find(|slot| slot.active)
                .map(|slot| Arc::clone(&slot.handle));
            if existing.is_none() {
                pages.push(PageSlot {
                    id: page_id.clone(),
                    url: String::new(),
                    handle: Arc::clone(&handle),
                    active: true,
                });
            }
            existing
        };
        if let Some(existing) = raced {
            let _ = context.close_page(page_id).await;
            return Ok(existing);
        }
        self.backbone
            .publish(self.id.clone(), Event::PageOpened { page: page_id });
        Ok(handle)
    }

    /// Executes one action with the given origin and returns the fresh
    /// snapshot; requested/completed/failed land on the event backbone.
    /// After a completed action the page URL and the storage state are
    /// refreshed (blueprint §7.4: persist on change).
    pub async fn execute(&self, action: Action, origin: Origin) -> Result<Snapshot, SessionError> {
        let page = self.active_page().await.map_err(engine_error)?;
        let page_id = {
            let pages = self.lock_pages();
            match pages.iter().find(|slot| slot.active) {
                Some(slot) => slot.id.clone(),
                // active_page guarantees an active slot; an empty id is
                // harmless for event payloads.
                None => PageId::new(String::new()),
            }
        };

        // Supervision gate (blueprint §7.6): agent-origin actions are
        // evaluated against the current page URL; human-origin actions
        // bypass approval and are recorded identically.
        let ops = PageOps {
            page: page.as_ref(),
            config: &self.config,
        };
        if origin == Origin::Agent {
            let url = ops.url().await;
            self.enforce_policy(rutter_policy::class_of(&action), &url, &page_id, &action)
                .await?;
        }

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
        let result = executor.run(&action).await;
        match &result {
            Ok(_) => self.backbone.publish(
                self.id.clone(),
                Event::ActionCompleted {
                    page: page_id.clone(),
                    origin,
                    action: action.clone(),
                },
            ),
            Err(error) => self.backbone.publish(
                self.id.clone(),
                Event::ActionFailed {
                    page: page_id.clone(),
                    origin,
                    action: action.clone(),
                    error: as_action_error(&self.id, error),
                },
            ),
        };

        // Refresh the tracked URL, then persist the changed state
        // (blueprint §7.4: persist per session on change).
        let url = ops.url().await;
        self.lock_pages()
            .iter_mut()
            .filter(|slot| slot.active)
            .for_each(|slot| slot.url = url.clone());
        self.persist_storage(&page, &url).await;

        result
    }

    /// Renders the active page as a snapshot; no events, no auto-wait.
    pub async fn snapshot(&self) -> Result<Snapshot, SessionError> {
        let page = self.active_page().await.map_err(engine_error)?;
        PageOps {
            page: page.as_ref(),
            config: &self.config,
        }
        .snapshot()
        .await
    }

    /// Starts a live screencast of the active page; the stream is
    /// observation only — dropping it stops the capture (blueprint
    /// §7.7: on-demand, dashboard never executes actions).
    pub async fn screencast(&self) -> Result<ScreencastStream, SessionError> {
        let page = self.active_page().await.map_err(engine_error)?;
        page.start_screencast().await.map_err(SessionError::Engine)
    }

    /// Captures the active page.
    pub async fn screenshot(&self) -> Result<Screenshot, SessionError> {
        let page = self.active_page().await.map_err(engine_error)?;
        page.capture_screenshot()
            .await
            .map_err(SessionError::Engine)
    }

    /// Polls the active page until `needle` appears in its text.
    pub async fn wait_for(&self, needle: &str, budget: Duration) -> Result<Snapshot, SessionError> {
        let page = self.active_page().await.map_err(engine_error)?;
        PageOps {
            page: page.as_ref(),
            config: &self.config,
        }
        .wait_for(needle, budget)
        .await
    }

    /// Lists the session's pages with their last known URLs.
    pub async fn tabs(&self) -> Vec<TabInfo> {
        self.lock_pages()
            .iter()
            .map(|slot| TabInfo {
                page: slot.id.clone(),
                url: slot.url.clone(),
                active: slot.active,
            })
            .collect()
    }

    /// Opens a new page and makes it active (tabs_new tool).
    pub async fn open_tab(&self) -> Result<PageId, SessionError> {
        let context = self.context.read().await.clone();
        let (page_id, handle) = context.open_page().await.map_err(engine_error)?;
        self.lock_pages()
            .iter_mut()
            .for_each(|slot| slot.active = false);
        self.lock_pages().push(PageSlot {
            id: page_id.clone(),
            url: String::new(),
            handle: Arc::clone(&handle),
            active: true,
        });
        self.backbone.publish(
            self.id.clone(),
            Event::PageOpened {
                page: page_id.clone(),
            },
        );
        Ok(page_id)
    }

    /// Makes another page active and returns its snapshot.
    pub async fn select_tab(&self, page_id: PageId) -> Result<Snapshot, SessionError> {
        let handle = {
            let mut pages = self.lock_pages();
            let Some(slot) = pages.iter_mut().find(|slot| slot.id == page_id) else {
                return Err(unknown_tab(&page_id));
            };
            let handle = Arc::clone(&slot.handle);
            pages
                .iter_mut()
                .for_each(|slot| slot.active = slot.id == page_id);
            handle
        };
        PageOps {
            page: handle.as_ref(),
            config: &self.config,
        }
        .snapshot()
        .await
    }

    /// Closes a page; closing the active page promotes the first
    /// remaining page.
    pub async fn close_tab(&self, page_id: PageId) -> Result<String, SessionError> {
        let context = self.context.read().await.clone();
        context
            .close_page(page_id.clone())
            .await
            .map_err(engine_error)?;
        {
            let mut pages = self.lock_pages();
            pages.retain(|slot| slot.id != page_id);
            if !pages.is_empty() && !pages.iter().any(|slot| slot.active) {
                pages[0].active = true;
            }
        }
        self.backbone.publish(
            self.id.clone(),
            Event::PageClosed {
                page: page_id.clone(),
            },
        );
        Ok(format!("closed {page_id}"))
    }

    /// Sets cookies on the session's context, subject to the policy's
    /// `cookies` class rules (approval-required by default). Policy
    /// events carry `Reload` as the closest action payload since cookie
    /// changes have no `Action` variant.
    pub async fn set_cookies(&self, cookies: &[Cookie]) -> Result<(), SessionError> {
        let page = self.active_page().await.map_err(engine_error)?;
        let page_id = {
            let pages = self.lock_pages();
            match pages.iter().find(|slot| slot.active) {
                Some(slot) => slot.id.clone(),
                None => PageId::new(String::new()),
            }
        };
        let url = PageOps {
            page: page.as_ref(),
            config: &self.config,
        }
        .url()
        .await;
        self.enforce_policy(ActionClass::Cookies, &url, &page_id, &Action::Reload)
            .await?;
        let context = self.context.read().await.clone();
        context.set_cookies(cookies).await.map_err(engine_error)?;
        self.persist_storage(&page, &url).await;
        Ok(())
    }

    /// Captures the session's storage state (cookies plus localStorage
    /// of every open page).
    pub async fn capture_storage(&self) -> StorageState {
        let context = self.context.read().await.clone();
        let pairs = self
            .lock_pages()
            .iter()
            .map(|slot| (slot.url.clone(), Arc::clone(&slot.handle)))
            .collect::<Vec<_>>();
        StorageState::capture(context.as_ref(), &pairs).await
    }

    /// Saves the current storage state to the session's persistence
    /// file (explicit save; blueprint §7.4).
    pub async fn save_storage(&self) -> Result<(), SessionError> {
        let page = self.active_page().await.map_err(engine_error)?;
        let url = PageOps {
            page: page.as_ref(),
            config: &self.config,
        }
        .url()
        .await;
        self.persist_storage(&page, &url).await;
        Ok(())
    }

    /// Loads a previously saved storage state and applies it to the
    /// session (explicit load; blueprint §7.4).
    pub async fn load_storage(&self) -> Result<(), SessionError> {
        let state = match &self.state_path {
            Some(path) => StorageState::read(path),
            None => {
                return Err(SessionError::Action(ActionError::Internal {
                    detail: "this session has no storage state directory".to_owned(),
                }));
            }
        };
        let context = self.context.read().await.clone();
        let pairs = self
            .lock_pages()
            .iter()
            .map(|slot| (slot.url.clone(), Arc::clone(&slot.handle)))
            .collect::<Vec<_>>();
        state.restore(context.as_ref(), &pairs).await;
        *self.lock_last_storage() = state;
        Ok(())
    }

    /// Rebuilds the session on a fresh context after an engine restart:
    /// replays cookies and localStorage, re-opens the tracked pages at
    /// their URLs, and publishes `EngineRestarted` (blueprint §7.4).
    pub(crate) async fn recover(&self, context: Arc<dyn ContextHandle>) {
        let saved = std::mem::take(&mut *self.lock_pages());
        let state = self.lock_last_storage().clone();

        let shared = Arc::clone(&context);
        *self.context.write().await = shared;
        if !state.cookies.is_empty() {
            let _ = context.set_cookies(&state.cookies).await;
        }

        // Track the active slot by position: two tabs can share a URL,
        // and both must not come back active (one active page is the
        // invariant every caller relies on).
        let active_index = saved.iter().position(|slot| slot.active);
        let mut restored = Vec::new();
        for (index, slot) in saved.into_iter().enumerate() {
            if slot.url.is_empty() || slot.url == "about:blank" {
                continue;
            }
            let Ok((id, handle)) = context.open_page().await else {
                continue;
            };
            let _ = handle.navigate(&slot.url).await;
            let origin = PageOps {
                page: handle.as_ref(),
                config: &self.config,
            }
            .url()
            .await;
            state
                .restore(context.as_ref(), &[(origin, Arc::clone(&handle))])
                .await;
            let was_active = active_index == Some(index);
            restored.push(PageSlot {
                id: id.clone(),
                url: slot.url,
                handle,
                active: was_active,
            });
            self.backbone
                .publish(self.id.clone(), Event::PageOpened { page: id });
        }
        if !restored.is_empty() && !restored.iter().any(|slot| slot.active) {
            restored[0].active = true;
        }
        *self.lock_pages() = restored;

        self.backbone
            .publish(self.id.clone(), Event::EngineRestarted);
    }

    /// Captures and persists the storage state; the in-memory copy
    /// updates first so recovery works even if the file write fails.
    async fn persist_storage(&self, page: &Arc<dyn PageHandle>, url: &str) {
        let context = self.context.read().await.clone();
        let state =
            StorageState::capture(context.as_ref(), &[(url.to_owned(), Arc::clone(page))]).await;
        *self.lock_last_storage() = state.clone();
        if let Some(path) = &self.state_path {
            if let Err(error) = state.write(path) {
                eprintln!("rutter: cannot write storage state: {error}");
            }
        }
    }

    /// Closes every page of the session; page-close failures are
    /// tolerated so a session always closes.
    pub async fn close(&self) {
        let context = self.context.read().await.clone();
        for page_id in context.pages() {
            let _ = context.close_page(page_id).await;
        }
        self.lock_pages().clear();
    }

    fn lock_pages(&self) -> std::sync::MutexGuard<'_, Vec<PageSlot>> {
        self.pages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_last_storage(&self) -> std::sync::MutexGuard<'_, StorageState> {
        self.last_storage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The reference an approval denial names; actions without a target
/// element use an empty reference.
fn action_reference(action: &Action) -> rutter_core::reference::Reference {
    match action {
        Action::Click { reference }
        | Action::Hover { reference }
        | Action::Type { reference, .. }
        | Action::SelectOption { reference, .. } => reference.clone(),
        Action::Scroll {
            reference: Some(reference),
            ..
        } => reference.clone(),
        _ => rutter_core::reference::Reference::new(""),
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
