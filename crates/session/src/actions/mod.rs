//! The action executor: three-phase auto-wait, act, settle, snapshot.
//!
//! Boundary: one action per call, executed against one page handle.
//! Reference actions auto-wait through the page resolver
//! (docs/tool-catalog.md §3); every mutating action returns a fresh
//! snapshot (docs/tool-catalog.md §2). Policy verdicts and approval are
//! decided by the session layer and never run here.

use std::time::{Duration, Instant};

use rutter_core::action::{Action, ScrollDirection};
use rutter_core::error::{ActionError, WaitPhase};
use rutter_core::ids::{PageId, SessionId};
use rutter_core::readout::Readout;
use rutter_core::reference::Reference;
use rutter_core::snapshot::Snapshot;
use rutter_events::Backbone;
use rutter_observe::{
    focus_script, reader_script, select_script, serializer_script, wait_for_script,
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
            Action::Navigate { url } => {
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
                self.page
                    .dispatch_input(rutter_engine::input::InputEvent::MouseMove { x, y })
                    .await
                    .map_err(SessionError::Engine)?;
                self.settle_snapshot().await
            }
            Action::Type { reference, text } => {
                self.auto_wait(reference).await?;
                let focused = self
                    .page
                    .evaluate(&focus_script(reference.as_str()))
                    .await
                    .map_err(SessionError::Engine)?;
                if focused.get("missing") == Some(&serde_json::Value::Bool(true)) {
                    return Err(expired(reference.as_str()));
                }
                self.page
                    .dispatch_input(rutter_engine::input::InputEvent::InsertText {
                        text: text.clone(),
                    })
                    .await
                    .map_err(SessionError::Engine)?;
                self.settle_snapshot().await
            }
            Action::PressKey { key } => {
                for event in [
                    rutter_engine::input::InputEvent::KeyPressed { key: key.clone() },
                    rutter_engine::input::InputEvent::KeyReleased { key: key.clone() },
                ] {
                    self.page
                        .dispatch_input(event)
                        .await
                        .map_err(SessionError::Engine)?;
                }
                self.settle_snapshot().await
            }
            Action::SelectOption { reference, values } => {
                self.auto_wait(reference).await?;
                let values_json = serde_json::to_string(values).map_err(|error| {
                    SessionError::Action(ActionError::Internal {
                        detail: format!("values are not representable as JSON: {error}"),
                    })
                })?;
                let answer = self
                    .page
                    .evaluate(&select_script(reference.as_str(), &values_json))
                    .await
                    .map_err(SessionError::Engine)?;
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
            Action::Scroll {
                reference,
                direction,
                amount,
            } => {
                let (x, y) = match reference {
                    Some(reference) => {
                        let element_box = self.auto_wait(reference).await?;
                        element_box.center()
                    }
                    None => self.viewport_center().await,
                };
                let (delta_x, delta_y) = wheel_deltas(*direction, *amount);
                self.page
                    .dispatch_input(rutter_engine::input::InputEvent::MouseWheel {
                        x,
                        y,
                        delta_x,
                        delta_y,
                    })
                    .await
                    .map_err(SessionError::Engine)?;
                self.settle_snapshot().await
            }
        }
    }

    /// Runs the three-phase auto-wait: visible, then stable, then
    /// enabled. A gone reference fails fast; each phase gets its own
    /// budget and exhaustion maps to `TimedOut` naming the phase that
    /// was pending (docs/tool-catalog.md §3). The budget restarts only when the
    /// wait advances to a later phase, so a flickering page cannot
    /// stretch the wait indefinitely.
    async fn auto_wait(&self, reference: &Reference) -> Result<ElementBox, SessionError> {
        let reference = reference.as_str();
        let mut deadline = Instant::now() + self.config.phase_timeout;
        let mut pending = WaitPhase::Visible;
        let mut previous: Option<ElementBox> = None;
        loop {
            match resolve::resolve(self.page, reference).await {
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
                        if phase_rank(now_pending) > phase_rank(pending) {
                            deadline = Instant::now() + self.config.phase_timeout;
                        }
                        pending = now_pending;
                    }
                }
                // A page mid-navigation can reject evaluates; keep trying
                // within the phase budget (docs/tool-catalog.md §3).
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
            // loop cadence is the stability interval (docs/tool-catalog.md §3).
            tokio::time::sleep(self.config.stability_sample_interval).await;
        }
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
            self.page
                .dispatch_input(event)
                .await
                .map_err(SessionError::Engine)?;
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

    /// Settles for the configured pause (docs/tool-catalog.md §3: lets same-tick
    /// navigations start), then takes the fresh snapshot.
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
                phase: WaitPhase::Settle,
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
        WaitPhase::Act | WaitPhase::Settle => 3,
    }
}

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
