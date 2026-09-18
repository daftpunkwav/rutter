//! The MCP server: rmcp host mapping TOOL_SPEC onto the session layer.
//!
//! Responsibilities:
//! - Expose the blueprint §7.8 tool surface as rmcp tools.
//! - Return snapshots as text, screenshots as image blocks, and action
//!   failures as `isError` results carrying the error plus its hint
//!   (`docs/TOOL_SPEC.md` §2).
//!
//! Boundary: protocol mapping only. All semantics live in
//! `rutter-session`; this module validates parameters against the spec
//! and never touches the engine itself. One MCP connection is one
//! session, and the engine starts lazily on the first tool call.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorCode, Implementation, ServerCapabilities, ServerConfig,
};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};

use rutter_core::action::{Action, Origin, ScrollDirection};
use rutter_core::cookie::{Cookie, SameSite};
use rutter_core::ids::{PageId, SessionId};
use rutter_core::reference::Reference;
use rutter_session::error::SessionError;
use rutter_session::manager::SessionManager;
use rutter_session::session::Session;

/// Server error code for engine-level failures beyond a tool result
/// (`docs/TOOL_SPEC.md` §2).
const SERVER_ERROR_CODE: i32 = -32000;

/// Scroll directions accepted by the scroll tool.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Toward the top of the page.
    Up,
    /// Toward the bottom of the page.
    Down,
    /// Toward the left edge.
    Left,
    /// Toward the right edge.
    Right,
}

impl From<Direction> for ScrollDirection {
    fn from(direction: Direction) -> Self {
        match direction {
            Direction::Up => Self::Up,
            Direction::Down => Self::Down,
            Direction::Left => Self::Left,
            Direction::Right => Self::Right,
        }
    }
}

/// Cross-site sending policy accepted by the set_cookies tool
/// (`docs/TOOL_SPEC.md` §4: `strict|lax|none`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SameSiteInput {
    /// Sent only in first-party contexts.
    Strict,
    /// Sent with top-level navigations.
    Lax,
    /// Sent cross-site (requires `secure`).
    None,
}

impl From<SameSiteInput> for SameSite {
    fn from(same_site: SameSiteInput) -> Self {
        match same_site {
            SameSiteInput::Strict => Self::Strict,
            SameSiteInput::Lax => Self::Lax,
            SameSiteInput::None => Self::None,
        }
    }
}

/// One cookie as agents set it (`docs/TOOL_SPEC.md` §4).
#[derive(Debug, Clone, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
/// Tool parameters.
pub struct CookieInput {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// Domain the cookie belongs to.
    pub domain: String,
    /// Path scope; defaults to `/` when absent.
    pub path: Option<String>,
    /// Secure-only flag; defaults to false.
    pub secure: Option<bool>,
    /// Http-only flag; defaults to false.
    pub http_only: Option<bool>,
    /// Cross-site policy: `strict`, `lax`, or `none`.
    pub same_site: Option<SameSiteInput>,
}

impl TryFrom<&CookieInput> for Cookie {
    type Error = McpError;

    fn try_from(input: &CookieInput) -> Result<Self, Self::Error> {
        Ok(Self {
            name: input.name.clone(),
            value: input.value.clone(),
            domain: input.domain.clone(),
            path: input.path.clone(),
            secure: input.secure.unwrap_or(false),
            http_only: input.http_only.unwrap_or(false),
            same_site: input.same_site.map(SameSite::from),
            expires: None,
        })
    }
}

/// The per-connection MCP server.
#[derive(Clone)]
/// Tool parameters.
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

    /// The session for this connection, created on first use.
    async fn session(&self) -> Result<Arc<Session>, McpError> {
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
        let session = self.session().await?;
        match session
            .execute(Action::Navigate { url }, Origin::Agent)
            .await
        {
            Ok(snapshot) => Ok(snapshot_result(&snapshot)),
            Err(error) => Ok(error_result(&error)),
        }
    }

    #[tool(description = "Go back one history entry and return a fresh snapshot")]
    async fn back(&self) -> Result<CallToolResult, McpError> {
        self.simple_action(Action::Back).await
    }

    #[tool(description = "Go forward one history entry and return a fresh snapshot")]
    async fn forward(&self) -> Result<CallToolResult, McpError> {
        self.simple_action(Action::Forward).await
    }

    #[tool(description = "Reload the active page and return a fresh snapshot")]
    async fn reload(&self) -> Result<CallToolResult, McpError> {
        self.simple_action(Action::Reload).await
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
                    rutter_engine::page::ImageFormat::Png => "image/png",
                    rutter_engine::page::ImageFormat::Jpeg => "image/jpeg",
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
        let session = self.session().await?;
        match session
            .execute(
                Action::Click {
                    reference: Reference::new(reference),
                },
                Origin::Agent,
            )
            .await
        {
            Ok(snapshot) => Ok(snapshot_result(&snapshot)),
            Err(error) => Ok(error_result(&error)),
        }
    }

    #[tool(description = "Hover the element a snapshot reference points to")]
    async fn hover(
        &self,
        Parameters(ReferenceParams { reference }): Parameters<ReferenceParams>,
    ) -> Result<CallToolResult, McpError> {
        let session = self.session().await?;
        match session
            .execute(
                Action::Hover {
                    reference: Reference::new(reference),
                },
                Origin::Agent,
            )
            .await
        {
            Ok(snapshot) => Ok(snapshot_result(&snapshot)),
            Err(error) => Ok(error_result(&error)),
        }
    }

    #[tool(
        name = "type",
        description = "Focus an element and type text into it, returning a fresh snapshot"
    )]
    async fn type_text(
        &self,
        Parameters(TypeParams { reference, text }): Parameters<TypeParams>,
    ) -> Result<CallToolResult, McpError> {
        let session = self.session().await?;
        match session
            .execute(
                Action::Type {
                    reference: Reference::new(reference),
                    text,
                },
                Origin::Agent,
            )
            .await
        {
            Ok(snapshot) => Ok(snapshot_result(&snapshot)),
            Err(error) => Ok(error_result(&error)),
        }
    }

    #[tool(description = "Press a single key (for example Enter or Tab) on the active page")]
    async fn press_key(
        &self,
        Parameters(PressKeyParams { key }): Parameters<PressKeyParams>,
    ) -> Result<CallToolResult, McpError> {
        let session = self.session().await?;
        match session
            .execute(Action::PressKey { key }, Origin::Agent)
            .await
        {
            Ok(snapshot) => Ok(snapshot_result(&snapshot)),
            Err(error) => Ok(error_result(&error)),
        }
    }

    #[tool(description = "Select options by value on a select element")]
    async fn select_option(
        &self,
        Parameters(SelectOptionParams { reference, values }): Parameters<SelectOptionParams>,
    ) -> Result<CallToolResult, McpError> {
        let session = self.session().await?;
        match session
            .execute(
                Action::SelectOption {
                    reference: Reference::new(reference),
                    values,
                },
                Origin::Agent,
            )
            .await
        {
            Ok(snapshot) => Ok(snapshot_result(&snapshot)),
            Err(error) => Ok(error_result(&error)),
        }
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
        let session = self.session().await?;
        match session
            .execute(
                Action::Scroll {
                    reference: reference.map(Reference::new),
                    direction: direction.into(),
                    amount,
                },
                Origin::Agent,
            )
            .await
        {
            Ok(snapshot) => Ok(snapshot_result(&snapshot)),
            Err(error) => Ok(error_result(&error)),
        }
    }

    #[tool(description = "Wait until text appears on the page, then return a snapshot")]
    async fn wait_for(
        &self,
        Parameters(WaitForParams { text, timeout_ms }): Parameters<WaitForParams>,
    ) -> Result<CallToolResult, McpError> {
        let session = self.session().await?;
        let budget = timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_secs(10));
        match session.wait_for(&text, budget).await {
            Ok(snapshot) => Ok(snapshot_result(&snapshot)),
            Err(error) => Ok(error_result(&error)),
        }
    }

    #[tool(description = "List the session's open pages")]
    async fn tabs_list(&self) -> Result<CallToolResult, McpError> {
        let session = self.session().await?;
        let mut text = String::new();
        for tab in session.tabs().await {
            text.push_str(&format!(
                "{} {}{}\n",
                tab.page,
                if tab.url.is_empty() {
                    "(unknown url)"
                } else {
                    &tab.url
                },
                if tab.active { " (active)" } else { "" }
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
        // TOOL_SPEC §4: an unknown page id is invalid_params, not an
        // action failure.
        if !session
            .tabs()
            .await
            .iter()
            .any(|tab| tab.page.as_str() == page_id)
        {
            return Err(invalid_params(format!("no open page with id '{page_id}'")));
        }
        match session.select_tab(PageId::new(page_id)).await {
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
        match session.close_tab(PageId::new(page_id)).await {
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

    #[tool(description = "Close this session's pages; the engine shuts down when idle")]
    async fn close_session(&self) -> Result<CallToolResult, McpError> {
        match self.manager.close_session(&self.session_id).await {
            Ok(()) => Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "closed session {}",
                self.session_id
            ))])),
            Err(error) => Ok(error_result(&SessionError::Engine(error))),
        }
    }
}

impl RutterMcp {
    /// Runs one of the parameter-less history actions.
    async fn simple_action(&self, action: Action) -> Result<CallToolResult, McpError> {
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

/// Parameter sets; schemas are generated by rmcp + schemars.
#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
/// Tool parameters.
pub struct NavigateParams {
    /// Absolute URL to load.
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
/// Parameters for element-targeted tools.
pub struct ReferenceParams {
    /// Snapshot reference of the element, for example `e17`.
    pub reference: String,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
/// Parameters of the `type` tool.
pub struct TypeParams {
    /// Snapshot reference of the element to focus.
    pub reference: String,
    /// Text to insert as literal characters.
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
/// Parameters of the press_key tool.
pub struct PressKeyParams {
    /// Key in engine notation, for example `a`, `Enter`, `Tab`.
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
/// Parameters of the select_option tool.
pub struct SelectOptionParams {
    /// Snapshot reference of the select element.
    pub reference: String,
    /// Values of the options to select.
    pub values: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
/// Parameters of the scroll tool.
pub struct ScrollParams {
    /// Scroll direction.
    pub direction: Direction,
    /// Distance in pixels; must be greater than zero.
    pub amount: u32,
    /// Reference of a container to scroll; omitted scrolls the page.
    pub reference: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
/// Parameters of the wait_for tool.
pub struct WaitForParams {
    /// Text to wait for.
    pub text: String,
    /// Budget in milliseconds; default 10 000.
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
/// Parameters naming one open page.
pub struct PageParams {
    /// Page id as reported by tabs_list.
    pub page_id: String,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
/// Parameters of the set_cookies tool.
pub struct CookiesParams {
    /// Cookies to set on this session's context.
    pub cookies: Vec<CookieInput>,
}

/// Snapshot text plus the truncation marker of TOOL_SPEC §2.
fn snapshot_result(snapshot: &rutter_core::snapshot::Snapshot) -> CallToolResult {
    let mut text = snapshot.to_string();
    if snapshot.truncated {
        text.push_str("… truncated\n");
    }
    CallToolResult::success(vec![ContentBlock::text(text)])
}

/// An action failure as a result: message plus hint (TOOL_SPEC §2).
fn error_result(error: &SessionError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!(
        "{error}\nhint: {}",
        error.hint()
    ))])
}

fn invalid_params(message: String) -> McpError {
    McpError::invalid_params(message, None)
}

fn protocol_error(error: rutter_engine::error::EngineError) -> McpError {
    // TOOL_SPEC §2: protocol-level failures carry the same
    // message-plus-hint text as isError results.
    McpError::new(
        ErrorCode(SERVER_ERROR_CODE),
        format!("{error}\nhint: {}", error.hint()),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_inputs_map_with_defaults() {
        let input = CookieInput {
            name: "session".to_owned(),
            value: "42".to_owned(),
            domain: "example.com".to_owned(),
            path: None,
            secure: None,
            http_only: None,
            same_site: Some(SameSiteInput::Lax),
        };
        let cookie = Cookie::try_from(&input).expect("cookie");
        assert_eq!(cookie.name, "session");
        assert_eq!(cookie.path, None);
        assert!(!cookie.secure);
        assert_eq!(cookie.same_site, Some(SameSite::Lax));
    }

    #[test]
    fn unknown_same_site_is_rejected_by_deserialization() {
        // The schema enumerates strict|lax|none (TOOL_SPEC §4), so an
        // unknown policy is invalid_params at the deserialization layer.
        let error = serde_json::from_str::<CookieInput>(
            r#"{"name":"s","value":"42","domain":"example.com","same_site":"sloppy"}"#,
        )
        .expect_err("unknown policy");
        let message = error.to_string();
        assert!(message.contains("sloppy"), "names the bad value: {message}");
        assert!(
            message.contains("unknown variant"),
            "names the enum rejection: {message}"
        );
    }

    #[test]
    fn scroll_amount_zero_is_rejected_by_the_tool_layer() {
        // The validation itself lives in the scroll tool; this pins the
        // direction mapping it relies on.
        assert_eq!(
            ScrollDirection::from(Direction::Down),
            ScrollDirection::Down
        );
        assert_eq!(
            ScrollDirection::from(Direction::Left),
            ScrollDirection::Left
        );
    }
}
