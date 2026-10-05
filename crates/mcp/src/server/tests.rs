//! Server tests: session lifecycle over a stub engine, result
//! mapping (truncation marker), and tool handlers over a scripted page.

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
    manager.close_session(&SessionId::new("s1")).await;

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
async fn a_close_landing_during_init_reports_the_closed_session() {
    // The close can land between the manager inserting the session and
    // the connection latching it: initialization returns the `Arc` of a
    // session that is already gone. Handing that exact state to
    // `session()` — the cell holds what initialization produced, the
    // manager map no longer knows the id — must answer the
    // closed-session error, not hand out the dead handle.
    let manager = manager();
    let mcp = RutterMcp::new(Arc::clone(&manager), SessionId::new("s1"));
    let id = SessionId::new("s1");
    let created = manager.session(id.clone()).await.expect("session");
    // `set` only fails when the cell is already full, which cannot
    // happen on a fresh connection; the error half carries no `Debug`.
    assert!(mcp.session.set(created).is_ok());
    manager.close_session(&id).await;

    let error = match mcp.session().await {
        Ok(_) => panic!("a session closed during init must fail fast"),
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
    // A truncated snapshot must tell the agent it only sees part of
    // the page, or the agent trusts a cropped view.
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

#[test]
fn truncated_readouts_carry_the_spec_marker() {
    // Same contract as snapshots: a cropped
    // readout must say so, or the agent trusts a partial document.
    let full = rutter_core::readout::Readout {
        title: "Example".to_owned(),
        markdown: "# Example".to_owned(),
        truncated: false,
    };
    let text = first_text_block(&read_result(&full));
    assert!(
        !text.contains("truncated"),
        "an intact readout carries no marker: {text}"
    );

    let cropped = rutter_core::readout::Readout {
        title: "Example".to_owned(),
        markdown: "# Example".to_owned(),
        truncated: true,
    };
    let text = first_text_block(&read_result(&cropped));
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

#[tokio::test]
async fn tabs_close_unknown_page_is_invalid_params() {
    // A page id names an open page, so an unknown id is invalid_params
    // as in tabs_select — not an action failure whose
    // hint would tell the agent to re-snapshot and pick an element.
    let mcp = RutterMcp::new(manager(), SessionId::new("s1"));
    let error = match mcp
        .tabs_close(Parameters(PageParams {
            page_id: "p99".to_owned(),
        }))
        .await
    {
        Ok(_) => panic!("an unknown page must not close"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(error.message.contains("p99"), "names the id: {error}");
}

#[tokio::test]
async fn scroll_zero_and_negatives_are_invalid_params() {
    // `amount` <= 0 is invalid_params — one rejection path for both.
    // The u32 schema used to send negatives
    // through rmcp's deserializer instead, whose error text carries no
    // `hint:` line; signed now keeps them on this check.
    let mcp = RutterMcp::new(manager(), SessionId::new("s1"));
    for amount in [0, -1] {
        let error = match mcp
            .scroll(Parameters(ScrollParams {
                direction: ScrollDirectionInput::Down,
                amount,
                reference: None,
            }))
            .await
        {
            Ok(_) => panic!("a non-positive scroll must be refused"),
            Err(error) => error,
        };
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(
            error.message.contains("greater than zero"),
            "names the rule: {error}"
        );
    }
    // Beyond the u32 the action vocabulary accepts, the refusal names
    // the bound instead of silently truncating.
    let error = match mcp
        .scroll(Parameters(ScrollParams {
            direction: ScrollDirectionInput::Down,
            amount: i64::MAX,
            reference: None,
        }))
        .await
    {
        Ok(_) => panic!("an absurd scroll must be refused"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(
        error.message.contains("must not exceed"),
        "names the bound: {error}"
    );
}

#[tokio::test]
async fn close_session_is_terminal_for_the_connection() {
    // Later tool calls on a closed connection fail with invalid_params
    // naming the closed session — including a
    // repeated close, which used to answer success twice.
    let mcp = RutterMcp::new(manager(), SessionId::new("s1"));
    // Open the session first: the terminality gate guards connections
    // that had one, and a listing is the cheapest way to create it.
    mcp.tabs_list().await.expect("listing opens the session");
    mcp.close_session()
        .await
        .expect("the first close closes the session");

    let error = match mcp.close_session().await {
        Ok(_) => panic!("a closed connection cannot close again"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(
        error.message.contains("s1") && error.message.contains("closed"),
        "names the closed session: {error}"
    );
}

#[tokio::test]
async fn close_session_on_an_unused_connection_creates_nothing() {
    // The closed-session gate must not lazily initialize the session:
    // for a connection that never ran a tool, closing is the manager's
    // no-op — not an engine launch immediately torn down. The connection
    // stays uninitialized, so a later close is the same no-op.
    let manager = manager();
    let mcp = RutterMcp::new(Arc::clone(&manager), SessionId::new("s-never"));
    mcp.close_session()
        .await
        .expect("closing an unused connection succeeds as a no-op");
    mcp.close_session()
        .await
        .expect("still a no-op: no session exists to be terminal about");
    assert!(
        manager.session_ids().await.is_empty(),
        "no session was created just to be closed"
    );
}

#[tokio::test]
async fn protocol_failures_surface_as_server_errors_carrying_hints() {
    // Protocol-level failures are JSON-RPC errors with code -32000,
    // carrying the same message-plus-hint text an isError
    // result would. The wiring is exercised for real through
    // `session()`: a zero-session cap fails the first session request
    // after the stub engine starts, with no engine I/O.
    let manager = Arc::new(SessionManager::new(
        Arc::new(StubLauncher),
        LaunchMode::Headless,
        SessionConfig {
            max_sessions: 0,
            ..SessionConfig::default()
        },
        Arc::new(RuleSet::default_set()),
        Arc::new(ApprovalBroker::new()),
        None,
    ));
    let mcp = RutterMcp::new(manager, SessionId::new("s1"));

    let error = match mcp.session().await {
        Ok(_) => panic!("no session can open"),
        Err(error) => error,
    };
    // The literal, not the constant: the spec pins -32000, and a drift
    // in SERVER_ERROR_CODE must fail this assertion, not move with it.
    assert_eq!(error.code.0, -32000, "-32000 is the spec code");
    assert!(
        error.message.contains("capacity exceeded"),
        "the message carries the failure: {error}"
    );
    assert!(
        error.message.contains("\nhint: "),
        "the hint rides on its own line: {error}"
    );
}

#[test]
fn action_failures_carry_the_message_plus_hint_contract() {
    // an action failure is an isError result whose text holds the error
    // message and, on its own second line, the
    // actionable hint (`hint: …`). Agents read the hint as data, so a
    // merged or dropped hint line breaks them silently.
    let error = SessionError::Action(rutter_core::error::ActionError::Internal {
        detail: "capture failed".to_owned(),
    });
    let text = first_text_block(&error_result(&error));
    let (message, hint) = text
        .split_once("\nhint: ")
        .expect("the hint rides on its own line");
    assert!(
        message.contains("capture failed"),
        "the message names the failure: {message}"
    );
    assert!(!hint.trim().is_empty(), "the hint is not empty: {hint}");
}

#[test]
fn every_agent_visible_failure_carries_the_message_plus_hint_contract() {
    // The tool boundary renders whatever `SessionError` it is handed, and
    // that error is not always an `ActionError`: an engine-layer failure
    // (a navigation that could not be sent, a dead engine, a page cap)
    // keeps the engine's own classification and its own hint.
    // `docs/tool-catalog.md` §2 documents both shapes, so what must hold
    // for all of them — the part an agent parses — is the envelope: the
    // message first, the hint on its own `hint: ` line, never empty. If
    // this test fails, some failure stopped speaking the contract; the
    // fix belongs at the boundary, not in the docs.
    let session = SessionId::new("s1");
    let reference = rutter_core::reference::Reference::new("e17");
    let failures: Vec<SessionError> = vec![
        // The action taxonomy, as every action failure reports it.
        SessionError::Action(rutter_core::error::ActionError::NavigationFailed {
            url: "https://example.com".to_owned(),
            cause: rutter_core::error::TransportCause::DnsFailed,
        }),
        SessionError::Action(rutter_core::error::ActionError::ReferenceExpired {
            reference: reference.clone(),
        }),
        SessionError::Action(rutter_core::error::ActionError::NotInteractable {
            reference: reference.clone(),
            reason: "covered by another element".to_owned(),
        }),
        SessionError::Action(rutter_core::error::ActionError::TimedOut {
            phase: rutter_core::error::WaitPhase::Visible,
            elapsed: std::time::Duration::from_secs(5),
        }),
        SessionError::Action(rutter_core::error::ActionError::ApprovalDenied {
            reference: reference.clone(),
        }),
        SessionError::Action(rutter_core::error::ActionError::ApprovalTimedOut {
            waited: std::time::Duration::from_secs(120),
        }),
        SessionError::Action(rutter_core::error::ActionError::EngineTerminated {
            session: session.clone(),
        }),
        SessionError::Action(rutter_core::error::ActionError::Internal {
            detail: "unreachable state".to_owned(),
        }),
        // The engine layer, which an action surfaces unchanged.
        SessionError::Engine(EngineError::NavigationFailed {
            url: "https://example.com".to_owned(),
            cause: rutter_core::error::TransportCause::DnsFailed,
            detail: "dns error".to_owned(),
        }),
        SessionError::Engine(EngineError::Terminated),
        SessionError::Engine(EngineError::Timeout {
            operation: "navigate".to_owned(),
            elapsed: std::time::Duration::from_secs(30),
        }),
        SessionError::Engine(EngineError::Capacity {
            detail: "page cap reached".to_owned(),
        }),
        SessionError::Engine(EngineError::Internal {
            detail: "capture failed".to_owned(),
        }),
        // The session layer's own answers.
        SessionError::Capacity {
            detail: "session cap reached".to_owned(),
        },
        SessionError::NoOpenPage,
        SessionError::Internal {
            detail: "unreachable state".to_owned(),
        },
    ];

    for failure in failures {
        let result = error_result(&failure);
        assert!(is_error(&result), "reported as an isError: {failure}");
        let text = first_text_block(&result);
        let (message, hint) = text
            .split_once("\nhint: ")
            .unwrap_or_else(|| panic!("the hint rides on its own line: {text}"));
        assert!(!message.is_empty(), "the message is not empty: {failure}");
        assert!(
            !hint.trim().is_empty(),
            "every failure carries an actionable hint: {failure}"
        );
        // The same rendering whether the failure arrives as a tool result
        // or as a protocol error: one envelope, one contract.
        let protocol = protocol_error(failure);
        assert!(
            protocol.message.contains(&text),
            "protocol errors carry the same message-plus-hint text"
        );
    }
}

// ---------------------------------------------------------------------
// Tool handlers over a scripted page: the bodies map results and
// failures; the stub engine above (no pages) only reaches the failure
// arms. The page double answers the observe-script markers, mirroring
// `crates/session/tests/common`.
// ---------------------------------------------------------------------

use rutter_engine::input::InputEvent;
use rutter_engine::page::{ImageFormat, ScreencastStream, Screenshot};
use serde_json::{Value, json};

struct ScriptedPage {
    url: std::sync::Mutex<String>,
    found: std::sync::atomic::AtomicBool,
    /// Screenshot formats handed out in order; the last one repeats.
    formats: std::sync::Mutex<Vec<ImageFormat>>,
    inputs: std::sync::Mutex<Vec<InputEvent>>,
}

#[async_trait::async_trait]
impl PageHandle for ScriptedPage {
    async fn navigate(&self, url: &str) -> Result<String, EngineError> {
        *self.url.lock().unwrap() = url.to_owned();
        Ok(url.to_owned())
    }

    async fn reload(&self) -> Result<(), EngineError> {
        Ok(())
    }

    async fn go_back(&self) -> Result<String, EngineError> {
        Ok(self.url.lock().unwrap().clone())
    }

    async fn go_forward(&self) -> Result<String, EngineError> {
        Ok(self.url.lock().unwrap().clone())
    }

    async fn evaluate(&self, expression: &str) -> Result<Value, EngineError> {
        if expression.contains("var NEEDLE = ") {
            Ok(json!({
                "found": self.found.load(std::sync::atomic::Ordering::SeqCst)
            }))
        } else if expression.contains("var REF = ") && expression.contains("var VALUES = ") {
            // The select script answers with the outcome object.
            Ok(json!({
                "missing": false,
                "not_select": false,
                "matched": 1,
            }))
        } else if expression.contains("var REF = ") {
            // The resolver/focus scripts answer with the element box:
            // visible, stable, and enabled, so auto-wait settles at once.
            Ok(json!({
                "missing": false,
                "x": 10.0,
                "y": 20.0,
                "width": 100.0,
                "height": 30.0,
                "disabled": false,
                "hidden": false,
            }))
        } else if expression.contains("MAX_NODES") {
            let url = self.url.lock().unwrap().clone();
            Ok(json!({
                "version": 1,
                "truncated": false,
                "root": {
                    "role": "button",
                    "name": "Ok",
                    "ref": "e1",
                    "children": [],
                },
                "url": url,
            }))
        } else if expression.contains("location.href") {
            Ok(json!(self.url.lock().unwrap().clone()))
        } else {
            Ok(Value::Null)
        }
    }

    async fn dispatch_input(&self, event: InputEvent) -> Result<(), EngineError> {
        self.inputs.lock().unwrap().push(event);
        Ok(())
    }

    async fn capture_screenshot(&self) -> Result<Screenshot, EngineError> {
        let mut formats = self.formats.lock().unwrap();
        let format = formats.first().copied().unwrap_or(ImageFormat::Png);
        if formats.len() > 1 {
            formats.remove(0);
        }
        Ok(Screenshot {
            format,
            data: vec![7; 64],
        })
    }

    async fn start_screencast(&self) -> Result<ScreencastStream, EngineError> {
        let (_sender, receiver) = tokio::sync::mpsc::channel(1);
        Ok(ScreencastStream::new(receiver))
    }
}

struct ScriptedContext {
    page: Arc<ScriptedPage>,
    /// When true, `set_cookies` fails (the tool's error arm).
    fail_cookies: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl ContextHandle for ScriptedContext {
    fn id(&self) -> rutter_core::ids::ContextId {
        rutter_core::ids::ContextId::new("ctx-scripted")
    }

    fn pages(&self) -> Vec<PageId> {
        vec![PageId::new("ctx-scripted:page-0")]
    }

    async fn open_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
        Ok((
            PageId::new("ctx-scripted:page-0"),
            Arc::clone(&self.page) as Arc<dyn PageHandle>,
        ))
    }

    fn page(&self, _id: PageId) -> Option<Arc<dyn PageHandle>> {
        Some(Arc::clone(&self.page) as Arc<dyn PageHandle>)
    }

    async fn close_page(&self, _id: PageId) -> Result<(), EngineError> {
        Ok(())
    }

    async fn set_cookies(&self, cookies: &[Cookie]) -> Result<(), EngineError> {
        if self.fail_cookies.load(std::sync::atomic::Ordering::SeqCst)
            || cookies.iter().any(|cookie| cookie.name == "boom")
        {
            return Err(EngineError::Unsupported {
                operation: "set_cookies".to_owned(),
                reason: "scripted failure".to_owned(),
            });
        }
        Ok(())
    }

    async fn cookies(&self) -> Result<Vec<Cookie>, EngineError> {
        Ok(Vec::new())
    }

    async fn close(&self) -> Result<(), EngineError> {
        Ok(())
    }
}

struct ScriptedEngine;

#[async_trait::async_trait]
impl Engine for ScriptedEngine {
    fn descriptor(&self) -> EngineDescriptor {
        EngineDescriptor {
            backend: EngineBackend::ChromiumHeadlessShell,
            version: "scripted".to_owned(),
            capabilities: EngineCapabilities {
                headless: true,
                headed: false,
                screencast: true,
                per_context_isolation: true,
            },
        }
    }

    async fn create_context(
        &self,
        _config: ContextConfig,
    ) -> Result<Arc<dyn ContextHandle>, EngineError> {
        Ok(Arc::new(ScriptedContext {
            page: Arc::new(ScriptedPage {
                url: std::sync::Mutex::new(String::new()),
                found: std::sync::atomic::AtomicBool::new(true),
                formats: std::sync::Mutex::new(vec![ImageFormat::Jpeg, ImageFormat::Png]),
                inputs: std::sync::Mutex::new(Vec::new()),
            }),
            fail_cookies: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    async fn health(&self) -> Result<HealthReport, EngineError> {
        Ok(HealthReport {
            healthy: true,
            backend_version: Some("scripted".to_owned()),
            detail: None,
        })
    }

    async fn shutdown(&self) -> Result<(), EngineError> {
        Ok(())
    }
}

struct ScriptedLauncher;

#[async_trait::async_trait]
impl EngineLauncher for ScriptedLauncher {
    fn describe(&self) -> String {
        "scripted".to_owned()
    }

    async fn launch(&self, _mode: LaunchMode) -> Result<Arc<dyn Engine>, EngineError> {
        Ok(Arc::new(ScriptedEngine))
    }
}

fn scripted_manager() -> Arc<SessionManager> {
    // Allow every action: these tests exercise the tool handlers, not
    // the approval flow (that pairing has its own tests).
    Arc::new(SessionManager::new(
        Arc::new(ScriptedLauncher),
        LaunchMode::Headless,
        SessionConfig::default(),
        Arc::new(RuleSet::new(Vec::new(), rutter_policy::Verdict::Allow)),
        Arc::new(ApprovalBroker::new()),
        None,
    ))
}

fn is_error(result: &CallToolResult) -> bool {
    result.is_error.unwrap_or(false)
}

#[tokio::test]
async fn action_tools_report_failures_as_iserror_results() {
    // With no page open every action fails; the handlers still answer,
    // with the failure as an isError result, not a protocol error.
    let mcp = RutterMcp::new(manager(), SessionId::new("s1"));
    for result in [
        mcp.back().await,
        mcp.forward().await,
        mcp.reload().await,
        mcp.press_key(Parameters(PressKeyParams {
            key: "Enter".to_owned(),
        }))
        .await,
        mcp.hover(Parameters(ReferenceParams {
            reference: "e1".to_owned(),
        }))
        .await,
        mcp.select_option(Parameters(SelectOptionParams {
            reference: "e1".to_owned(),
            values: vec!["a".to_owned()],
        }))
        .await,
        mcp.scroll(Parameters(ScrollParams {
            direction: ScrollDirectionInput::Down,
            amount: 120,
            reference: None,
        }))
        .await,
        mcp.snapshot().await,
    ] {
        let result = result.expect("a tool result, not a protocol error");
        assert!(is_error(&result), "no page means an isError result");
        let text = first_text_block(&result);
        assert!(
            text.contains("\nhint: "),
            "the failure carries its hint: {text}"
        );
    }
}

#[tokio::test]
async fn navigation_tools_return_snapshots_on_success() {
    let mcp = RutterMcp::new(scripted_manager(), SessionId::new("s1"));
    let navigate = mcp
        .navigate(Parameters(NavigateParams {
            url: "https://example.com".to_owned(),
        }))
        .await
        .expect("navigate result");
    assert!(!is_error(&navigate), "navigate succeeds");
    assert!(
        first_text_block(&navigate).contains("button \"Ok\""),
        "a fresh snapshot comes back: {}",
        first_text_block(&navigate)
    );

    // Reference-taking tools run against that fresh snapshot; each
    // returns a fresh snapshot of its own. History tools navigate, which
    // expires references, so they run after the reference-taking ones.
    for (tool, params) in [
        (
            "hover",
            mcp.hover(Parameters(ReferenceParams {
                reference: "e1".to_owned(),
            }))
            .await,
        ),
        (
            "select_option",
            mcp.select_option(Parameters(SelectOptionParams {
                reference: "e1".to_owned(),
                values: vec!["a".to_owned()],
            }))
            .await,
        ),
        (
            "scroll",
            mcp.scroll(Parameters(ScrollParams {
                direction: ScrollDirectionInput::Up,
                amount: 60,
                reference: Some("e1".to_owned()),
            }))
            .await,
        ),
        (
            "press_key",
            mcp.press_key(Parameters(PressKeyParams {
                key: "Tab".to_owned(),
            }))
            .await,
        ),
    ] {
        let result = params.expect("a tool result");
        assert!(!is_error(&result), "{tool} succeeds: {result:?}");
    }

    for result in [mcp.back().await, mcp.forward().await, mcp.reload().await] {
        let result = result.expect("a tool result");
        assert!(!is_error(&result), "history tools succeed: {result:?}");
    }
}

#[tokio::test]
async fn screenshot_returns_image_blocks_for_each_format() {
    let mcp = RutterMcp::new(scripted_manager(), SessionId::new("s1"));
    mcp.navigate(Parameters(NavigateParams {
        url: "https://example.com".to_owned(),
    }))
    .await
    .expect("navigate");

    // The scripted page hands out Jpeg first, then Png: both mime arms.
    for mime in ["image/jpeg", "image/png"] {
        let result = mcp.screenshot().await.expect("screenshot result");
        assert!(!is_error(&result));
        let block = result
            .content
            .iter()
            .find_map(|block| block.as_image().cloned())
            .expect("an image content block");
        assert_eq!(block.mime_type, mime);
    }
}

#[tokio::test]
async fn tabs_tools_list_select_and_close_open_pages() {
    let manager = scripted_manager();
    let mcp = RutterMcp::new(Arc::clone(&manager), SessionId::new("s1"));
    mcp.navigate(Parameters(NavigateParams {
        url: "https://a.example".to_owned(),
    }))
    .await
    .expect("a page");

    // The session's own page list (via the manager) is the source of ids.
    let session = manager
        .get_session(&SessionId::new("s1"))
        .await
        .expect("the session");
    let pages = session.pages().await;
    let page = pages.first().expect("the navigated page");
    assert!(page.active, "navigation leaves its page active");

    let listed = mcp.tabs_list().await.expect("tabs_list result");
    let text = first_text_block(&listed);
    assert!(text.contains("https://a.example"), "lists urls: {text}");
    assert!(text.contains("(active)"), "marks the active page: {text}");

    // Selecting the already-active page is still a valid select: the id
    // names an open page, so the tool maps the outcome, not the no-op.
    let selected = mcp
        .tabs_select(Parameters(PageParams {
            page_id: page.id.as_str().to_owned(),
        }))
        .await
        .expect("tabs_select result");
    assert!(!is_error(&selected), "selecting an open page succeeds");

    let closed = mcp
        .tabs_close(Parameters(PageParams {
            page_id: page.id.as_str().to_owned(),
        }))
        .await
        .expect("tabs_close result");
    let text = first_text_block(&closed);
    assert!(
        !text.contains("\nhint: "),
        "closing an open page confirms, not an error: {text}"
    );
}

#[tokio::test]
async fn tabs_list_with_no_pages_says_so() {
    let mcp = RutterMcp::new(manager(), SessionId::new("s1"));
    mcp.session().await.expect("the session opens");
    let listed = mcp.tabs_list().await.expect("tabs_list result");
    assert_eq!(
        first_text_block(&listed),
        "no pages open; navigate to open one\n"
    );
}

#[tokio::test]
async fn console_and_network_tools_name_the_active_page() {
    // No page tracked: the probe still answers, with no id to name.
    let pageless = RutterMcp::new(manager(), SessionId::new("s1"));
    let console = pageless.console_messages().await.expect("console result");
    assert_eq!(
        first_text_block(&console),
        "no console output on the active page\n"
    );
    let network = pageless.network_requests().await.expect("network result");
    assert_eq!(
        first_text_block(&network),
        "no network requests on the active page\n"
    );

    // With a tracked page, the output leads with the page's id — the
    // same id `tabs_list` reports.
    let script_manager = scripted_manager();
    let mcp = RutterMcp::new(Arc::clone(&script_manager), SessionId::new("s1"));
    mcp.navigate(Parameters(NavigateParams {
        url: "https://a.example".to_owned(),
    }))
    .await
    .expect("a page");
    let session = script_manager
        .get_session(&SessionId::new("s1"))
        .await
        .expect("the session");
    let page = session
        .pages()
        .await
        .into_iter()
        .find(|page| page.active)
        .expect("the navigated page")
        .id;
    let console = mcp.console_messages().await.expect("console result");
    let text = first_text_block(&console);
    assert!(
        text.starts_with(&format!("active page {page}\n")),
        "console output names the active page: {text}"
    );
    let network = mcp.network_requests().await.expect("network result");
    let text = first_text_block(&network);
    assert!(
        text.starts_with(&format!("active page {page}\n")),
        "network output names the active page: {text}"
    );
}

#[tokio::test]
async fn tabs_select_unknown_page_is_invalid_params() {
    // mirrors tabs_close — an unknown id is invalid_params, not an
    // action failure.
    let mcp = RutterMcp::new(scripted_manager(), SessionId::new("s1"));
    mcp.navigate(Parameters(NavigateParams {
        url: "https://example.com".to_owned(),
    }))
    .await
    .expect("a page");
    let error = match mcp
        .tabs_select(Parameters(PageParams {
            page_id: "p99".to_owned(),
        }))
        .await
    {
        Ok(_) => panic!("an unknown page must not be selected"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(error.message.contains("p99"), "names the id: {error}");
}

#[tokio::test]
async fn upload_file_with_no_paths_is_invalid_params() {
    // An empty `paths` names no file to set: the refusal is a
    // validation rejection on this crate's invalid_params path, checked
    // before the session gate, so no engine launches just to refuse.
    let mcp = RutterMcp::new(manager(), SessionId::new("s1"));
    let error = match mcp
        .upload_file(Parameters(UploadFileParams {
            reference: "e1".to_owned(),
            paths: Vec::new(),
        }))
        .await
    {
        Ok(_) => panic!("an empty paths list must be refused"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(
        error.message.contains("must not be empty"),
        "names the rule: {error}"
    );
}

#[tokio::test]
async fn press_key_with_an_empty_key_is_invalid_params() {
    // An empty key names no key: the engine would dispatch a name-only
    // event that lands nowhere while the tool answered a snapshot. The
    // refusal rides this crate's invalid_params path, before the session
    // gate, like every other malformed input.
    let mcp = RutterMcp::new(manager(), SessionId::new("s1"));
    let error = match mcp
        .press_key(Parameters(PressKeyParams { key: String::new() }))
        .await
    {
        Ok(_) => panic!("an empty key must be refused"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(
        error.message.contains("key must not be empty"),
        "names the rule: {error}"
    );
}

#[tokio::test]
async fn set_cookies_with_no_cookies_is_invalid_params() {
    // An empty batch would park a human approval for a write that sets
    // nothing. It is refused before the session gate, so nobody is asked
    // and no engine launches just to refuse.
    let mcp = RutterMcp::new(manager(), SessionId::new("s1"));
    let error = match mcp
        .set_cookies(Parameters(CookiesParams {
            cookies: Vec::new(),
        }))
        .await
    {
        Ok(_) => panic!("an empty cookie batch must be refused"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(
        error.message.contains("must not be empty"),
        "names the rule: {error}"
    );
}

#[tokio::test]
async fn set_cookies_maps_inputs_and_reports_failures() {
    let mcp = RutterMcp::new(scripted_manager(), SessionId::new("s1"));
    // A real URL keeps the review from failing closed on a missing page
    // URL; the test targets the handler, not the gate.
    mcp.navigate(Parameters(NavigateParams {
        url: "https://example.com".to_owned(),
    }))
    .await
    .expect("a page with a url");

    let params = CookiesParams {
        cookies: vec![CookieInput {
            name: "session".to_owned(),
            value: "42".to_owned(),
            domain: "example.com".to_owned(),
            path: None,
            secure: None,
            http_only: None,
            same_site: Some(SameSiteInput::Lax),
            expires: None,
        }],
    };
    let result = mcp.set_cookies(Parameters(params)).await.expect("result");
    assert_eq!(first_text_block(&result), "set 1 cookie(s)");

    // A context-side failure is an isError result with the hint, like
    // every other action-adjacent failure.
    let failing = CookiesParams {
        cookies: vec![CookieInput {
            name: "boom".to_owned(),
            value: "x".to_owned(),
            domain: "example.com".to_owned(),
            path: None,
            secure: None,
            http_only: None,
            same_site: None,
            expires: None,
        }],
    };
    let result = mcp
        .set_cookies(Parameters(failing))
        .await
        .expect("a tool result, not a protocol error");
    assert!(is_error(&result), "the failure is an isError result");
    assert!(
        first_text_block(&result).contains("\nhint: "),
        "the failure carries its hint"
    );
}
