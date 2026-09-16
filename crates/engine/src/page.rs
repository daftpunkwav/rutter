//! Page-scoped engine operations.
//!
//! Boundary: one tab/target inside a context. Navigation readiness,
//! auto-wait semantics, and snapshots-after-acting are orchestration
//! concerns that live in `rutter-session`; implementations only execute
//! the raw operation.

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

/// Operations on one page (tab) inside a context.
#[async_trait]
pub trait PageHandle: Send + Sync {
    /// Navigates the page and resolves with the effective URL after
    /// redirects.
    async fn navigate(&self, url: &str) -> Result<String, EngineError>;

    /// Evaluates a JavaScript expression in the page and resolves with
    /// its JSON result.
    async fn evaluate(&self, expression: &str) -> Result<Value, EngineError>;

    /// Dispatches one input event to the page.
    async fn dispatch_input(&self, event: InputEvent) -> Result<(), EngineError>;

    /// Captures an image of the current viewport.
    async fn capture_screenshot(&self) -> Result<Screenshot, EngineError>;
}
