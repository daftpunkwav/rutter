//! CDP implementation of page-scoped operations.
//!
//! Boundary: one CDP target per handle. Every operation carries a
//! timeout (docs/architecture.md); navigation uses the context's configured
//! budget, the others a fixed bound so no call can wait forever.
//! Chromiumoxide types are implementation details and never appear in
//! the public `rutter-engine` trait signatures.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventType, DispatchMouseEventType, InsertTextParams,
};
use chromiumoxide::cdp::browser_protocol::page::{
    EventFrameNavigated, EventScreencastFrame, GetNavigationHistoryParams, NavigateParams,
    NavigateToHistoryEntryParams, ScreencastFrameAckParams, StartScreencastFormat,
    StartScreencastParams, StopScreencastParams,
};
use chromiumoxide::cdp::browser_protocol::target::TargetId;
use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use chromiumoxide::error::CdpError;
use chromiumoxide::page::ScreenshotParams;
use serde_json::Value;

use rutter_engine::error::EngineError;
use rutter_engine::input::InputEvent;
use rutter_engine::page::{ImageFormat, ScreencastFrame, ScreencastStream, Screenshot};

use crate::error::{self, COMMAND_TIMEOUT, fold, fold_navigation, with_deadline, with_deadline_by};

/// Budget for the page's URL query. The query is an untyped oneshot
/// into the chromiumoxide handler: if the handler died (browser crash
/// mid-navigation), the send fails, but a wedged handler would hang a
/// bare await forever, so it gets its own deadline like every call.
const URL_TIMEOUT: Duration = Duration::from_secs(10);

/// Screencast capture parameters, shared by the initial start and the
/// post-navigation restart: the two must stay identical so the viewer
/// sees one continuous stream across navigations.
fn screencast_params() -> StartScreencastParams {
    StartScreencastParams::builder()
        .format(StartScreencastFormat::Jpeg)
        .quality(50)
        .max_width(1024)
        .every_nth_frame(1)
        .build()
}

/// Resolves the instant the next capture may run and records it as the
/// new reservation, given the previous one. Pure so the spacing rules
/// are unit-testable without a page.
fn capture_due(last: Option<Instant>, min_interval: Duration, now: Instant) -> Instant {
    let wait = match last {
        // A future `at` is a slot a concurrent capture already reserved;
        // `elapsed()` saturates to zero there, which would shorten the
        // spacing, so this capture queues behind that slot instead.
        Some(at) if at > now => (at - now) + min_interval,
        // Both arms measure against the caller's `now`, keeping the
        // function deterministic (a second clock read inside would race
        // the first).
        Some(at) => min_interval.saturating_sub(now - at),
        None => Duration::ZERO,
    };
    now + wait
}

/// One CDP target wrapped as a page handle.
pub struct CdpPage {
    page: Page,
    navigation_timeout: Duration,
    /// Minimum interval between captures, from the context caps; `None`
    /// disables the rate limit.
    screenshot_min_interval: Option<Duration>,
    last_capture: Mutex<Option<Instant>>,
    /// Whether closing this page may close the underlying target.
    /// Pages attached to a target that already existed (the Electron
    /// engine's one visible surface, created by its own UI) outlive
    /// their handle: closing that target would take the browser's
    /// window content with it.
    closable: bool,
}

impl CdpPage {
    /// Creates a handle over a chromiumoxide page.
    pub fn new(
        page: Page,
        navigation_timeout: Duration,
        screenshot_min_interval: Option<Duration>,
    ) -> Self {
        Self {
            page,
            navigation_timeout,
            screenshot_min_interval,
            last_capture: Mutex::new(None),
            closable: true,
        }
    }

    /// Wraps a target that the browser owned before this handle asked
    /// for a page. The target belongs to the engine's own UI, so it
    /// survives the handle: `close_page` removes the registration only.
    pub fn attach(
        page: Page,
        navigation_timeout: Duration,
        screenshot_min_interval: Option<Duration>,
    ) -> Self {
        Self::adopted(page, navigation_timeout, screenshot_min_interval, false)
    }

    /// Wraps a target this context did not create. `closable` marks the
    /// rare case where the target still sits inside the adopting
    /// context — a `window.open` tab of the session's own isolation —
    /// so closing it is legitimate; every engine-owned surface (the app
    /// window, shells without context support) stays unclosable.
    pub fn adopted(
        page: Page,
        navigation_timeout: Duration,
        screenshot_min_interval: Option<Duration>,
        closable: bool,
    ) -> Self {
        Self {
            page,
            navigation_timeout,
            screenshot_min_interval,
            last_capture: Mutex::new(None),
            closable,
        }
    }

    /// Whether closing this page may close the underlying target.
    pub fn closable(&self) -> bool {
        self.closable
    }

    /// Target id for closing the tab through the browser connection
    /// (avoids consuming the page handle).
    pub fn target_id(&self) -> TargetId {
        self.page.target_id().clone()
    }

    /// Reads the page URL under a deadline; see [`URL_TIMEOUT`] for why
    /// this query cannot go bare.
    async fn url(&self) -> Result<Option<String>, EngineError> {
        with_deadline("page_url", URL_TIMEOUT, self.page.url()).await
    }

    /// Walks the session history by `offset` entries and resolves with
    /// the effective URL after the navigation settles.
    async fn navigate_history(&self, offset: i64) -> Result<String, EngineError> {
        with_deadline("history", self.navigation_timeout, async {
            let history = self
                .page
                .execute(GetNavigationHistoryParams::default())
                .await?;
            let target = history.result.current_index as i64 + offset;
            if target < 0 || target >= history.result.entries.len() as i64 {
                return Err(CdpError::ChromeMessage(format!(
                    "no history entry at offset {offset}"
                )));
            }
            let entry_id = history.result.entries[target as usize].id;
            self.page
                .execute(NavigateToHistoryEntryParams::new(entry_id))
                .await?;
            self.page.wait_for_navigation().await
        })
        .await?;

        self.url().await?.ok_or_else(|| EngineError::Internal {
            detail: "page URL unavailable after history navigation".to_owned(),
        })
    }
}

#[async_trait]
impl rutter_engine::page::PageHandle for CdpPage {
    async fn navigate(&self, url: &str) -> Result<String, EngineError> {
        with_deadline_by(
            "navigate",
            self.navigation_timeout,
            async {
                self.page.goto(NavigateParams::new(url)).await?;
                self.page.wait_for_navigation().await
            },
            |error| fold_navigation(error, url),
        )
        .await?;

        self.url().await?.ok_or_else(|| EngineError::Internal {
            detail: "page URL unavailable after navigation".to_owned(),
        })
    }

    async fn reload(&self) -> Result<(), EngineError> {
        with_deadline("reload", self.navigation_timeout, self.page.reload()).await?;
        Ok(())
    }

    async fn go_back(&self) -> Result<String, EngineError> {
        self.navigate_history(-1).await
    }

    async fn go_forward(&self) -> Result<String, EngineError> {
        self.navigate_history(1).await
    }

    async fn evaluate(&self, expression: &str) -> Result<Value, EngineError> {
        let mut params = EvaluateParams::new(expression);
        params.return_by_value = Some(true);
        let result = with_deadline(
            "evaluate",
            COMMAND_TIMEOUT,
            self.page.evaluate_expression(params),
        )
        .await?;
        Ok(result.value().cloned().unwrap_or(Value::Null))
    }

    async fn dispatch_input(&self, event: InputEvent) -> Result<(), EngineError> {
        let command = match event {
            InputEvent::MouseMove { x, y } => {
                error::mouse_params(DispatchMouseEventType::MouseMoved, x, y)
            }
            InputEvent::MousePressed { x, y, button } => {
                let mut params = error::mouse_params(DispatchMouseEventType::MousePressed, x, y);
                params.button = Some(error::cdp_button(button));
                params.click_count = Some(1);
                params
            }
            InputEvent::MouseReleased { x, y, button } => {
                let mut params = error::mouse_params(DispatchMouseEventType::MouseReleased, x, y);
                params.button = Some(error::cdp_button(button));
                params.click_count = Some(1);
                params
            }
            InputEvent::MouseWheel {
                x,
                y,
                delta_x,
                delta_y,
            } => {
                let mut params = error::mouse_params(DispatchMouseEventType::MouseWheel, x, y);
                params.delta_x = Some(delta_x);
                params.delta_y = Some(delta_y);
                params
            }
            InputEvent::InsertText { text } => {
                let params = InsertTextParams::new(text);
                return with_deadline("insert_text", COMMAND_TIMEOUT, self.page.execute(params))
                    .await
                    .map(|_| ());
            }
            InputEvent::KeyPressed { key } => {
                let params = error::key_params(DispatchKeyEventType::KeyDown, &key);
                return with_deadline("dispatch_key", COMMAND_TIMEOUT, self.page.execute(params))
                    .await
                    .map(|_| ());
            }
            InputEvent::KeyReleased { key } => {
                let params = error::key_params(DispatchKeyEventType::KeyUp, &key);
                return with_deadline("dispatch_key", COMMAND_TIMEOUT, self.page.execute(params))
                    .await
                    .map(|_| ());
            }
        };
        with_deadline(
            "dispatch_mouse",
            COMMAND_TIMEOUT,
            self.page.execute(command),
        )
        .await
        .map(|_| ())
    }

    async fn capture_screenshot(&self) -> Result<Screenshot, EngineError> {
        // Enforce the context's capture-rate cap so one caller cannot
        // flood the engine with captures.
        if let Some(min_interval) = self.screenshot_min_interval {
            let now = Instant::now();
            let due = {
                let mut last = self
                    .last_capture
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let due = capture_due(*last, min_interval, now);
                *last = Some(due);
                due
            };
            if due > now {
                tokio::time::sleep(due - now).await;
            }
        }

        let data = with_deadline(
            "screenshot",
            COMMAND_TIMEOUT,
            self.page.screenshot(ScreenshotParams::default()),
        )
        .await?;
        Ok(Screenshot {
            format: ImageFormat::Png,
            data,
        })
    }

    async fn start_screencast(&self) -> Result<ScreencastStream, EngineError> {
        use futures::StreamExt;

        // Register the listeners before starting so early frames are
        // not lost (docs/dashboard.md: correct ack loop).
        let mut frames = self
            .page
            .event_listener::<EventScreencastFrame>()
            .await
            .map_err(fold)?;
        let mut navigations = self
            .page
            .event_listener::<EventFrameNavigated>()
            .await
            .map_err(fold)?;
        let start = screencast_params();
        with_deadline(
            "start_screencast",
            COMMAND_TIMEOUT,
            self.page.execute(start),
        )
        .await?;

        let (sender, receiver) = tokio::sync::mpsc::channel::<ScreencastFrame>(4);
        let task_page = self.page.clone();
        // The forwarding task owns the capture lifecycle: acks every
        // frame (docs/dashboard.md), restarts after navigations where CDP
        // stops the capture on its own, and stops when the viewer drops
        // the stream. Every CDP call inside stays under a deadline so a
        // dead browser ends the stream instead of parking the task.
        //
        // Frames are handed off without awaiting: a stalled viewer must
        // never block this task, because the backlog would then pile up
        // in the chromiumoxide event listener, which is an unbounded
        // queue. Frames are droppable (docs/events.md, latest-wins
        // backpressure); over a full channel the newest frames are
        // dropped and the memory stays bounded by the channel capacity.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(event) = frames.next() => {
                        let _ = with_deadline(
                            "screencast_ack",
                            COMMAND_TIMEOUT,
                            task_page.execute(ScreencastFrameAckParams::new(event.session_id)),
                        )
                        .await;
                        use base64::Engine as _;
                        let Ok(jpeg) =
                            base64::engine::general_purpose::STANDARD.decode(AsRef::<[u8]>::as_ref(&event.data))
                        else {
                            continue;
                        };
                        match sender.try_send(ScreencastFrame { jpeg }) {
                            Ok(()) => {}
                            // A slow viewer loses frames, not memory.
                            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
                            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => break,
                        }
                    }
                    Some(_) = navigations.next() => {
                        // CDP stops the capture on navigation; restart it.
                        let _ = with_deadline(
                            "screencast_restart",
                            COMMAND_TIMEOUT,
                            task_page.execute(screencast_params()),
                        )
                        .await;
                    }
                    else => break,
                }
            }
            let _ = with_deadline(
                "stop_screencast",
                COMMAND_TIMEOUT,
                task_page.execute(StopScreencastParams::default()),
            )
            .await;
        });

        Ok(ScreencastStream::new(receiver))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_capture_is_immediate() {
        let now = Instant::now();
        assert_eq!(capture_due(None, Duration::from_secs(10), now), now);
    }

    #[test]
    fn capture_waits_out_the_remaining_interval() {
        let now = Instant::now();
        let last = now - Duration::from_secs(2);
        assert_eq!(
            capture_due(Some(last), Duration::from_secs(10), now),
            now + Duration::from_secs(8)
        );
    }

    #[test]
    fn elapsed_interval_does_not_delay_the_capture() {
        let now = Instant::now();
        let last = now - Duration::from_secs(20);
        assert_eq!(capture_due(Some(last), Duration::from_secs(10), now), now);
    }

    #[test]
    fn concurrent_capture_queues_behind_a_reserved_slot() {
        let now = Instant::now();
        // A slot another capture is already sleeping on; elapsed() would
        // saturate to zero here and reset the schedule instead.
        let reserved = now + Duration::from_secs(7);
        assert_eq!(
            capture_due(Some(reserved), Duration::from_secs(10), now),
            reserved + Duration::from_secs(10)
        );
    }

    #[test]
    fn zero_interval_never_waits() {
        let now = Instant::now();
        assert_eq!(capture_due(Some(now), Duration::ZERO, now), now);
    }
}
