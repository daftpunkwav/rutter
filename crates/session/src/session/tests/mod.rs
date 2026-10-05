//! Tests for the `Session` type, run against the crate's mock engine
//! (`crate::mock`) — no real browser, no network.
//!
//! The suite is split by responsibility: [`pages`] covers the page
//! lifecycle (tabs, adoption, recovery), [`approvals`] covers the
//! policy gate and the audit trail, and [`observations`] covers the
//! read-only surfaces. Shared fixtures live here.

mod approvals;
mod observations;
mod pages;
mod storage;

use super::*;
use crate::mock::{MockContext, MockPage};
use rutter_core::ids::ContextId;
use rutter_policy::{ActionClass, Verdict};
use serde_json::{Value, json};
use std::sync::Mutex;

fn default_session(context: Arc<dyn ContextHandle>) -> Session {
    session_with_policy(context, RuleSet::default_set())
}

fn session_with_short_approval(context: Arc<dyn ContextHandle>) -> Session {
    session_with_policy(
        context,
        RuleSet::default_set().with_approval_timeout(Duration::from_millis(100)),
    )
}

fn session_with_policy(context: Arc<dyn ContextHandle>, rules: RuleSet) -> Session {
    Session::new(
        SessionId::new("s-test"),
        context,
        Arc::new(Backbone::new()),
        SessionConfig::default(),
        Arc::new(rules),
        Arc::new(ApprovalBroker::new()),
        None,
    )
}

/// A session with a persistence file, the only kind that ever writes.
fn session_persisting_to(context: Arc<dyn ContextHandle>, path: PathBuf) -> Session {
    Session::new(
        SessionId::new("s-test"),
        context,
        Arc::new(Backbone::new()),
        SessionConfig::default(),
        Arc::new(RuleSet::default_set()),
        Arc::new(ApprovalBroker::new()),
        Some(path),
    )
}

/// A navigation-class deny rule for `pattern`.
fn navigation_deny(pattern: &str) -> rutter_policy::rules::PolicyRule {
    rutter_policy::rules::PolicyRule {
        action_class: Some(ActionClass::Navigation),
        url_pattern: rutter_policy::Pattern::parse(pattern).ok(),
        verdict: rutter_policy::Verdict::Deny,
    }
}

/// A page whose `navigate` parks until the test releases it, so another
/// session call can interleave mid-action deterministically.
struct GatedPage {
    url: Mutex<String>,
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl GatedPage {
    /// Moves the page behind any running operation: the shape of a
    /// concurrent navigation or a page-side redirect. An empty string
    /// stands in for a page that stopped answering `location.href`.
    fn move_page(&self, url: &str) {
        *self
            .url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = url.to_owned();
    }
}

#[async_trait::async_trait]
impl PageHandle for GatedPage {
    async fn navigate(&self, url: &str) -> Result<String, EngineError> {
        let _ = self.entered.send(());
        let receiver = self.release.lock().await.take();
        if let Some(receiver) = receiver {
            let _ = receiver.await;
        }
        let mut current = self
            .url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current = url.to_owned();
        Ok(current.clone())
    }

    async fn reload(&self) -> Result<(), EngineError> {
        Ok(())
    }

    async fn go_back(&self) -> Result<String, EngineError> {
        Ok(self
            .url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone())
    }

    async fn go_forward(&self) -> Result<String, EngineError> {
        Ok(self
            .url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone())
    }

    async fn evaluate(&self, expression: &str) -> Result<Value, EngineError> {
        if expression.contains("location.href") {
            return Ok(json!(
                self.url
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone()
            ));
        }
        if expression.contains("MAX_NODES") {
            return Ok(json!({
                "version": 1,
                "truncated": false,
                "root": { "role": "root", "children": [] }
            }));
        }
        Ok(Value::Null)
    }

    async fn dispatch_input(
        &self,
        _event: rutter_engine::input::InputEvent,
    ) -> Result<(), EngineError> {
        Ok(())
    }

    async fn capture_screenshot(&self) -> Result<Screenshot, EngineError> {
        Ok(Screenshot {
            format: rutter_engine::page::ImageFormat::Png,
            data: Vec::new(),
        })
    }

    async fn start_screencast(&self) -> Result<ScreencastStream, EngineError> {
        let (_sender, receiver) = tokio::sync::mpsc::channel(1);
        Ok(ScreencastStream::new(receiver))
    }
}

/// A context whose `open_page` always hands back one shared page.
struct SinglePageContext(Arc<dyn PageHandle>);

#[async_trait::async_trait]
impl ContextHandle for SinglePageContext {
    fn id(&self) -> ContextId {
        ContextId::new("ctx-single")
    }

    fn pages(&self) -> Vec<PageId> {
        vec![PageId::new("ctx-single:page-0")]
    }

    async fn open_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
        Ok((PageId::new("ctx-single:page-0"), Arc::clone(&self.0)))
    }

    fn page(&self, _id: PageId) -> Option<Arc<dyn PageHandle>> {
        None
    }

    async fn close_page(&self, _id: PageId) -> Result<(), EngineError> {
        Ok(())
    }

    async fn set_cookies(&self, _cookies: &[Cookie]) -> Result<(), EngineError> {
        Ok(())
    }

    async fn cookies(&self) -> Result<Vec<Cookie>, EngineError> {
        Err(EngineError::Terminated)
    }

    async fn close(&self) -> Result<(), EngineError> {
        Ok(())
    }
}

/// A page whose `evaluate` never answers (`Value::Null` for
/// everything): the shape of a page caught mid-navigation or dead.
struct BrokenLens;

#[async_trait::async_trait]
impl PageHandle for BrokenLens {
    async fn navigate(&self, url: &str) -> Result<String, EngineError> {
        Ok(url.to_owned())
    }

    async fn reload(&self) -> Result<(), EngineError> {
        Ok(())
    }

    async fn go_back(&self) -> Result<String, EngineError> {
        Ok(String::new())
    }

    async fn go_forward(&self) -> Result<String, EngineError> {
        Ok(String::new())
    }

    async fn evaluate(&self, _expression: &str) -> Result<Value, EngineError> {
        Ok(Value::Null)
    }

    async fn dispatch_input(
        &self,
        _event: rutter_engine::input::InputEvent,
    ) -> Result<(), EngineError> {
        Ok(())
    }

    async fn capture_screenshot(&self) -> Result<Screenshot, EngineError> {
        Err(EngineError::Terminated)
    }

    async fn start_screencast(&self) -> Result<ScreencastStream, EngineError> {
        Err(EngineError::Terminated)
    }
}

/// Pins which engine failures fold into which action-taxonomy entry.
/// The mapping (`as_action_error`) ends in an `other => Internal`
/// fallback, so the compiler cannot flag a newly added `EngineError`
/// variant as undecided; the exhaustive mirror match below breaks the
/// build instead, forcing the decision to land here — including the
/// decision to fall back, which is deliberate for the five non-action
/// failures.
#[test]
fn every_engine_error_variant_has_a_decided_action_mapping() {
    // Exhaustive over `EngineError`: a new variant is a compile error
    // here until its mapping below is decided on purpose.
    fn every_variant(error: &EngineError) {
        match error {
            EngineError::NavigationFailed { .. }
            | EngineError::LaunchFailed { .. }
            | EngineError::Terminated
            | EngineError::Timeout { .. }
            | EngineError::Unsupported { .. }
            | EngineError::Internal { .. }
            | EngineError::Capacity { .. }
            | EngineError::DownloadFailed { .. }
            | EngineError::ReferenceExpired { .. } => {}
        }
    }

    let session = SessionId::new("s1");
    let cases: Vec<(EngineError, ActionError)> = vec![
        // The deliberate one-to-one mappings.
        (
            EngineError::NavigationFailed {
                url: "https://a.example".to_owned(),
                cause: rutter_core::error::TransportCause::DnsFailed,
                detail: "no resolution".to_owned(),
            },
            ActionError::NavigationFailed {
                url: "https://a.example".to_owned(),
                cause: rutter_core::error::TransportCause::DnsFailed,
            },
        ),
        (
            EngineError::Terminated,
            ActionError::EngineTerminated {
                session: session.clone(),
            },
        ),
        // Engine-level timeouts are reported as the act phase on purpose
        // (see `as_action_error`); the pin keeps that decision visible.
        (
            EngineError::Timeout {
                operation: "click".to_owned(),
                elapsed: Duration::from_secs(1),
            },
            ActionError::TimedOut {
                phase: rutter_core::error::WaitPhase::Act,
                elapsed: Duration::from_secs(1),
            },
        ),
        (
            EngineError::ReferenceExpired {
                reference: "e17".to_owned(),
            },
            ActionError::ReferenceExpired {
                reference: rutter_core::reference::Reference::new("e17"),
            },
        ),
        // The deliberate fallbacks: launch, capability, contained bugs,
        // caps, and download failures are not action-shaped, so they
        // wear `Internal` while the tool result still carries the
        // engine's own message.
        (
            EngineError::LaunchFailed {
                detail: "no binary".to_owned(),
            },
            ActionError::Internal {
                detail: String::new(),
            },
        ),
        (
            EngineError::Unsupported {
                operation: "op".to_owned(),
                reason: "no".to_owned(),
            },
            ActionError::Internal {
                detail: String::new(),
            },
        ),
        (
            EngineError::Internal {
                detail: "bug".to_owned(),
            },
            ActionError::Internal {
                detail: String::new(),
            },
        ),
        (
            EngineError::Capacity {
                detail: "full".to_owned(),
            },
            ActionError::Internal {
                detail: String::new(),
            },
        ),
        (
            EngineError::DownloadFailed {
                detail: "gone".to_owned(),
            },
            ActionError::Internal {
                detail: String::new(),
            },
        ),
    ];
    for (engine, expected) in cases {
        every_variant(&engine);
        let mapped = as_action_error(&session, &SessionError::Engine(engine));
        assert_eq!(
            std::mem::discriminant(&mapped),
            std::mem::discriminant(&expected),
            "the engine failure must keep its decided taxonomy entry, got {mapped:?}"
        );
    }
}
