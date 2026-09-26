//! Page-scoped engine operations.
//!
//! Boundary: one tab/target inside a context. Navigation readiness,
//! auto-wait semantics, and snapshots-after-acting are orchestration
//! concerns that live in `rutter-session`; implementations only execute
//! the raw operation.

use std::fmt;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::EngineError;
use crate::input::InputEvent;

/// Pixel format of captured image data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    /// Portable Network Graphics; lossless.
    Png,
    /// JPEG; the screencast format.
    Jpeg,
}

/// A captured image of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    /// Format of the encoded bytes.
    pub format: ImageFormat,
    /// Encoded image bytes.
    pub data: Vec<u8>,
}

/// One JPEG frame from a live page screencast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreencastFrame {
    /// Encoded JPEG bytes.
    pub jpeg: Vec<u8>,
}

/// A live screencast frame stream; dropping it stops the capture
/// (docs/dashboard.md: on-demand, stops when the last viewer leaves).
pub struct ScreencastStream {
    receiver: tokio::sync::mpsc::Receiver<ScreencastFrame>,
}

impl ScreencastStream {
    /// Wraps a frame receiver; backends build streams over channels.
    pub fn new(receiver: tokio::sync::mpsc::Receiver<ScreencastFrame>) -> Self {
        Self { receiver }
    }

    /// Awaits the next frame; `None` once the capture stopped.
    pub async fn next_frame(&mut self) -> Option<ScreencastFrame> {
        self.receiver.recv().await
    }
}

/// The kind of a JavaScript dialog a page opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DialogKind {
    /// `alert()` — the page pauses until the dialog is answered.
    Alert,
    /// `confirm()` — answering dismiss resolves the call to `false`.
    Confirm,
    /// `prompt()` — answering dismiss resolves the call to `null`.
    Prompt,
    /// A `beforeunload` confirmation.
    Beforeunload,
}

impl DialogKind {
    /// The lowercase wire form the event backbone carries.
    pub fn as_str(&self) -> &'static str {
        match self {
            DialogKind::Alert => "alert",
            DialogKind::Confirm => "confirm",
            DialogKind::Prompt => "prompt",
            DialogKind::Beforeunload => "beforeunload",
        }
    }
}

/// The severity of a console call a page made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConsoleLevel {
    /// `console.log` and the grouped/structured calls.
    Log,
    /// `console.debug`.
    Debug,
    /// `console.info`.
    Info,
    /// `console.warn`.
    Warning,
    /// `console.error` and `console.assert` failures.
    Error,
}

impl ConsoleLevel {
    /// The lowercase label tool output carries.
    pub fn as_str(&self) -> &'static str {
        match self {
            ConsoleLevel::Log => "log",
            ConsoleLevel::Debug => "debug",
            ConsoleLevel::Info => "info",
            ConsoleLevel::Warning => "warning",
            ConsoleLevel::Error => "error",
        }
    }
}

impl fmt::Display for ConsoleLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One recorded console line: severity plus the formatted call text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleEntry {
    /// Severity of the console call.
    pub level: ConsoleLevel,
    /// Formatted text of the call, arguments joined.
    pub text: String,
}

/// A fact a page produced on its own — no orchestration asked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageObservation {
    /// The page opened a JavaScript dialog. Until the dialog is answered
    /// the page is wedged, so consumers are expected to answer through
    /// [`PageHandle::handle_dialog`].
    DialogOpened {
        /// Which dialog kind opened.
        kind: DialogKind,
        /// The message the dialog shows.
        message: String,
    },
    /// The page made a `console.*` call.
    ConsoleEmitted {
        /// Severity of the call.
        level: ConsoleLevel,
        /// Formatted text of the call.
        text: String,
    },
    /// The page threw an uncaught exception.
    UncaughtException {
        /// The exception text.
        text: String,
    },
}

/// The live feed of one page's [`PageObservation`]s; the stream ends
/// when the page closes or the backend stops observing it.
pub struct ObservationStream {
    receiver: tokio::sync::mpsc::Receiver<PageObservation>,
}

impl ObservationStream {
    /// Wraps an observation receiver; backends build streams over
    /// channels.
    pub fn new(receiver: tokio::sync::mpsc::Receiver<PageObservation>) -> Self {
        Self { receiver }
    }

    /// Awaits the next observation; `None` once the feed ended.
    pub async fn next_observation(&mut self) -> Option<PageObservation> {
        self.receiver.recv().await
    }
}

/// Operations on one page (tab) inside a context.
#[async_trait]
pub trait PageHandle: Send + Sync {
    /// Navigates the page and resolves with the effective URL after
    /// redirects.
    async fn navigate(&self, url: &str) -> Result<String, EngineError>;

    /// Reloads the current page.
    async fn reload(&self) -> Result<(), EngineError>;

    /// Goes back one history entry; resolves with the effective URL.
    async fn go_back(&self) -> Result<String, EngineError>;

    /// Goes forward one history entry; resolves with the effective URL.
    async fn go_forward(&self) -> Result<String, EngineError>;

    /// Evaluates a JavaScript expression in the page and resolves with
    /// its JSON result.
    async fn evaluate(&self, expression: &str) -> Result<Value, EngineError>;

    /// Dispatches one input event to the page.
    async fn dispatch_input(&self, event: InputEvent) -> Result<(), EngineError>;

    /// Captures an image of the current viewport.
    async fn capture_screenshot(&self) -> Result<Screenshot, EngineError>;

    /// Starts a JPEG screencast (~1-5 fps, width capped); frames flow
    /// on the returned stream until it is dropped (docs/dashboard.md).
    /// Backends must restart the capture after navigations, where the
    /// protocol stops it on its own.
    async fn start_screencast(&self) -> Result<ScreencastStream, EngineError>;

    /// Streams what the page produces on its own: dialogs it opens,
    /// console calls it makes, uncaught exceptions it throws. The feed
    /// is single-consumer — a second call on the same page fails — and
    /// ends when the page closes.
    ///
    /// A backend that cannot surface observations answers a stream that
    /// ends immediately; nothing above this trait may assume an
    /// observation arrives, and every consumer must keep working when
    /// none do.
    async fn observe(&self) -> Result<ObservationStream, EngineError> {
        let _ = self;
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        drop(sender);
        Ok(ObservationStream::new(receiver))
    }

    /// Answers a JavaScript dialog the page opened through
    /// [`PageHandle::observe`]; a `prompt` answered with `None` keeps
    /// its default text. Fails when no dialog is open or the backend
    /// cannot answer dialogs.
    async fn handle_dialog(
        &self,
        accept: bool,
        prompt_text: Option<&str>,
    ) -> Result<(), EngineError> {
        let _ = (self, accept, prompt_text);
        Err(EngineError::Unsupported {
            operation: "handle_dialog".to_owned(),
            reason: "this engine cannot answer page dialogs".to_owned(),
        })
    }

    /// Sets the files of the file input a snapshot reference points to.
    /// Paths are resolved by the machine the engine runs on; a missing
    /// path fails here. Fails with [`EngineError::ReferenceExpired`]
    /// when the reference no longer resolves. Backends that cannot set
    /// files answer [`EngineError::Unsupported`].
    async fn set_input_files(&self, reference: &str, files: &[String]) -> Result<(), EngineError> {
        let _ = (self, reference, files);
        Err(EngineError::Unsupported {
            operation: "set_input_files".to_owned(),
            reason: "this engine cannot set files on file inputs".to_owned(),
        })
    }

    /// Overrides the page's viewport size in CSS pixels. Backends that
    /// cannot resize a page answer [`EngineError::Unsupported`].
    async fn set_viewport(&self, width: u32, height: u32) -> Result<(), EngineError> {
        let _ = (self, width, height);
        Err(EngineError::Unsupported {
            operation: "set_viewport".to_owned(),
            reason: "this engine cannot resize a page".to_owned(),
        })
    }
}
