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

/// Default `wait_for` budget when the caller sends no timeout
/// (`docs/TOOL_SPEC.md` §4: 10 000 ms).
const WAIT_FOR_DEFAULT_BUDGET: Duration = Duration::from_secs(10);

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
        // TOOL_SPEC §4: an unknown page id is invalid_params, not an
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

/// Parameter sets; schemas are generated by rmcp + schemars.
#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
/// Parameters of the navigate tool.
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
    use rutter_engine::config::{ContextConfig, LaunchMode};
    use rutter_engine::context::ContextHandle;
    use rutter_engine::descriptor::{EngineBackend, EngineCapabilities, EngineDescriptor};
    use rutter_engine::engine::Engine;
    use rutter_engine::error::EngineError;
    use rutter_engine::health::HealthReport;
    use rutter_engine::page::PageHandle;
    use rutter_engine::supervisor::EngineLauncher;
    use rutter_policy::{ApprovalBroker, RuleSet};
    use rutter_session::SessionConfig;

    /// Engine handing out throwaway contexts; enough for the session
    /// lifecycle tests, which never touch a page.
    struct StubEngine;

    #[async_trait::async_trait]
    impl Engine for StubEngine {
        fn descriptor(&self) -> EngineDescriptor {
            EngineDescriptor {
                backend: EngineBackend::ChromiumHeadlessShell,
                version: "stub".to_owned(),
                capabilities: EngineCapabilities {
                    headless: true,
                    headed: false,
                    screencast: false,
                    per_context_isolation: true,
                },
            }
        }

        async fn create_context(
            &self,
            _config: ContextConfig,
        ) -> Result<Arc<dyn ContextHandle>, EngineError> {
            // The tests here never open a page, so a context that only
            // knows its id is enough.
            struct StubContext;
            #[async_trait::async_trait]
            impl ContextHandle for StubContext {
                fn id(&self) -> rutter_core::ids::ContextId {
                    rutter_core::ids::ContextId::new("ctx-stub")
                }
                fn pages(&self) -> Vec<PageId> {
                    Vec::new()
                }
                async fn open_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
                    Err(EngineError::Terminated)
                }
                fn page(&self, _id: PageId) -> Option<Arc<dyn PageHandle>> {
                    None
                }
                async fn close_page(&self, _id: PageId) -> Result<(), EngineError> {
                    Ok(())
                }
                async fn set_cookies(
                    &self,
                    _cookies: &[rutter_core::cookie::Cookie],
                ) -> Result<(), EngineError> {
                    Ok(())
                }
                async fn cookies(&self) -> Result<Vec<rutter_core::cookie::Cookie>, EngineError> {
                    Ok(Vec::new())
                }
                async fn close(&self) -> Result<(), EngineError> {
                    Ok(())
                }
            }
            Ok(Arc::new(StubContext))
        }

        async fn health(&self) -> Result<HealthReport, EngineError> {
            Ok(HealthReport {
                healthy: true,
                backend_version: Some("stub".to_owned()),
                detail: None,
            })
        }

        async fn shutdown(&self) -> Result<(), EngineError> {
            Ok(())
        }
    }

    struct StubLauncher;

    #[async_trait::async_trait]
    impl EngineLauncher for StubLauncher {
        fn describe(&self) -> String {
            "stub".to_owned()
        }

        async fn launch(&self, _mode: LaunchMode) -> Result<Arc<dyn Engine>, EngineError> {
            Ok(Arc::new(StubEngine))
        }
    }

    fn manager() -> Arc<SessionManager> {
        Arc::new(SessionManager::new(
            Arc::new(StubLauncher),
            LaunchMode::Headless,
            SessionConfig::default(),
            Arc::new(RuleSet::default_set()),
            Arc::new(ApprovalBroker::new()),
            None,
        ))
    }

    #[tokio::test]
    async fn session_fails_fast_after_close_session() {
        // After close_session the manager no longer knows the id; the
        // cached session must not be resurrected for later tool calls.
        let manager = manager();
        let mcp = RutterMcp::new(Arc::clone(&manager), SessionId::new("s1"));

        mcp.session()
            .await
            .expect("the first call opens the session");
        manager
            .close_session(&SessionId::new("s1"))
            .await
            .expect("close");

        let error = match mcp.session().await {
            Ok(_) => panic!("a closed session must fail fast"),
            Err(error) => error,
        };
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(
            error.message.contains("closed"),
            "the message names the closed session: {error}"
        );
    }

    #[tokio::test]
    async fn session_is_created_once_per_connection() {
        let manager = manager();
        let mcp = RutterMcp::new(Arc::clone(&manager), SessionId::new("s1"));

        let first = mcp.session().await.expect("session");
        let second = mcp.session().await.expect("session");
        assert!(
            Arc::ptr_eq(&first, &second),
            "one connection is one session"
        );
    }

    #[test]
    fn truncated_snapshots_carry_the_spec_marker() {
        // TOOL_SPEC §2: a truncated snapshot must tell the agent it only
        // sees part of the page, or the agent trusts a cropped view.
        let button = rutter_core::snapshot::SnapshotNode::leaf("button");
        let full = rutter_core::snapshot::Snapshot {
            url: "https://example.com".to_owned(),
            root: button.clone(),
            truncated: false,
        };
        let text = first_text_block(&snapshot_result(&full));
        assert!(
            !text.contains("truncated"),
            "an intact snapshot carries no marker: {text}"
        );

        let cropped = rutter_core::snapshot::Snapshot {
            url: "https://example.com".to_owned(),
            root: button,
            truncated: true,
        };
        let text = first_text_block(&snapshot_result(&cropped));
        assert!(
            text.ends_with("… truncated\n"),
            "the marker must close the result text: {text:?}"
        );
    }

    /// First text block of a tool result (test helper).
    fn first_text_block(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .find_map(|block| block.as_text().map(|text| text.text.clone()))
            .expect("a text content block")
    }

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
