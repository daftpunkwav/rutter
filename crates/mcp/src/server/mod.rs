//! The MCP server: rmcp host mapping docs/tool-catalog.md onto the session layer.
//!
//! Responsibilities:
//! - Expose the docs/tool-catalog.md tool surface as rmcp tools.
//! - Return snapshots as text, screenshots as image blocks, and action
//!   failures as `isError` results carrying the error plus its hint
//!   (`docs/tool-catalog.md` §2).
//!
//! Boundary: protocol mapping only. All semantics live in
//! `rutter-session`; this module validates parameters against the spec
//! and never touches the engine itself. One MCP connection is one
//! session, and the engine starts lazily on the first tool call.
//!
//! Module layout: `params` holds the tool input types; this file
//! holds the server, the tool implementations, and the result mapping.

mod params;

pub use params::{
    CookieInput, CookiesParams, Direction, NavigateParams, PageParams, PressKeyParams,
    ReferenceParams, SameSiteInput, ScrollParams, SelectOptionParams, TypeParams, WaitForParams,
};

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorCode, Implementation, ServerCapabilities, ServerConfig,
};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};

use rutter_core::action::{Action, Origin};
use rutter_core::cookie::Cookie;
use rutter_core::ids::{PageId, SessionId};
use rutter_core::reference::Reference;
use rutter_session::error::SessionError;
use rutter_session::manager::SessionManager;
use rutter_session::session::Session;

/// Server error code for engine-level failures beyond a tool result
/// (`docs/tool-catalog.md` §2).
const SERVER_ERROR_CODE: i32 = -32000;

/// Default `wait_for` budget when the caller sends no timeout
/// (`docs/tool-catalog.md` §4: 10 000 ms).
const WAIT_FOR_DEFAULT_BUDGET: Duration = Duration::from_secs(10);

/// The per-connection MCP server.
#[derive(Clone)]
pub struct RutterMcp {
    manager: Arc<SessionManager>,
    session_id: SessionId,
    session: Arc<tokio::sync::OnceCell<Arc<Session>>>,
}

#[tool_router]
impl RutterMcp {
    /// Builds the server for one connection over one manager.
    pub fn new(manager: Arc<SessionManager>, session_id: SessionId) -> Self {
        Self {
            manager,
            session_id,
            session: Arc::new(tokio::sync::OnceCell::new()),
        }
    }

    /// The session for this connection, created on first use. After a
    /// `close_session` call the manager no longer knows the session, so
    /// the cached one is discarded and later calls fail fast instead of
    /// resurrecting a closed session.
    async fn session(&self) -> Result<Arc<Session>, McpError> {
        if let Some(session) = self.session.get() {
            return match self.manager.get_session(&self.session_id).await {
                Some(_) => Ok(Arc::clone(session)),
                None => Err(McpError::invalid_params(
                    format!(
                        "session '{}' is closed; reconnect to open a new one",
                        self.session_id
                    ),
                    None,
                )),
            };
        }
        let session = self
            .session
            .get_or_try_init(|| {
                let manager = Arc::clone(&self.manager);
                let id = self.session_id.clone();
                async move { manager.session(id).await }
            })
            .await
            .map_err(protocol_error)?;
        Ok(Arc::clone(session))
    }

    #[tool(description = "Navigate the active page to a URL and return a fresh snapshot")]
    async fn navigate(
        &self,
        Parameters(NavigateParams { url }): Parameters<NavigateParams>,
    ) -> Result<CallToolResult, McpError> {
        self.run_action(Action::Navigate { url }).await
    }

    #[tool(description = "Go back one history entry and return a fresh snapshot")]
    async fn back(&self) -> Result<CallToolResult, McpError> {
        self.run_action(Action::Back).await
    }

    #[tool(description = "Go forward one history entry and return a fresh snapshot")]
    async fn forward(&self) -> Result<CallToolResult, McpError> {
        self.run_action(Action::Forward).await
    }

    #[tool(description = "Reload the active page and return a fresh snapshot")]
    async fn reload(&self) -> Result<CallToolResult, McpError> {
        self.run_action(Action::Reload).await
    }

    #[tool(description = "Render the active page as a YAML accessibility snapshot")]
    async fn snapshot(&self) -> Result<CallToolResult, McpError> {
        let session = self.session().await?;
        match session.snapshot().await {
            Ok(snapshot) => Ok(snapshot_result(&snapshot)),
            Err(error) => Ok(error_result(&error)),
        }
    }

    #[tool(description = "Capture the active page as a PNG image")]
    async fn screenshot(&self) -> Result<CallToolResult, McpError> {
        let session = self.session().await?;
        match session.screenshot().await {
            Ok(image) => {
                let data = base64::engine::general_purpose::STANDARD.encode(&image.data);
                let mime = match image.format {
                    rutter_session::ImageFormat::Png => "image/png",
                    rutter_session::ImageFormat::Jpeg => "image/jpeg",
                };
                Ok(CallToolResult::success(vec![ContentBlock::image(
                    data, mime,
                )]))
            }
            Err(error) => Ok(error_result(&error)),
        }
    }

    #[tool(description = "Click the element a snapshot reference points to")]
    async fn click(
        &self,
        Parameters(ReferenceParams { reference }): Parameters<ReferenceParams>,
    ) -> Result<CallToolResult, McpError> {
        self.run_action(Action::Click {
            reference: Reference::new(reference),
        })
        .await
    }

    #[tool(description = "Hover the element a snapshot reference points to")]
    async fn hover(
        &self,
        Parameters(ReferenceParams { reference }): Parameters<ReferenceParams>,
    ) -> Result<CallToolResult, McpError> {
        self.run_action(Action::Hover {
            reference: Reference::new(reference),
        })
        .await
    }

    #[tool(
        name = "type",
        description = "Focus an element and type text into it, returning a fresh snapshot"
    )]
    async fn type_text(
        &self,
        Parameters(TypeParams { reference, text }): Parameters<TypeParams>,
    ) -> Result<CallToolResult, McpError> {
        self.run_action(Action::Type {
            reference: Reference::new(reference),
            text,
        })
        .await
    }

    #[tool(description = "Press a single key (for example Enter or Tab) on the active page")]
    async fn press_key(
        &self,
        Parameters(PressKeyParams { key }): Parameters<PressKeyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.run_action(Action::PressKey { key }).await
    }

    #[tool(description = "Select options by value on a select element")]
    async fn select_option(
        &self,
        Parameters(SelectOptionParams { reference, values }): Parameters<SelectOptionParams>,
    ) -> Result<CallToolResult, McpError> {
        self.run_action(Action::SelectOption {
            reference: Reference::new(reference),
            values,
        })
        .await
    }

    #[tool(description = "Scroll the page or a container by an amount in pixels")]
    async fn scroll(
        &self,
        Parameters(ScrollParams {
            direction,
            amount,
            reference,
        }): Parameters<ScrollParams>,
    ) -> Result<CallToolResult, McpError> {
        if amount == 0 {
            return Err(invalid_params(
                "amount must be greater than zero".to_owned(),
            ));
        }
        self.run_action(Action::Scroll {
            reference: reference.map(Reference::new),
            direction: direction.into(),
            amount,
        })
        .await
    }

    #[tool(description = "Wait until text appears on the page, then return a snapshot")]
    async fn wait_for(
        &self,
        Parameters(WaitForParams { text, timeout_ms }): Parameters<WaitForParams>,
    ) -> Result<CallToolResult, McpError> {
        let session = self.session().await?;
        let budget = timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(WAIT_FOR_DEFAULT_BUDGET);
        match session.wait_for(&text, budget).await {
            Ok(snapshot) => Ok(snapshot_result(&snapshot)),
            Err(error) => Ok(error_result(&error)),
        }
    }

    #[tool(description = "List the session's open pages")]
    async fn tabs_list(&self) -> Result<CallToolResult, McpError> {
        let session = self.session().await?;
        let mut text = String::new();
        for page in session.pages().await {
            text.push_str(&format!(
                "{} {}{}\n",
                page.id,
                if page.url.is_empty() {
                    "(unknown url)"
                } else {
                    &page.url
                },
                if page.active { " (active)" } else { "" }
            ));
        }
        if text.is_empty() {
            text.push_str("no pages open; navigate to open one\n");
        }
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }

    #[tool(description = "Make another open page the active page")]
    async fn tabs_select(
        &self,
        Parameters(PageParams { page_id }): Parameters<PageParams>,
    ) -> Result<CallToolResult, McpError> {
        let session = self.session().await?;
        // docs/tool-catalog.md §4: an unknown page id is invalid_params, not an
        // action failure.
        if !session
            .pages()
            .await
            .iter()
            .any(|page| page.id.as_str() == page_id)
        {
            return Err(invalid_params(format!("no open page with id '{page_id}'")));
        }
        match session.select_page(PageId::new(page_id)).await {
            Ok(snapshot) => Ok(snapshot_result(&snapshot)),
            Err(error) => Ok(error_result(&error)),
        }
    }

    #[tool(description = "Close one open page of this session")]
    async fn tabs_close(
        &self,
        Parameters(PageParams { page_id }): Parameters<PageParams>,
    ) -> Result<CallToolResult, McpError> {
        let session = self.session().await?;
        // docs/tool-catalog.md §4: a page id names an open page, so an unknown id is
        // invalid_params as in tabs_select, not an action failure.
        if !session
            .pages()
            .await
            .iter()
            .any(|page| page.id.as_str() == page_id)
        {
            return Err(invalid_params(format!("no open page with id '{page_id}'")));
        }
        match session.close_page(PageId::new(page_id)).await {
            Ok(confirmation) => Ok(CallToolResult::success(vec![ContentBlock::text(
                confirmation,
            )])),
            Err(error) => Ok(error_result(&error)),
        }
    }

    #[tool(description = "Set cookies on this session's browser context")]
    async fn set_cookies(
        &self,
        Parameters(CookiesParams { cookies }): Parameters<CookiesParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut mapped = Vec::with_capacity(cookies.len());
        for input in &cookies {
            mapped.push(Cookie::try_from(input)?);
        }
        let session = self.session().await?;
        match session.set_cookies(&mapped).await {
            Ok(()) => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "set {} cookie(s)",
                mapped.len()
            ))])),
            Err(error) => Ok(error_result(&error)),
        }
    }

    #[tool(description = "Close this session's pages and context")]
    async fn close_session(&self) -> Result<CallToolResult, McpError> {
        self.manager.close_session(&self.session_id).await;
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "closed session {}",
            self.session_id
        ))]))
    }
}

impl RutterMcp {
    /// Executes one action on this connection's session and maps the
    /// outcome onto a tool result (snapshot text, or the failure as
    /// `isError` with its hint).
    async fn run_action(&self, action: Action) -> Result<CallToolResult, McpError> {
        let session = self.session().await?;
        match session.execute(action, Origin::Agent).await {
            Ok(snapshot) => Ok(snapshot_result(&snapshot)),
            Err(error) => Ok(error_result(&error)),
        }
    }
}

#[tool_handler]
impl ServerHandler for RutterMcp {
    fn get_info(&self) -> ServerConfig {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        let mut info = ServerConfig::new(capabilities).with_instructions(
            "rutter exposes a supervised browser. Start with navigate, \
             read snapshots, and act through snapshot references (e17).",
        );
        let mut implementation = Implementation::default();
        implementation.name = "rutter".to_owned();
        implementation.title = Some("rutter".to_owned());
        implementation.version = env!("CARGO_PKG_VERSION").to_owned();
        info.server_info = implementation;
        info
    }
}

/// Snapshot text plus the truncation marker of docs/tool-catalog.md §2.
fn snapshot_result(snapshot: &rutter_core::snapshot::Snapshot) -> CallToolResult {
    let mut text = snapshot.to_string();
    if snapshot.truncated {
        text.push_str("… truncated\n");
    }
    CallToolResult::success(vec![ContentBlock::text(text)])
}

/// An action failure as a result: message plus hint (docs/tool-catalog.md §2).
fn error_result(error: &SessionError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!(
        "{error}\nhint: {}",
        error.hint()
    ))])
}

fn invalid_params(message: String) -> McpError {
    McpError::invalid_params(message, None)
}

fn protocol_error(error: SessionError) -> McpError {
    // docs/tool-catalog.md §2: protocol-level failures carry the same
    // message-plus-hint text as isError results.
    McpError::new(
        ErrorCode(SERVER_ERROR_CODE),
        format!("{error}\nhint: {}", error.hint()),
        None,
    )
}

#[cfg(test)]
mod tests;
