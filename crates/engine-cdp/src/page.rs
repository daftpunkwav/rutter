//! CDP implementation of page-scoped operations.
//!
//! Boundary: one CDP target per handle. Every operation carries a
//! timeout; navigation uses the context's configured
//! budget, the others a fixed bound so no call can wait forever.
//! Chromiumoxide types are implementation details and never appear in
//! the public `rutter-engine` trait signatures.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::dom::SetFileInputFilesParams;
use chromiumoxide::cdp::browser_protocol::emulation::{
    ScreenOrientation, ScreenOrientationType, SetDeviceMetricsOverrideParams,
};
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventType, DispatchMouseEventType, InsertTextParams,
};
use chromiumoxide::cdp::browser_protocol::network::{
    EventLoadingFailed, EventRequestWillBeSent, EventResponseReceived, RequestId,
};
use chromiumoxide::cdp::browser_protocol::page::{
    EventFrameNavigated, EventJavascriptDialogOpening, EventScreencastFrame,
    GetNavigationHistoryParams, HandleJavaScriptDialogParams, NavigateParams,
    NavigateToHistoryEntryParams, ScreencastFrameAckParams, StartScreencastFormat,
    StartScreencastParams, StopScreencastParams,
};
use chromiumoxide::cdp::browser_protocol::target::TargetId;
use chromiumoxide::cdp::js_protocol::runtime::{
    EvaluateParams, EventConsoleApiCalled, EventExceptionThrown, RemoteObject,
};
use chromiumoxide::error::CdpError;
use chromiumoxide::page::ScreenshotParams;
use serde_json::Value;

use rutter_engine::error::EngineError;
use rutter_engine::input::InputEvent;
use rutter_engine::page::{
    ConsoleLevel, DialogKind, ImageFormat, ObservationStream, PageObservation, RequestEntry,
    ScreencastFrame, ScreencastStream, Screenshot,
};
use std::collections::{HashMap, VecDeque};

use crate::error::{self, COMMAND_TIMEOUT, fold, fold_navigation, with_deadline, with_deadline_by};
use crate::keys;

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

/// Cap on one observation's text: a hostile or chatty page must not be
/// able to bloat the event ring or a console feed with one call.
const OBSERVATION_TEXT_CAP: usize = 2_000;

/// Bounds an observation text at [`OBSERVATION_TEXT_CAP`] characters.
fn cap_text(text: String) -> String {
    if text.chars().count() <= OBSERVATION_TEXT_CAP {
        return text;
    }
    let mut capped: String = text.chars().take(OBSERVATION_TEXT_CAP).collect();
    capped.push('…');
    capped
}

/// Maps a protocol dialog kind onto the engine vocabulary.
fn dialog_kind(kind: &str) -> DialogKind {
    match kind {
        "confirm" => DialogKind::Confirm,
        "prompt" => DialogKind::Prompt,
        "beforeunload" => DialogKind::Beforeunload,
        _ => DialogKind::Alert,
    }
}

/// Maps a protocol console call type onto the engine severity; the
/// grouped and structured calls count as ordinary log output.
fn console_level(call_type: &str) -> ConsoleLevel {
    match call_type {
        "debug" => ConsoleLevel::Debug,
        "info" => ConsoleLevel::Info,
        "warning" => ConsoleLevel::Warning,
        "error" | "assert" => ConsoleLevel::Error,
        _ => ConsoleLevel::Log,
    }
}

/// Formats one console argument the way a developer console would: a
/// string argument verbatim, other values as JSON, and a type-name
/// placeholder when the value carried neither.
fn console_argument(argument: &RemoteObject) -> String {
    if let Some(value) = argument.value.as_ref() {
        return match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
    }
    if let Some(description) = argument.description.as_deref() {
        return description.to_owned();
    }
    format!("[{}]", argument.r#type.as_ref())
}

/// Formats a console call's text from its arguments, joined with
/// spaces like a developer console's default formatting.
fn console_text(arguments: &[RemoteObject]) -> String {
    let joined = arguments
        .iter()
        .map(console_argument)
        .collect::<Vec<_>>()
        .join(" ");
    cap_text(joined)
}

/// Formats an uncaught exception's text: the thrown value's
/// description when the protocol carried one, otherwise the bare
/// exception text.
fn exception_text(details: &chromiumoxide::cdp::js_protocol::runtime::ExceptionDetails) -> String {
    let text = details
        .exception
        .as_ref()
        .and_then(|exception| exception.description.as_deref())
        .unwrap_or(details.text.as_str());
    cap_text(text.to_owned())
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
    /// Whether the page's observation feed has been handed out; the
    /// feed is single-consumer, so a second `observe` call fails
    /// instead of silently splitting the observations.
    observation_claimed: Mutex<bool>,
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
            observation_claimed: Mutex::new(false),
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
            observation_claimed: Mutex::new(false),
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
                let params = keys::key_params(DispatchKeyEventType::KeyDown, &key);
                return with_deadline("dispatch_key", COMMAND_TIMEOUT, self.page.execute(params))
                    .await
                    .map(|_| ());
            }
            InputEvent::KeyReleased { key } => {
                let params = keys::key_params(DispatchKeyEventType::KeyUp, &key);
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
        // not lost (correct ack loop).
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
        // frame, restarts after navigations where CDP
        // stops the capture on its own, and stops when the viewer drops
        // the stream. Every CDP call inside stays under a deadline so a
        // dead browser ends the stream instead of parking the task.
        //
        // Frames are handed off without awaiting: a stalled viewer must
        // never block this task, because the backlog would then pile up
        // in the chromiumoxide event listener, which is an unbounded
        // queue. Frames are droppable (latest-wins backpressure); over
        // a full channel the newest frames are
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
                    // A quiet page paints no frames, so the Closed arm
                    // above may not fire for a long time; watching the
                    // receiver directly ends the capture the moment the
                    // viewer leaves (streaming stops when the last
                    // viewer leaves).
                    _ = sender.closed() => break,
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

    async fn observe(&self) -> Result<ObservationStream, EngineError> {
        use futures::StreamExt;

        {
            let mut claimed = self
                .observation_claimed
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if *claimed {
                return Err(EngineError::Unsupported {
                    operation: "observe".to_owned(),
                    reason: "this page's observation feed already has a consumer".to_owned(),
                });
            }
            *claimed = true;
        }

        // Register the listeners before returning so observations from
        // the first ticks are not lost.
        let mut dialogs = self
            .page
            .event_listener::<EventJavascriptDialogOpening>()
            .await
            .map_err(fold)?;
        let mut console = self
            .page
            .event_listener::<EventConsoleApiCalled>()
            .await
            .map_err(fold)?;
        let mut exceptions = self
            .page
            .event_listener::<EventExceptionThrown>()
            .await
            .map_err(fold)?;
        let mut requests = self
            .page
            .event_listener::<EventRequestWillBeSent>()
            .await
            .map_err(fold)?;
        let mut responses = self
            .page
            .event_listener::<EventResponseReceived>()
            .await
            .map_err(fold)?;
        let mut failures = self
            .page
            .event_listener::<EventLoadingFailed>()
            .await
            .map_err(fold)?;

        let mut pending = PendingRequests::default();

        let (sender, receiver) = tokio::sync::mpsc::channel::<PageObservation>(64);
        // The forwarding task owns the feed: observations hand off
        // without awaiting, so a stalled consumer loses observations
        // instead of piling backlog into the chromiumoxide listeners,
        // which are an unbounded queue (same rule as the screencast).
        // Every observation is droppable; the feed ends when the page
        // or the consumer goes away.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(event) = dialogs.next() => {
                        let observation = PageObservation::DialogOpened {
                            kind: dialog_kind(event.r#type.as_ref()),
                            message: cap_text(event.message.clone()),
                        };
                        if !try_forward(&sender, observation) {
                            break;
                        }
                    }
                    Some(event) = console.next() => {
                        let observation = PageObservation::ConsoleEmitted {
                            level: console_level(event.r#type.as_ref()),
                            text: console_text(&event.args),
                        };
                        if !try_forward(&sender, observation) {
                            break;
                        }
                    }
                    Some(event) = exceptions.next() => {
                        let observation = PageObservation::UncaughtException {
                            text: exception_text(&event.exception_details),
                        };
                        if !try_forward(&sender, observation) {
                            break;
                        }
                    }
                    Some(event) = requests.next() => {
                        pending.note(
                            event.request_id.clone(),
                            event.request.method.clone(),
                            event.request.url.clone(),
                        );
                    }
                    Some(event) = responses.next() => {
                        let (method, _) = pending
                            .take(&event.request_id)
                            .unwrap_or_else(|| (String::new(), String::new()));
                        let observation = PageObservation::RequestObserved {
                            entry: RequestEntry {
                                method,
                                url: cap_text(event.response.url.clone()),
                                status: Some(event.response.status.clamp(0, u32::MAX as i64) as u32),
                                resource_type: Some(event.r#type.as_ref().to_lowercase()),
                                error: None,
                            },
                        };
                        if !try_forward(&sender, observation) {
                            break;
                        }
                    }
                    Some(event) = failures.next() => {
                        let (method, url) = pending
                            .take(&event.request_id)
                            .unwrap_or_else(|| (String::new(), String::new()));
                        let observation = PageObservation::RequestObserved {
                            entry: RequestEntry {
                                method,
                                url: cap_text(url),
                                status: None,
                                resource_type: Some(event.r#type.as_ref().to_lowercase()),
                                error: Some(cap_text(event.error_text.clone())),
                            },
                        };
                        if !try_forward(&sender, observation) {
                            break;
                        }
                    }
                    // A quiet page produces no observations, so the
                    // Closed arm inside `try_forward` may not fire for a
                    // long time; watching the sender directly ends the
                    // feed the moment the consumer leaves (same rule as
                    // the screencast).
                    _ = sender.closed() => break,
                    else => break,
                }
            }
        });

        Ok(ObservationStream::new(receiver))
    }

    async fn handle_dialog(
        &self,
        accept: bool,
        prompt_text: Option<&str>,
    ) -> Result<(), EngineError> {
        let mut params = HandleJavaScriptDialogParams::new(accept);
        params.prompt_text = prompt_text.map(str::to_owned);
        with_deadline("handle_dialog", COMMAND_TIMEOUT, self.page.execute(params))
            .await
            .map(|_| ())
    }

    async fn set_input_files(&self, reference: &str, files: &[String]) -> Result<(), EngineError> {
        // The element resolves through the same page-side reference
        // store every snapshot script uses (rutter-observe owns that
        // contract); the element comes back as a remote object handle,
        // which the file operation targets. A stale reference answers
        // `null` and carries no object id.
        let mut params = EvaluateParams::new(rutter_observe::element_script(reference));
        params.return_by_value = Some(false);
        let result = with_deadline("set_input_files_resolve", COMMAND_TIMEOUT, async {
            self.page.evaluate_expression(params).await
        })
        .await?;
        let object_id =
            result
                .object()
                .object_id
                .clone()
                .ok_or_else(|| EngineError::ReferenceExpired {
                    reference: reference.to_owned(),
                })?;
        let mut set = SetFileInputFilesParams::new(files.to_vec());
        set.object_id = Some(object_id);
        with_deadline("set_input_files", COMMAND_TIMEOUT, self.page.execute(set))
            .await
            .map(|_| ())
    }

    async fn set_viewport(&self, width: u32, height: u32) -> Result<(), EngineError> {
        let metrics = SetDeviceMetricsOverrideParams::builder()
            .mobile(false)
            .width(width)
            .height(height)
            .device_scale_factor(1.0)
            .screen_orientation(ScreenOrientation::new(
                ScreenOrientationType::PortraitPrimary,
                0,
            ))
            .build()
            .map_err(|error| EngineError::Internal {
                detail: format!("viewport override params: {error}"),
            })?;
        with_deadline("set_viewport", COMMAND_TIMEOUT, self.page.execute(metrics))
            .await
            .map(|_| ())
    }
}

/// Correlates requests that were sent with their terminal outcome.
/// Requests that never finish are bounded: once [`PENDING_CAP`] are
/// in flight, the oldest falls off and its eventual outcome degrades
/// to a method-less entry. Free of I/O so the rules are unit-testable.
#[derive(Default)]
struct PendingRequests {
    map: HashMap<RequestId, (String, String)>,
    order: VecDeque<RequestId>,
}

/// How many requests may sit unanswered before the oldest is dropped.
const PENDING_CAP: usize = 256;

impl PendingRequests {
    fn note(&mut self, id: RequestId, method: String, url: String) {
        if !self.map.contains_key(&id)
            && self.order.len() >= PENDING_CAP
            && let Some(oldest) = self.order.pop_front()
        {
            self.map.remove(&oldest);
        }
        if !self.map.contains_key(&id) {
            self.order.push_back(id.clone());
        }
        self.map.insert(id, (method, url));
    }

    fn take(&mut self, id: &RequestId) -> Option<(String, String)> {
        let entry = self.map.remove(id);
        if entry.is_some() {
            self.order.retain(|pending| pending != id);
        }
        entry
    }
}

/// Hands one observation to the feed's consumer; `false` means the
/// consumer is gone and the forwarding task should end.
fn try_forward(
    sender: &tokio::sync::mpsc::Sender<PageObservation>,
    observation: PageObservation,
) -> bool {
    match sender.try_send(observation) {
        Ok(()) => true,
        // A stalled consumer loses observations, not memory.
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => true,
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => false,
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

    #[test]
    fn dialog_kinds_map_by_wire_name_with_alert_as_the_default() {
        assert_eq!(dialog_kind("alert"), DialogKind::Alert);
        assert_eq!(dialog_kind("confirm"), DialogKind::Confirm);
        assert_eq!(dialog_kind("prompt"), DialogKind::Prompt);
        assert_eq!(dialog_kind("beforeunload"), DialogKind::Beforeunload);
        assert_eq!(dialog_kind("AccountChooser"), DialogKind::Alert);
    }

    #[test]
    fn console_severities_map_by_call_type() {
        assert_eq!(console_level("log"), ConsoleLevel::Log);
        assert_eq!(console_level("dir"), ConsoleLevel::Log);
        assert_eq!(console_level("table"), ConsoleLevel::Log);
        assert_eq!(console_level("debug"), ConsoleLevel::Debug);
        assert_eq!(console_level("info"), ConsoleLevel::Info);
        assert_eq!(console_level("warning"), ConsoleLevel::Warning);
        assert_eq!(console_level("error"), ConsoleLevel::Error);
        assert_eq!(console_level("assert"), ConsoleLevel::Error);
    }

    #[test]
    fn console_arguments_format_like_a_developer_console() {
        use chromiumoxide::cdp::js_protocol::runtime::{RemoteObject, RemoteObjectType};
        let bare_object = |r#type: RemoteObjectType| RemoteObject {
            r#type,
            subtype: None,
            class_name: None,
            value: None,
            unserializable_value: None,
            description: None,
            deep_serialized_value: None,
            object_id: None,
            preview: None,
            custom_preview: None,
        };
        let string_argument = RemoteObject {
            value: Some(Value::String("careful".to_owned())),
            ..bare_object(RemoteObjectType::String)
        };
        let object_argument = RemoteObject {
            value: Some(serde_json::json!({ "a": 1 })),
            ..bare_object(RemoteObjectType::Object)
        };
        assert_eq!(console_argument(&string_argument), "careful");
        assert_eq!(console_argument(&object_argument), r#"{"a":1}"#);

        let opaque = RemoteObject {
            description: Some("function click() {}".to_owned()),
            ..bare_object(RemoteObjectType::Function)
        };
        assert_eq!(console_argument(&opaque), "function click() {}");

        let bare = bare_object(RemoteObjectType::Symbol);
        assert_eq!(console_argument(&bare), "[symbol]");

        let joined = console_text(&[string_argument, object_argument]);
        assert_eq!(joined, "careful {\"a\":1}");
    }

    #[test]
    fn observation_text_is_capped_for_hostile_pages() {
        let short = cap_text("short".to_owned());
        assert_eq!(short, "short");

        let huge: String = "x".repeat(OBSERVATION_TEXT_CAP * 2);
        let capped = cap_text(huge);
        assert_eq!(capped.chars().count(), OBSERVATION_TEXT_CAP + 1);
        assert!(capped.ends_with('…'));
    }

    #[test]
    fn a_feed_consumer_leaving_ends_the_forwarding_decision() {
        let (sender, receiver) = tokio::sync::mpsc::channel::<PageObservation>(1);
        let observation = PageObservation::UncaughtException {
            text: "boom".to_owned(),
        };
        assert!(try_forward(&sender, observation.clone()));
        drop(receiver);
        assert!(
            !try_forward(&sender, observation),
            "a closed feed ends the task"
        );
    }
}

#[cfg(test)]
mod pending_tests {
    use super::*;

    fn id(name: &str) -> RequestId {
        RequestId::new(name)
    }

    #[test]
    fn outcomes_pair_with_their_request() {
        let mut pending = PendingRequests::default();
        pending.note(id("r1"), "GET".to_owned(), "https://a.example".to_owned());
        pending.note(id("r2"), "POST".to_owned(), "https://b.example".to_owned());

        assert_eq!(
            pending.take(&id("r1")),
            Some(("GET".to_owned(), "https://a.example".to_owned()))
        );
        assert_eq!(pending.take(&id("r1")), None, "a request answers once");
        assert_eq!(
            pending.take(&id("r2")),
            Some(("POST".to_owned(), "https://b.example".to_owned()))
        );
    }

    #[test]
    fn unknown_requests_degrade_to_a_methodless_entry() {
        let mut pending = PendingRequests::default();
        assert_eq!(pending.take(&id("ghost")), None);
    }

    #[test]
    fn pending_requests_are_bounded_and_the_oldest_falls_off() {
        let mut pending = PendingRequests::default();
        for index in 0..PENDING_CAP + 10 {
            pending.note(id(&format!("r{index}")), "GET".to_owned(), String::new());
        }
        // The oldest ten fell off: their outcomes degrade.
        assert_eq!(pending.take(&id("r0")), None);
        assert_eq!(pending.take(&id("r9")), None);
        assert!(pending.take(&id("r10")).is_some());
        // The cap freed room, so everything answered stays accounted.
        assert_eq!(
            pending.take(&id(&format!("r{}", PENDING_CAP + 9))),
            Some(("GET".to_owned(), String::new()))
        );
    }
}
