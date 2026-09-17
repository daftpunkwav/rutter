//! CDP implementation of page-scoped operations.
//!
//! Boundary: one CDP target per handle. Every operation carries a
//! timeout (blueprint §8.4); navigation uses the context's configured
//! budget, the others a fixed bound so no call can wait forever.
//! Chromiumoxide types are implementation details and never appear in
//! the public `rutter-engine` trait signatures.

use std::time::Duration;

use async_trait::async_trait;
use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::input::{DispatchKeyEventType, DispatchMouseEventType};
use chromiumoxide::cdp::browser_protocol::page::NavigateParams;
use chromiumoxide::cdp::browser_protocol::target::TargetId;
use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use chromiumoxide::page::ScreenshotParams;
use serde_json::Value;

use rutter_engine::error::EngineError;
use rutter_engine::input::InputEvent;
use rutter_engine::page::{ImageFormat, Screenshot};

use crate::error::{self, COMMAND_TIMEOUT, fold, fold_navigation, with_deadline, with_deadline_by};

/// One CDP target (tab) wrapped as a page handle.
pub struct CdpPage {
    page: Page,
    navigation_timeout: Duration,
}

impl CdpPage {
    /// Creates a handle over a chromiumoxide page.
    pub fn new(page: Page, navigation_timeout: Duration) -> Self {
        Self {
            page,
            navigation_timeout,
        }
    }

    /// Target id for closing the tab through the browser connection
    /// (avoids consuming the page handle).
    pub fn target_id(&self) -> TargetId {
        self.page.target_id().clone()
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

        self.page
            .url()
            .await
            .map_err(fold)?
            .ok_or_else(|| EngineError::Internal {
                detail: "page URL unavailable after navigation".to_owned(),
            })
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
}
