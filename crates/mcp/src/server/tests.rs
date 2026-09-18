//! Server tests: session lifecycle over a stub engine, result
//! mapping (truncation marker), and parameter mapping.

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
