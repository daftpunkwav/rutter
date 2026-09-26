//! One session: a client's context, its pages, and its events.
//!
//! Boundary: the orchestration surface the MCP tools call. The session
//! owns exactly one context (docs/architecture.md), tracks its open pages and
//! their URLs, executes actions through the [`crate::actions`]
//! executor, enforces the policy with approvals (docs/policy.md), and persists
//! its storage state after every change so a supervisor restart can
//! rebuild it (docs/sessions.md). Recovery swaps in a fresh context and replays.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rutter_core::action::{Action, Origin};
use rutter_core::cookie::Cookie;
use rutter_core::error::ActionError;
use rutter_core::ids::{PageId, SessionId};
use rutter_core::readout::Readout;
use rutter_core::snapshot::Snapshot;
use rutter_engine::context::ContextHandle;
use rutter_engine::error::EngineError;
use rutter_engine::page::{PageHandle, ScreencastStream, Screenshot};
use rutter_events::{Backbone, Event};
use rutter_policy::{
    ApprovalBrief, ApprovalBroker, ApprovalEffect, ApprovalOutcome, Review, RuleSet,
};

use crate::actions::{Executor, PageOps};
use crate::audit::{self, ApprovalAudit};
use crate::config::SessionConfig;
use crate::error::SessionError;
use crate::pages::{PageInfo, PageRegistry, PageSlot};
use crate::storage::StorageState;

/// The client-visible state of one session.
pub struct Session {
    id: SessionId,
    backbone: Arc<Backbone>,
    config: SessionConfig,
    policy: Arc<RuleSet>,
    broker: Arc<ApprovalBroker>,
    /// Swappable so recovery can install a fresh context after an
    /// engine restart (docs/sessions.md).
    context: tokio::sync::RwLock<Arc<dyn ContextHandle>>,
    /// The tracked pages and their active flag, behind the registry's own
    /// lock (docs/sessions.md).
    pages: PageRegistry,
    /// Where the storage state persists; `None` disables persistence.
    state_path: Option<PathBuf>,
    /// The approval audit trail; a session with no state directory writes
    /// nothing (docs/policy.md).
    audit: ApprovalAudit,
    last_storage: Mutex<StorageState>,
    /// Whether the persistence file currently reflects `last_storage`;
    /// a failed write forces a rewrite on the next capture even when
    /// the captured state is unchanged.
    last_persist_ok: Mutex<bool>,
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
            pages: PageRegistry::new(),
            audit: ApprovalAudit::beside_storage(state_path.as_deref()),
            state_path,
            last_storage: Mutex::new(StorageState::default()),
            last_persist_ok: Mutex::new(false),
        }
    }

    /// The session's identifier.
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// The event backbone, for consumers that replay session history
    /// (the manager's recovery task, the dashboard).
    pub fn backbone(&self) -> Arc<Backbone> {
        Arc::clone(&self.backbone)
    }

    /// The approval broker decisions are submitted to.
    pub fn broker(&self) -> Arc<ApprovalBroker> {
        Arc::clone(&self.broker)
    }

    /// Reviews one operation against the rule set and, when the answer is
    /// to ask a human, publishes the request and parks until someone
    /// answers or the window closes (docs/policy.md).
    ///
    /// Policy owns the whole judgment: `raw_url` is canonicalized inside
    /// [`RuleSet::review`], so no caller can reach a verdict while skipping
    /// that step, and a URL that will not canonicalize — not a URL, or one
    /// hiding credentials behind a `user@host` decoy — fails closed.
    ///
    /// What the human is shown is the brief policy built, never a stand-in
    /// for it. Cookie writes had no `Action` variant to carry, so they used
    /// to ride on `reload` and a person approving a reload was authorizing
    /// something else.
    async fn enforce(
        &self,
        effect: ApprovalEffect,
        raw_url: Option<&str>,
        page_id: &PageId,
    ) -> Result<(), SessionError> {
        match self.policy.review(effect.clone(), raw_url) {
            Review::Allowed => Ok(()),
            Review::Denied => Err(approval_denied(&effect)),
            Review::NeedsApproval(brief) => self.park(brief, &effect, page_id).await,
        }
    }

    /// Publishes one parked request and waits for the human behind it:
    /// granted proceeds, denied and timed out fail with the agent-facing
    /// error, and either way the resolution lands on the backbone so the
    /// dashboard can retire the card.
    async fn park(
        &self,
        brief: Box<ApprovalBrief>,
        effect: &ApprovalEffect,
        page_id: &PageId,
    ) -> Result<(), SessionError> {
        let (request_id, receiver) = self.broker.open();
        let mut ticket = ParkTicket {
            session: self,
            request_id: request_id.as_str().to_owned(),
            page_id: page_id.clone(),
            brief,
            started: Instant::now(),
            outcome: None,
        };
        self.backbone.publish(
            self.id.clone(),
            Event::ApprovalRequested {
                request_id: ticket.request_id.clone(),
                page: page_id.clone(),
                brief: Box::clone(&ticket.brief),
            },
        );
        ticket.settle(
            self.broker
                .wait(&request_id, receiver, self.policy.approval_timeout())
                .await,
        );
        match ticket.outcome {
            Some(ApprovalOutcome::Granted) => Ok(()),
            Some(ApprovalOutcome::Denied) => Err(approval_denied(effect)),
            // No broker answer means the window closed, or the sender went
            // away with the manager; either way the agent sees a timeout.
            _ => Err(SessionError::Action(ActionError::ApprovalTimedOut {
                waited: self.policy.approval_timeout(),
            })),
        }
    }

    /// The page an operation runs on, opening the session's first page
    /// when it has none. The id and the handle come back together so a
    /// caller cannot observe the two at different moments.
    ///
    /// While recovery owns the list this fails with `Terminated`: the
    /// engine was just replaced and is being rebuilt, and opening a page
    /// into a list that is about to be written back would leave a live tab
    /// no one tracks (docs/sessions.md).
    async fn ensure_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), SessionError> {
        if let Some(found) = self.pages.active() {
            return Ok(found);
        }
        if !self.pages.accepts_new_page() {
            return Err(SessionError::Engine(EngineError::Terminated));
        }
        let context = self.context.read().await.clone();
        let (page_id, handle) = context.open_page().await.map_err(engine_error)?;
        if self.pages.admit_active(PageSlot {
            id: page_id.clone(),
            url: String::new(),
            handle: Arc::clone(&handle),
            active: true,
        }) {
            self.backbone.publish(
                self.id.clone(),
                Event::PageOpened {
                    page: page_id.clone(),
                },
            );
            return Ok((page_id, handle));
        }
        // A concurrent caller won the race, or recovery took the list over
        // while this page was opening. The loser closes what it opened: it
        // is untracked, and nothing else will close it.
        let _ = context.close_page(page_id).await;
        self.pages
            .active()
            .ok_or(SessionError::Engine(EngineError::Terminated))
    }

    /// Executes one action with the given origin and returns the fresh
    /// snapshot; requested/completed/failed land on the event backbone.
    /// After every attempt, successful or not, the tracked page URL and
    /// the storage state are refreshed (docs/sessions.md: persist on
    /// change).
    pub async fn execute(&self, action: Action, origin: Origin) -> Result<Snapshot, SessionError> {
        let (page_id, page) = self.ensure_page().await?;

        // Supervision gate (docs/policy.md): agent-origin actions are
        // judged at the URL the action leads to — a navigation at its
        // target, everything else at the page it acts on. Human-origin
        // actions bypass approval and are recorded identically.
        let ops = PageOps {
            page: page.as_ref(),
            config: &self.config,
        };
        if origin == Origin::Agent {
            let policy_url = match &action {
                Action::Navigate { url } => Some(url.clone()),
                _ => ops.url().await,
            };
            self.enforce(
                ApprovalEffect::Action {
                    action: action.clone(),
                },
                policy_url.as_deref(),
                &page_id,
            )
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

        // Refresh the tracked URL of the page the action ran on, then
        // persist the changed state (docs/sessions.md: persist per session
        // on change). Filtering by id, not by the current active flag: a
        // concurrent `select_page` may have switched the active slot
        // while this action was in flight, and the new page's URL must
        // not be overwritten with this page's.
        let url = match ops.url().await {
            Some(url) => {
                self.pages.set_url(&page_id, &url);
                url
            }
            // The page would not answer: keep the last known URL
            // instead of overwriting tracking with a placeholder.
            None => self.last_known_url(&page_id),
        };
        self.persist_storage(&page, &url).await;

        result
    }

    /// Renders the active page as a snapshot; no events, no auto-wait,
    /// no URL refresh.
    pub async fn snapshot(&self) -> Result<Snapshot, SessionError> {
        let (_, page) = self.ensure_page().await?;
        PageOps {
            page: page.as_ref(),
            config: &self.config,
        }
        .snapshot()
        .await
    }

    /// Extracts the active page's readable content as a markdown
    /// readout; like [`Session::snapshot`], observation only — no
    /// events, no auto-wait, no URL refresh.
    pub async fn read(&self) -> Result<Readout, SessionError> {
        let (_, page) = self.ensure_page().await?;
        PageOps {
            page: page.as_ref(),
            config: &self.config,
        }
        .read()
        .await
    }

    /// Starts a live screencast of the active page; the stream is
    /// observation only — dropping it stops the capture (docs/dashboard.md:
    /// on-demand, dashboard never executes actions).
    ///
    /// This path reads the registry instead of going through
    /// `ensure_page` on purpose: a viewer asking to watch must not
    /// cause a tab to open, so a session with no page answers
    /// [`SessionError::NoOpenPage`] rather than mutating first.
    pub async fn screencast(&self) -> Result<ScreencastStream, SessionError> {
        let (_, page) = self.pages.active().ok_or(SessionError::NoOpenPage)?;
        page.start_screencast().await.map_err(SessionError::Engine)
    }

    /// Captures the active page. docs/tool-catalog.md §4: capture errors map onto
    /// `ActionError::Internal` so the agent sees the action taxonomy, not
    /// a raw engine failure.
    pub async fn screenshot(&self) -> Result<Screenshot, SessionError> {
        let (_, page) = self.ensure_page().await?;
        page.capture_screenshot().await.map_err(|error| {
            SessionError::Action(ActionError::Internal {
                detail: error.to_string(),
            })
        })
    }

    /// Polls the active page until `needle` appears in its text.
    pub async fn wait_for(&self, needle: &str, budget: Duration) -> Result<Snapshot, SessionError> {
        let (_, page) = self.ensure_page().await?;
        PageOps {
            page: page.as_ref(),
            config: &self.config,
        }
        .wait_for(needle, budget)
        .await
    }

    /// Lists the session's pages with their last known URLs. Before
    /// listing, the registry is reconciled with the engine: windows
    /// that appeared without rutter opening them (`window.open`,
    /// `target=_blank`, a human's window) are adopted so they show up
    /// here and become selectable, exactly the surfaces `tabs_select`
    /// and `tabs_close` name (docs/tool-catalog.md).
    pub async fn pages(&self) -> Vec<PageInfo> {
        self.sync_foreign_pages().await;
        self.pages.list()
    }

    /// Adopts every engine-reported page surface the registry does not
    /// track yet. Failures degrade to the tracked-only listing: an
    /// engine that cannot enumerate reports nothing, a surface that
    /// vanished between listing and adopting is skipped, and recovery
    /// owning the list blocks the whole pass.
    async fn sync_foreign_pages(&self) {
        if !self.pages.accepts_new_page() {
            return;
        }
        let context = self.context.read().await.clone();
        let Ok(foreign) = context.foreign_pages().await else {
            return;
        };
        for surface in foreign {
            if self.pages.contains(&surface.id) {
                continue;
            }
            let Ok((id, handle)) = context.adopt_page(&surface.id).await else {
                continue;
            };
            if self.pages.admit(PageSlot {
                id: id.clone(),
                url: surface.url,
                handle,
                active: false,
            }) {
                // A page exists that this session did not open: the
                // timeline says so, like any other open.
                self.backbone
                    .publish(self.id.clone(), Event::PageOpened { page: id });
            }
        }
    }

    /// Makes another page active and returns its snapshot. No event is
    /// published; the page switch is visible to consumers only through
    /// the next action's events.
    pub async fn select_page(&self, page_id: PageId) -> Result<Snapshot, SessionError> {
        let handle = self
            .pages
            .activate(&page_id)
            .ok_or_else(|| unknown_page(&page_id))?;
        PageOps {
            page: handle.as_ref(),
            config: &self.config,
        }
        .snapshot()
        .await
    }

    /// Closes a page; closing the active page promotes the first
    /// remaining page. An id the session does not track is a client
    /// error: closing it would otherwise report success and publish a
    /// `PageClosed` event for a page that never existed.
    pub async fn close_page(&self, page_id: PageId) -> Result<String, SessionError> {
        if !self.pages.contains(&page_id) {
            return Err(unknown_page(&page_id));
        }
        let context = self.context.read().await.clone();
        context
            .close_page(page_id.clone())
            .await
            .map_err(engine_error)?;
        // Only after the engine agreed: a page still open in the browser
        // must stay tracked, or it becomes an uncounted live tab.
        self.pages.remove(&page_id);
        self.backbone.publish(
            self.id.clone(),
            Event::PageClosed {
                page: page_id.clone(),
            },
        );
        Ok(format!("closed {page_id}"))
    }

    /// Sets cookies on the session's context, subject to the policy's
    /// `cookies` class rules (approval-required by default). The approval
    /// a human sees describes a cookie write: there is no `Action` variant
    /// for it, and an unrelated action standing in would authorize
    /// something other than what was shown (docs/policy.md).
    pub async fn set_cookies(&self, cookies: &[Cookie]) -> Result<(), SessionError> {
        let (page_id, page) = self.ensure_page().await?;
        let url = PageOps {
            page: page.as_ref(),
            config: &self.config,
        }
        .url()
        .await;
        self.enforce(
            ApprovalEffect::Cookies {
                count: cookies.len(),
            },
            url.as_deref(),
            &page_id,
        )
        .await?;
        let context = self.context.read().await.clone();
        context.set_cookies(cookies).await.map_err(engine_error)?;
        let url = url.unwrap_or_else(|| self.last_known_url(&page_id));
        self.persist_storage(&page, &url).await;
        Ok(())
    }

    /// Captures the session's storage state (cookies plus localStorage
    /// of every open page). A read-only probe: nothing is persisted and
    /// the in-memory `last_storage` copy is not touched, so a capture
    /// never influences what the next persist-on-change run writes.
    pub async fn capture_storage(&self) -> StorageState {
        let context = self.context.read().await.clone();
        StorageState::capture(context.as_ref(), &self.pages.pairs()).await
    }

    /// Saves the current storage state to the session's persistence
    /// file (explicit save; docs/sessions.md).
    pub async fn save_storage(&self) -> Result<(), SessionError> {
        let (page_id, page) = self.ensure_page().await?;
        let url = PageOps {
            page: page.as_ref(),
            config: &self.config,
        }
        .url()
        .await;
        let url = url.unwrap_or_else(|| self.last_known_url(&page_id));
        self.persist_storage(&page, &url).await;
        Ok(())
    }

    /// Loads a previously saved storage state and applies it to the
    /// session (explicit load; docs/sessions.md). Restores over the
    /// currently open pages and records the loaded state as the last
    /// known one, so a later persistence run does not rewrite it.
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
        let pairs = self.pages.pairs();
        state.restore(context.as_ref(), &pairs).await;
        *self.lock_last_storage() = state;
        Ok(())
    }

    /// Rebuilds the session on a fresh context after an engine restart:
    /// replays cookies and localStorage, re-opens the tracked pages at
    /// their URLs, and publishes `EngineRestarted` (docs/sessions.md).
    ///
    /// The registry is gated for the whole rebuild. Without the gate a
    /// concurrent action could open a page between the read-out and the
    /// write-back, get itself registered as active, and then be dropped by
    /// the write-back — leaving a live tab the new engine kept, `tabs_list`
    /// never showed, and the page cap never counted.
    pub(crate) async fn recover(&self, context: Arc<dyn ContextHandle>) {
        let saved = self.pages.begin_recovery();
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
            // The freshly opened page may not answer yet; the restored
            // target scopes the state in that case.
            let origin = PageOps {
                page: handle.as_ref(),
                config: &self.config,
            }
            .url()
            .await
            .unwrap_or_else(|| slot.url.clone());
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
        self.pages.finish_recovery(restored);

        self.backbone
            .publish(self.id.clone(), Event::EngineRestarted);
    }

    /// The tracked URL of `page_id`, for bookkeeping only: it survives
    /// an unreadable page instead of degrading tracking, and policy
    /// judgments never consume it — they fail closed on `None` instead.
    fn last_known_url(&self, page_id: &PageId) -> String {
        self.pages
            .url_of(page_id)
            .unwrap_or_else(|| "about:blank".to_owned())
    }

    /// Captures and persists the storage state; the in-memory copy
    /// updates first so recovery works even if the file write fails.
    /// The disk write is skipped when the captured state is identical to
    /// the last persisted one: "persist on change" (docs/sessions.md)
    /// needs no rewrite when nothing changed, and rewriting identical
    /// JSON on every action only burns file I/O.
    async fn persist_storage(&self, page: &Arc<dyn PageHandle>, url: &str) {
        let context = self.context.read().await.clone();
        let state =
            StorageState::capture(context.as_ref(), &[(url.to_owned(), Arc::clone(page))]).await;
        let (unchanged, persist_ok) = {
            let mut last = self.lock_last_storage();
            let unchanged = *last == state;
            *last = state.clone();
            (unchanged, *self.lock_last_persist_ok())
        };
        if unchanged && persist_ok {
            return;
        }
        if let Some(path) = &self.state_path {
            match state.write(path) {
                Ok(()) => *self.lock_last_persist_ok() = true,
                Err(error) => {
                    *self.lock_last_persist_ok() = false;
                    eprintln!("rutter: cannot write storage state: {error}");
                }
            }
        }
    }

    /// Closes the session's browser context (every page inside it goes
    /// with it); failures are tolerated so a session always closes. The
    /// registry is emptied after the context is gone, so no lookup can
    /// hand out a handle into a dead context.
    pub async fn close(&self) {
        let context = self.context.read().await.clone();
        let _ = context.close().await;
        self.pages.clear();
    }

    /// The page registry, for tests that seed tracked pages without going
    /// through an action.
    #[cfg(test)]
    pub(crate) fn registry(&self) -> &PageRegistry {
        &self.pages
    }

    /// Opens (or returns) the active page; exposed for tests that need
    /// to seed tracked pages without going through an action.
    #[cfg(test)]
    pub(crate) async fn active_page_for_test(&self) -> (PageId, Arc<dyn PageHandle>) {
        self.ensure_page().await.expect("test page opens")
    }

    pub(crate) fn lock_last_storage(&self) -> std::sync::MutexGuard<'_, StorageState> {
        self.last_storage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_last_persist_ok(&self) -> std::sync::MutexGuard<'_, bool> {
        self.last_persist_ok
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The reference an operation names in an agent-facing error; actions
/// without a target element use an empty reference.
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

/// One parked approval, from the question to its resolution.
///
/// The `Drop` half is the point. Publishing the resolution and writing the
/// audit line from ordinary control flow would skip both whenever this
/// future is cancelled — which is exactly what happens when the MCP client
/// that asked goes away. The dashboard would then keep a card nothing can
/// retire, and the trail would be silent about a supervision decision that
/// did end. Dropping the ticket settles it either way.
struct ParkTicket<'a> {
    session: &'a Session,
    request_id: String,
    page_id: PageId,
    brief: Box<ApprovalBrief>,
    started: Instant,
    outcome: Option<ApprovalOutcome>,
}

impl ParkTicket<'_> {
    /// Records the answer the broker delivered.
    fn settle(&mut self, outcome: ApprovalOutcome) {
        self.outcome = Some(outcome);
    }

    /// The audit label for whatever state the ticket ended in.
    fn label(&self) -> &'static str {
        match self.outcome {
            Some(ApprovalOutcome::Granted) => audit::GRANTED,
            Some(ApprovalOutcome::Denied) => audit::DENIED,
            Some(ApprovalOutcome::TimedOut) => audit::TIMED_OUT,
            // Cancelled mid-park: nobody granted it and nobody timed out
            // waiting either.
            None => audit::CANCELLED,
        }
    }
}

impl Drop for ParkTicket<'_> {
    fn drop(&mut self) {
        let granted = self.outcome == Some(ApprovalOutcome::Granted);
        self.session.backbone.publish(
            self.session.id.clone(),
            Event::ApprovalResolved {
                request_id: self.request_id.clone(),
                granted,
            },
        );
        self.session.audit.record(
            &self.session.id,
            &self.page_id,
            &self.request_id,
            &self.brief,
            self.label(),
            self.started.elapsed(),
        );
    }
}

/// The refusal an agent sees when policy denied its operation outright.
/// The reference names the element where one exists; an operation without
/// a target element keeps the empty reference the action path uses.
fn approval_denied(effect: &ApprovalEffect) -> SessionError {
    let reference = match effect {
        ApprovalEffect::Action { action } => action_reference(action),
        ApprovalEffect::Cookies { .. } => rutter_core::reference::Reference::new(""),
    };
    SessionError::Action(ActionError::ApprovalDenied { reference })
}

/// Maps engine failures onto the action taxonomy for event payloads.
pub(crate) fn as_action_error(session: &SessionId, error: &SessionError) -> ActionError {
    match error {
        SessionError::Action(action) => action.clone(),
        SessionError::Engine(engine) => match engine {
            EngineError::Terminated => ActionError::EngineTerminated {
                session: session.clone(),
            },
            EngineError::NavigationFailed { url, cause, .. } => ActionError::NavigationFailed {
                url: url.clone(),
                cause: cause.clone(),
            },
            EngineError::Timeout { elapsed, .. } => ActionError::TimedOut {
                phase: rutter_core::error::WaitPhase::Act,
                elapsed: *elapsed,
            },
            other => ActionError::Internal {
                detail: other.to_string(),
            },
        },
        // Manager-level failures never flow through action execution,
        // but the event payload needs a total mapping.
        SessionError::Capacity { detail } | SessionError::Internal { detail } => {
            ActionError::Internal {
                detail: detail.clone(),
            }
        }
        // The observation-only refusal: no action path reaches it, since
        // actions open a page when they need one.
        SessionError::NoOpenPage => ActionError::NotInteractable {
            reference: rutter_core::reference::Reference::new(""),
            reason: "the session has no open page to observe".to_owned(),
        },
    }
}

fn engine_error(error: EngineError) -> SessionError {
    SessionError::Engine(error)
}

fn unknown_page(page_id: &PageId) -> SessionError {
    SessionError::Action(ActionError::NotInteractable {
        reference: rutter_core::reference::Reference::new(page_id.as_str()),
        reason: format!("no open page with id '{page_id}'"),
    })
}

#[cfg(test)]
mod flow_tests;
#[cfg(test)]
mod tests;
