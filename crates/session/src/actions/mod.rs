//! The action executor: three-phase auto-wait, act, settle, snapshot.
//!
//! Boundary: one action per call, executed against one page handle.
//! Reference actions auto-wait through the page resolver; every
//! mutating action returns a fresh snapshot. Policy verdicts and
//! approval are decided by the session layer and never run here.

use std::time::{Duration, Instant};

use rutter_core::action::{Action, ScrollDirection};
use rutter_core::error::{ActionError, WaitPhase};
use rutter_core::ids::{PageId, SessionId};
use rutter_core::readout::Readout;
use rutter_core::reference::Reference;
use rutter_core::snapshot::Snapshot;
use rutter_engine::error::EngineError;
use rutter_events::Backbone;
use rutter_observe::{
    files_check_script, focus_script, reader_script, select_script, serializer_script,
    wait_for_script,
};

use crate::config::SessionConfig;
use crate::error::SessionError;
use crate::resolve::{self, ElementBox};
use crate::wait::poll_until;

/// Executes single actions against one page of one session.
pub(crate) struct Executor<'a> {
    pub session: &'a SessionId,
    pub page_id: &'a PageId,
    pub page: &'a dyn rutter_engine::page::PageHandle,
    pub config: &'a SessionConfig,
    pub backbone: &'a Backbone,
}

impl Executor<'_> {
    /// Runs one action to completion, returning the fresh snapshot.
    pub async fn run(&self, action: &Action) -> Result<Snapshot, SessionError> {
        match action {
            Action::Navigate { url } => self.run_navigate(url).await,
            Action::Back => {
                self.page.go_back().await.map_err(SessionError::Engine)?;
                self.settle_snapshot().await
            }
            Action::Forward => {
                self.page.go_forward().await.map_err(SessionError::Engine)?;
                self.settle_snapshot().await
            }
            Action::Reload => {
                self.page.reload().await.map_err(SessionError::Engine)?;
                self.settle_snapshot().await
            }
            Action::Click { reference } => {
                let element_box = self.auto_wait(reference).await?;
                let (x, y) = element_box.center();
                self.dispatch_press(x, y).await?;
                self.settle_snapshot().await
            }
            Action::Hover { reference } => {
                let element_box = self.auto_wait(reference).await?;
                let (x, y) = element_box.center();
                self.dispatch(rutter_engine::input::InputEvent::MouseMove { x, y })
                    .await?;
                self.settle_snapshot().await
            }
            Action::Type { reference, text } => self.run_type(reference, text).await,
            Action::PressKey { key } => self.run_press_key(key).await,
            Action::SelectOption { reference, values } => {
                self.run_select_option(reference, values).await
            }
            Action::Scroll {
                reference,
                direction,
                amount,
            } => {
                self.run_scroll(reference.as_ref(), *direction, *amount)
                    .await
            }
            Action::SetInputFiles { reference, paths } => {
                self.run_set_input_files(reference, paths).await
            }
        }
    }

    /// Navigates the page, publishes where it landed, and answers its
    /// snapshot.
    ///
    /// The one action that does not settle: `navigate` already returned
    /// the loaded page, so the pause before the snapshot would only
    /// delay the answer the caller is already waiting on. Every other
    /// action mutates a page that keeps moving afterwards and settles.
    async fn run_navigate(&self, url: &str) -> Result<Snapshot, SessionError> {
        let effective = self
            .page
            .navigate(url)
            .await
            .map_err(SessionError::Engine)?;
        self.backbone.publish(
            self.session.clone(),
            rutter_events::Event::PageNavigated {
                page: self.page_id.clone(),
                url: effective,
            },
        );
        self.page_ops().snapshot().await
    }

    /// Focuses the referenced element and inserts the text into it.
    async fn run_type(&self, reference: &Reference, text: &str) -> Result<Snapshot, SessionError> {
        self.auto_wait(reference).await?;
        let focused = self.evaluate(&focus_script(reference.as_str())).await?;
        if focused.get("missing") == Some(&serde_json::Value::Bool(true)) {
            return Err(expired(reference.as_str()));
        }
        self.dispatch(rutter_engine::input::InputEvent::InsertText {
            text: text.to_owned(),
        })
        .await?;
        self.settle_snapshot().await
    }

    /// Sends one key press and its release.
    async fn run_press_key(&self, key: &str) -> Result<Snapshot, SessionError> {
        for event in [
            rutter_engine::input::InputEvent::KeyPressed {
                key: key.to_owned(),
            },
            rutter_engine::input::InputEvent::KeyReleased {
                key: key.to_owned(),
            },
        ] {
            self.dispatch(event).await?;
        }
        self.settle_snapshot().await
    }

    /// Selects option values on a select element.
    ///
    /// Three refusals live here because only the page can tell them
    /// apart, and each is the caller's to fix rather than an engine
    /// failure: the reference vanished, the element is not a select, or
    /// no option carried the requested values.
    async fn run_select_option(
        &self,
        reference: &Reference,
        values: &[String],
    ) -> Result<Snapshot, SessionError> {
        self.auto_wait(reference).await?;
        let answer = self
            .evaluate(&select_script(reference.as_str(), values))
            .await?;
        if answer.get("missing") == Some(&serde_json::Value::Bool(true)) {
            return Err(expired(reference.as_str()));
        }
        if answer.get("not_select") == Some(&serde_json::Value::Bool(true)) {
            return Err(SessionError::Action(ActionError::NotInteractable {
                reference: reference.clone(),
                reason: "the element is not a select".to_owned(),
            }));
        }
        let matched = answer
            .get("matched")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        if matched == 0 {
            return Err(SessionError::Action(ActionError::NotInteractable {
                reference: reference.clone(),
                reason: "no option matched the requested values".to_owned(),
            }));
        }
        self.settle_snapshot().await
    }

    /// Scrolls the referenced container, or the viewport when the
    /// action names no reference.
    async fn run_scroll(
        &self,
        reference: Option<&Reference>,
        direction: ScrollDirection,
        amount: u32,
    ) -> Result<Snapshot, SessionError> {
        let (x, y) = match reference {
            Some(reference) => {
                let element_box = self.auto_wait(reference).await?;
                element_box.center()
            }
            None => self.viewport_center().await,
        };
        let (delta_x, delta_y) = wheel_deltas(direction, amount);
        self.dispatch(rutter_engine::input::InputEvent::MouseWheel {
            x,
            y,
            delta_x,
            delta_y,
        })
        .await?;
        self.settle_snapshot().await
    }

    /// Sets the files of a file input element.
    ///
    /// Every refusal below is caller feedback, not an engine failure:
    /// no paths, a reference that vanished, an element that is not a
    /// file input, several paths on a single-file input, or a path that
    /// does not exist on the machine the engine runs on.
    async fn run_set_input_files(
        &self,
        reference: &Reference,
        paths: &[String],
    ) -> Result<Snapshot, SessionError> {
        if paths.is_empty() {
            return Err(SessionError::Action(ActionError::NotInteractable {
                reference: reference.clone(),
                reason: "no files were given".to_owned(),
            }));
        }
        self.auto_wait(reference).await?;
        let check = self
            .evaluate(&files_check_script(reference.as_str()))
            .await?;
        if check.get("missing") == Some(&serde_json::Value::Bool(true)) {
            return Err(expired(reference.as_str()));
        }
        if check.get("not_file") == Some(&serde_json::Value::Bool(true)) {
            return Err(SessionError::Action(ActionError::NotInteractable {
                reference: reference.clone(),
                reason: "the element is not a file input".to_owned(),
            }));
        }
        let multiple = check
            .get("multiple")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if !multiple && paths.len() > 1 {
            return Err(SessionError::Action(ActionError::NotInteractable {
                reference: reference.clone(),
                reason: "the file input accepts a single file".to_owned(),
            }));
        }
        // The engine resolves paths on this machine; a path that does
        // not exist is caller feedback, not an engine failure.
        for path in paths {
            let exists = std::path::Path::new(path).try_exists().unwrap_or(false);
            if !exists {
                return Err(SessionError::Action(ActionError::NotInteractable {
                    reference: reference.clone(),
                    reason: format!("file not found: {path}"),
                }));
            }
        }
        self.page
            .set_input_files(reference.as_str(), paths)
            .await
            .map_err(|error| match error {
                // The element vanished between the check and the file
                // handoff: same answer as above.
                EngineError::ReferenceExpired { reference } => {
                    SessionError::Action(ActionError::ReferenceExpired {
                        reference: Reference::new(&reference),
                    })
                }
                other => SessionError::Engine(other),
            })?;
        self.settle_snapshot().await
    }

    /// Runs the three-phase auto-wait: visible, then stable, then
    /// enabled. A gone reference fails fast; each phase gets its own
    /// budget and exhaustion maps to `TimedOut` naming the phase that
    /// was pending. The budget restarts only when the
    /// wait advances to a later phase, and at most
    /// [`MAX_PHASE_RESTARTS`] times — the advances a monotonic
    /// progression can make — so a flickering page cannot stretch the
    /// wait without end: the total stays bounded by
    /// `MAX_PHASE_RESTARTS + 1` budgets.
    async fn auto_wait(&self, reference: &Reference) -> Result<ElementBox, SessionError> {
        let reference = reference.as_str();
        let mut deadline = Instant::now() + self.config.phase_timeout;
        let mut pending = WaitPhase::Visible;
        let mut previous: Option<ElementBox> = None;
        let mut restarts = 0u32;
        loop {
            match resolve::element_box(self.page, reference).await {
                Ok(None) => return Err(expired(reference)),
                Ok(Some(element_box)) => {
                    let visible = element_box.visible();
                    let stable = previous
                        .as_ref()
                        .is_some_and(|prev| element_box.stable_against(prev));
                    if visible && stable && !element_box.disabled {
                        return Ok(element_box);
                    }
                    previous = if visible { Some(element_box) } else { None };
                    // The phases run in order, so the first unsatisfied
                    // one is the phase the wait is stuck in.
                    let now_pending = if !visible {
                        WaitPhase::Visible
                    } else if !stable {
                        WaitPhase::Stable
                    } else {
                        WaitPhase::Enabled
                    };
                    if now_pending != pending {
                        if phase_rank(now_pending) > phase_rank(pending)
                            && restarts < MAX_PHASE_RESTARTS
                        {
                            deadline = Instant::now() + self.config.phase_timeout;
                            restarts += 1;
                        }
                        pending = now_pending;
                    }
                }
                // A page mid-navigation can reject evaluates; keep trying
                // within the phase budget.
                Err(error) => {
                    if Instant::now() >= deadline {
                        return Err(SessionError::Engine(error));
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(SessionError::Action(ActionError::TimedOut {
                    phase: pending,
                    elapsed: self.config.phase_timeout,
                }));
            }
            // The two stability samples sit one interval apart, so the
            // loop cadence is the stability interval.
            tokio::time::sleep(self.config.stability_sample_interval).await;
        }
    }

    /// One engine call whose only question is whether it succeeded.
    /// Every action in [`Executor::run`] reaches the page through one
    /// of the two helpers below, so the engine-to-session error mapping
    /// is written once instead of once per arm.
    async fn dispatch(&self, event: rutter_engine::input::InputEvent) -> Result<(), SessionError> {
        self.page
            .dispatch_input(event)
            .await
            .map_err(SessionError::Engine)
    }

    async fn evaluate(&self, script: &str) -> Result<serde_json::Value, SessionError> {
        self.page
            .evaluate(script)
            .await
            .map_err(SessionError::Engine)
    }

    async fn dispatch_press(&self, x: f64, y: f64) -> Result<(), SessionError> {
        use rutter_engine::input::{InputEvent, MouseButton};
        for event in [
            InputEvent::MousePressed {
                x,
                y,
                button: MouseButton::Left,
            },
            InputEvent::MouseReleased {
                x,
                y,
                button: MouseButton::Left,
            },
        ] {
            self.dispatch(event).await?;
        }
        Ok(())
    }

    async fn viewport_center(&self) -> (f64, f64) {
        // The exact center only steers the wheel; a degraded answer from
        // a hostile page costs nothing.
        match self
            .page
            .evaluate("({ x: window.innerWidth / 2, y: window.innerHeight / 2 })")
            .await
        {
            Ok(answer) => match (answer.get("x"), answer.get("y")) {
                (Some(x), Some(y)) => (x.as_f64().unwrap_or(400.0), y.as_f64().unwrap_or(300.0)),
                _ => (400.0, 300.0),
            },
            Err(_) => (400.0, 300.0),
        }
    }

    /// Settles for the configured pause (lets same-tick navigations
    /// start), then takes the fresh snapshot.
    async fn settle_snapshot(&self) -> Result<Snapshot, SessionError> {
        if !self.config.settle.is_zero() {
            tokio::time::sleep(self.config.settle).await;
        }
        self.page_ops().snapshot().await
    }

    fn page_ops(&self) -> PageOps<'_> {
        PageOps {
            page: self.page,
            config: self.config,
        }
    }
}

/// Snapshot and wait operations over one page, shared by the executor
/// and the session's read-only tools.
pub(crate) struct PageOps<'a> {
    pub page: &'a dyn rutter_engine::page::PageHandle,
    pub config: &'a SessionConfig,
}

impl PageOps<'_> {
    /// Reads the page's current URL; `None` when the page does not
    /// answer (mid-navigation or dead). Policy judgments fail closed on
    /// `None`; bookkeeping keeps the last known value instead.
    pub async fn url(&self) -> Option<String> {
        self.page
            .evaluate("location.href")
            .await
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
    }

    /// Renders the current page as a snapshot.
    pub async fn snapshot(&self) -> Result<Snapshot, SessionError> {
        // Display only: a page that will not answer href keeps the
        // familiar placeholder rather than failing the snapshot.
        let url = self.url().await.unwrap_or_else(|| "about:blank".to_owned());
        let raw = self
            .page
            .evaluate(serializer_script())
            .await
            .map_err(SessionError::Engine)?;
        Ok(rutter_observe::snapshot_from_response(&url, &raw))
    }

    /// Extracts the current page's readable content as a readout.
    pub async fn read(&self) -> Result<Readout, SessionError> {
        let raw = self
            .page
            .evaluate(reader_script())
            .await
            .map_err(SessionError::Engine)?;
        Ok(rutter_observe::read_from_response(&raw))
    }

    /// Polls the page text until `needle` appears within `budget`.
    pub async fn wait_for(&self, needle: &str, budget: Duration) -> Result<Snapshot, SessionError> {
        // Clamp here so the timeout error reports the effective budget,
        // not a raw request the poll layer would have clamped anyway.
        let budget = budget.min(crate::wait::MAX_POLL_BUDGET);
        let text_json = serde_json::to_string(needle).map_err(|error| {
            SessionError::Action(ActionError::Internal {
                detail: format!("needle is not representable as JSON: {error}"),
            })
        })?;
        let script = wait_for_script(&text_json);
        let found = poll_until(
            || async {
                self.page
                    .evaluate(&script)
                    .await
                    .ok()
                    .and_then(|answer| answer.get("found").and_then(serde_json::Value::as_bool))
                    .filter(|found| *found)
                    .map(|_| ())
            },
            budget,
            self.config.poll_interval,
        )
        .await;
        match found {
            Some(()) => self.snapshot().await,
            None => Err(SessionError::Action(ActionError::TimedOut {
                phase: WaitPhase::Poll,
                elapsed: budget,
            })),
        }
    }
}

/// Order of the auto-wait phases for budget restarts: visible, then
/// stable, then enabled. Other phases rank above and never restart.
fn phase_rank(phase: WaitPhase) -> u8 {
    match phase {
        WaitPhase::Visible => 0,
        WaitPhase::Stable => 1,
        WaitPhase::Enabled => 2,
        WaitPhase::Act | WaitPhase::Settle | WaitPhase::Poll => 3,
    }
}

/// How many times one auto-wait may restart its phase budget. A
/// monotonic progression (visible → stable → enabled) advances at most
/// twice, so a genuinely progressing element never notices the cap —
/// while a flickering one (an `aria-disabled` flag toggling, say)
/// cannot push the deadline out again on every cycle.
const MAX_PHASE_RESTARTS: u32 = 2;

/// Wheel deltas for a direction; the engine dispatches one wheel event.
fn wheel_deltas(direction: ScrollDirection, amount: u32) -> (f64, f64) {
    let amount = f64::from(amount);
    match direction {
        ScrollDirection::Up => (0.0, -amount),
        ScrollDirection::Down => (0.0, amount),
        ScrollDirection::Left => (-amount, 0.0),
        ScrollDirection::Right => (amount, 0.0),
    }
}

fn expired(reference: &str) -> SessionError {
    SessionError::Action(ActionError::ReferenceExpired {
        reference: Reference::new(reference),
    })
}

#[cfg(test)]
mod tests;
