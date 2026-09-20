//! Tests for the `Session` type, run against the crate's mock engine
//! (`crate::mock`) — no real browser, no network.

use super::*;
use crate::mock::{MockContext, MockPage};
use rutter_core::ids::ContextId;
use rutter_policy::Verdict;
use serde_json::{Value, json};

fn session_over(context: Arc<dyn ContextHandle>) -> Session {
    Session::new(
        SessionId::new("s-test"),
        context,
        Arc::new(Backbone::new()),
        SessionConfig::default(),
        Arc::new(RuleSet::default_set()),
        Arc::new(ApprovalBroker::new()),
        None,
    )
}

fn session_with_short_approval(context: Arc<dyn ContextHandle>) -> Session {
    Session::new(
        SessionId::new("s-test"),
        context,
        Arc::new(Backbone::new()),
        SessionConfig::default(),
        Arc::new(RuleSet::default_set().with_approval_timeout(Duration::from_millis(100))),
        Arc::new(ApprovalBroker::new()),
        None,
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

/// A navigation-class deny rule for `pattern`.
fn navigation_deny(pattern: &str) -> rutter_policy::rules::PolicyRule {
    rutter_policy::rules::PolicyRule {
        action_class: Some("navigation".to_owned()),
        url_pattern: rutter_policy::Pattern::parse(pattern),
        verdict: rutter_policy::Verdict::Deny,
    }
}

#[tokio::test]
async fn close_closes_the_context_even_when_the_context_errors() {
    // The close contract: teardown always succeeds, and the real
    // context's `close` is the mechanism (a context that reports
    // "not found" must not wedge the session close either).
    struct FailingContext;
    #[async_trait::async_trait]
    impl ContextHandle for FailingContext {
        fn id(&self) -> ContextId {
            ContextId::new("ctx-failing")
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
            Err(EngineError::Terminated)
        }
        async fn set_cookies(&self, _cookies: &[Cookie]) -> Result<(), EngineError> {
            Err(EngineError::Terminated)
        }
        async fn cookies(&self) -> Result<Vec<Cookie>, EngineError> {
            Err(EngineError::Terminated)
        }
        async fn close(&self) -> Result<(), EngineError> {
            Err(EngineError::Terminated)
        }
    }

    let session = session_over(Arc::new(FailingContext));
    session.close().await;
}

#[tokio::test]
async fn close_page_removes_the_page_and_promotes_a_remaining_one() {
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));

    let (page_id, _handle) = session.active_page_for_test().await;
    session
        .execute(
            Action::Navigate {
                url: "https://example.com".to_owned(),
            },
            Origin::Human,
        )
        .await
        .expect("navigate seeds the tracked url");

    // Sessions track a second page only through recovery; inject it
    // the way `recover` would (same-module test, private fields).
    let (other_id, other_handle) = context.open_page().await.expect("second page");
    session.registry().append_for_test(PageSlot {
        id: other_id.clone(),
        url: "https://other.example".to_owned(),
        handle: other_handle,
        active: false,
    });

    // Closing the active page promotes the first remaining one.
    session
        .close_page(page_id.clone())
        .await
        .expect("close the page");
    let listed = session.pages().await;
    assert_eq!(listed.len(), 1);
    assert!(listed[0].active, "the remaining page must become active");
    assert_eq!(listed[0].id, other_id);
    assert!(
        session
            .backbone()
            .replay(&SessionId::new("s-test"))
            .iter()
            .any(|envelope| matches!(envelope.event, Event::PageClosed { .. })),
        "the close lands on the event backbone"
    );
}

#[tokio::test]
async fn closing_an_untracked_page_fails_without_a_fake_event() {
    // tabs_close with a stale or invented id is a client error: a silent
    // success would also publish a PageClosed event for a page that
    // never existed.
    let context = MockContext::new();
    let session = session_over(Arc::new(context));

    let error = session.close_page(PageId::new("nope")).await;
    assert!(
        matches!(
            error,
            Err(SessionError::Action(ActionError::NotInteractable { .. }))
        ),
        "an unknown page id fails like tabs_select does: {error:?}"
    );
    assert!(
        !session
            .backbone()
            .replay(&SessionId::new("s-test"))
            .iter()
            .any(|envelope| matches!(envelope.event, Event::PageClosed { .. })),
        "no PageClosed event may be published for an unknown page"
    );
}

#[tokio::test]
async fn select_page_switches_activity_and_unknown_pages_fail() {
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));

    let (first, _) = session.active_page_for_test().await;
    let (second, second_handle) = context.open_page().await.expect("second page");
    session.registry().append_for_test(PageSlot {
        id: second.clone(),
        url: "https://other.example".to_owned(),
        handle: second_handle,
        active: false,
    });

    session
        .select_page(second.clone())
        .await
        .expect("select the second page");
    let listed = session.pages().await;
    assert!(
        listed.iter().all(|info| info.active == (info.id == second)),
        "exactly the selected page is active: {listed:?}"
    );

    let missing = session.select_page(PageId::new("nope")).await;
    assert!(
        matches!(
            missing,
            Err(SessionError::Action(ActionError::NotInteractable { .. }))
        ),
        "an unknown page is not interactable, not an engine failure"
    );
    let _ = first;
}

#[tokio::test]
async fn set_cookies_requires_approval_by_the_default_policy() {
    // The default rule set requires approval for the cookies class;
    // with nobody answering within the short test window, the call
    // must time out rather than silently writing the cookies.
    let context = MockContext::new();
    let session = session_with_short_approval(Arc::new(context.clone()));
    session.active_page_for_test().await;

    let outcome = session
        .set_cookies(&[Cookie {
            name: "session".to_owned(),
            value: "42".to_owned(),
            domain: "example.com".to_owned(),
            path: None,
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        }])
        .await;
    assert!(
        matches!(
            outcome,
            Err(SessionError::Action(ActionError::ApprovalTimedOut { .. }))
        ),
        "unanswered approvals park, they do not write: {outcome:?}"
    );
    assert!(
        context.set_cookie_calls().is_empty(),
        "no cookie reached the context without a grant"
    );
}

#[tokio::test]
async fn wait_for_survives_an_absurd_budget() {
    // A u64::MAX budget overflows `Instant + Duration` unless the
    // 600 s clamp runs before the deadline is built; the mock finds
    // the needle immediately, so the happy path exercises exactly
    // that deadline construction.
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));
    let (page_id, _page) = session.active_page_for_test().await;
    let page = context
        .page_mock(page_id)
        .expect("the active page is registered in the mock context");
    page.set_found(true);

    let snapshot = tokio::time::timeout(
        Duration::from_secs(5),
        session.wait_for("needle", Duration::from_millis(u64::MAX)),
    )
    .await
    .expect("the clamped wait must stay bounded by real time here")
    .expect("the wait resolves with a snapshot");
    assert!(snapshot.to_string().contains("Ok"));
}

#[tokio::test]
async fn wait_for_timeout_names_the_requested_budget() {
    // A timeout must report the budget the caller asked for, so the
    // agent can reason about what it just waited.
    let context = MockContext::new();
    let session = session_over(Arc::new(context));
    session.active_page_for_test().await;

    let error = session
        .wait_for("needle", Duration::from_millis(150))
        .await
        .expect_err("the needle never appears");
    match &error {
        SessionError::Action(ActionError::TimedOut { phase, elapsed }) => {
            assert_eq!(*phase, rutter_core::error::WaitPhase::Settle);
            assert_eq!(*elapsed, Duration::from_millis(150));
        }
        other => panic!("expected a TimedOut error, got {other:?}"),
    }
}

/// A page whose `navigate` parks until the test releases it, so another
/// session call can interleave mid-action deterministically.
struct GatedPage {
    url: Mutex<String>,
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
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
        if expression.contains("var MAX_NODES = ") {
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

#[tokio::test]
async fn url_refresh_targets_the_page_the_action_ran_on() {
    // Race: a select_page that lands while another action is still in
    // flight must not paste the acting page's URL onto the newly
    // selected one — the refresh filters by page id, not by whichever
    // slot is active when the action finishes.
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let gated = Arc::new(GatedPage {
        url: Mutex::new("https://a.example/start".to_owned()),
        entered: entered_tx,
        release: tokio::sync::Mutex::new(Some(release_rx)),
    });
    let session = Arc::new(session_over(Arc::new(SinglePageContext(gated))));
    let (a_id, _) = session.active_page_for_test().await;

    // A second page select_page can switch to while the action runs.
    let other = Arc::new(MockPage::new());
    other.set_url("https://b.example/other");
    let b_id = PageId::new("ctx-single:page-b");
    session.registry().append_for_test(PageSlot {
        id: b_id.clone(),
        url: "https://b.example/other".to_owned(),
        handle: other,
        active: false,
    });

    let running = tokio::spawn({
        let session = Arc::clone(&session);
        async move {
            session
                .execute(
                    Action::Navigate {
                        url: "https://a.example/after".to_owned(),
                    },
                    Origin::Human,
                )
                .await
        }
    });
    entered_rx
        .recv()
        .await
        .expect("the action reached the gated navigate");

    session
        .select_page(b_id.clone())
        .await
        .expect("the selection lands mid-action");
    release_tx.send(()).expect("release the gated navigate");
    running
        .await
        .expect("the action task joins")
        .expect("the action succeeds");

    let listed = session.pages().await;
    let a = listed
        .iter()
        .find(|page| page.id == a_id)
        .expect("page a stays tracked");
    let b = listed
        .iter()
        .find(|page| page.id == b_id)
        .expect("page b stays tracked");
    assert_eq!(
        a.url, "https://a.example/after",
        "the acted-on page receives the URL refresh"
    );
    assert_eq!(
        b.url, "https://b.example/other",
        "the selected page keeps its own URL"
    );
    assert!(b.active, "the selection survives the concurrent action");
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

#[tokio::test]
async fn screenshot_capture_errors_map_to_the_internal_taxonomy() {
    // docs/tool-catalog.md §4: capture errors surface as `ActionError::Internal` —
    // the taxonomy every action failure uses — not as a raw engine
    // error.
    let session = session_over(Arc::new(SinglePageContext(Arc::new(BrokenLens))));
    let error = session
        .screenshot()
        .await
        .expect_err("the broken lens never captures");
    assert!(
        matches!(error, SessionError::Action(ActionError::Internal { .. })),
        "capture errors use the action taxonomy: {error:?}"
    );
}

#[tokio::test]
async fn a_navigation_is_judged_by_its_target_url() {
    // A deny rule on the CURRENT page must not wave through to a
    // navigation, and vice versa: the judgment URL for `Navigate` is
    // where it goes (the old current-page judgment made
    // `deny https://allowed.example/*` blind to every navigation
    // away from it).
    let context = MockContext::new();
    let session = session_with_policy(
        Arc::new(context.clone()),
        RuleSet::new(
            vec![navigation_deny("https://allowed.example/*")],
            Verdict::Allow,
        ),
    );

    session
        .execute(
            Action::Navigate {
                url: "https://allowed.example/".to_owned(),
            },
            Origin::Human,
        )
        .await
        .expect("human origin bypasses policy");

    session
        .execute(
            Action::Navigate {
                url: "https://other.example/".to_owned(),
            },
            Origin::Agent,
        )
        .await
        .expect("the target URL is what counts, not the current page");
}

#[tokio::test]
async fn a_denied_target_url_stops_the_navigation() {
    let context = MockContext::new();
    let session = session_with_policy(
        Arc::new(context),
        RuleSet::new(
            vec![navigation_deny("https://denied.example/*")],
            Verdict::Allow,
        ),
    );

    let error = session
        .execute(
            Action::Navigate {
                url: "https://denied.example/pay".to_owned(),
            },
            Origin::Agent,
        )
        .await;
    assert!(
        matches!(
            error,
            Err(SessionError::Action(ActionError::ApprovalDenied { .. }))
        ),
        "a denied destination must not load: {error:?}"
    );
}

#[tokio::test]
async fn a_denied_target_is_not_bypassed_by_case_or_default_port() {
    // The judgment URL is the canonical form: a spelling the browser
    // resolves to the same origin (host case, an explicit default port)
    // must reach the same rule. Judged on the raw string instead, the
    // byte-level pattern match would treat this navigation as a miss
    // and let it through.
    let context = MockContext::new();
    let session = session_with_policy(
        Arc::new(context),
        RuleSet::new(
            vec![navigation_deny("https://denied.example/*")],
            Verdict::Allow,
        ),
    );

    let error = session
        .execute(
            Action::Navigate {
                url: "HTTPS://Denied.Example:443/pay".to_owned(),
            },
            Origin::Agent,
        )
        .await;
    assert!(
        matches!(
            error,
            Err(SessionError::Action(ActionError::ApprovalDenied { .. }))
        ),
        "the canonical form of the target is what the rule sees: {error:?}"
    );
}

#[tokio::test]
async fn a_credential_target_fails_closed_to_approval() {
    // A user@host target has no canonical form (the decoy before the @
    // would end up in a textual judgment while the browser contacts the
    // host after it), so the navigation must take the fail-closed path:
    // URL-scoped rules cannot match, and the bare allow upgrades to an
    // approval instead of silently passing as a pattern miss.
    let context = MockContext::new();
    let session = session_with_policy(
        Arc::new(context),
        RuleSet::new(
            vec![navigation_deny("https://denied.example/*")],
            Verdict::Allow,
        )
        .with_approval_timeout(Duration::from_millis(100)),
    );

    let error = session
        .execute(
            Action::Navigate {
                url: "https://good.example@evil.example/".to_owned(),
            },
            Origin::Agent,
        )
        .await;
    assert!(
        matches!(
            error,
            Err(SessionError::Action(ActionError::ApprovalTimedOut { .. }))
        ),
        "the credential target parks for a human: {error:?}"
    );
    assert!(
        session
            .backbone()
            .replay(&SessionId::new("s-test"))
            .iter()
            .any(|envelope| matches!(envelope.event, Event::ApprovalRequested { .. })),
        "the human is asked, silently allowing is not an option"
    );
}

#[tokio::test]
async fn an_unreadable_page_fails_closed_to_approval() {
    // Old semantics judged on a degraded `about:blank` and let the
    // action sail through; the fail-closed upgrade parks it for a
    // human instead.
    let session = session_with_policy(
        Arc::new(SinglePageContext(Arc::new(BrokenLens))),
        RuleSet::new(vec![], Verdict::Allow).with_approval_timeout(Duration::from_millis(100)),
    );

    let error = session
        .execute(
            Action::Click {
                reference: rutter_core::reference::Reference::new("e1"),
            },
            Origin::Agent,
        )
        .await;
    assert!(
        matches!(
            error,
            Err(SessionError::Action(ActionError::ApprovalTimedOut { .. }))
        ),
        "the fail-closed upgrade goes through the approval park: {error:?}"
    );
    assert!(
        session
            .backbone()
            .replay(&SessionId::new("s-test"))
            .iter()
            .any(|envelope| matches!(envelope.event, Event::ApprovalRequested { .. })),
        "the human is asked, silently allowing is not an option"
    );
}

#[tokio::test]
async fn an_unreadable_page_still_honors_class_level_denies() {
    let session = session_with_policy(
        Arc::new(SinglePageContext(Arc::new(BrokenLens))),
        RuleSet::new(vec![], Verdict::Deny),
    );

    let error = session
        .execute(
            Action::Click {
                reference: rutter_core::reference::Reference::new("e1"),
            },
            Origin::Agent,
        )
        .await;
    assert!(
        matches!(
            error,
            Err(SessionError::Action(ActionError::ApprovalDenied { .. }))
        ),
        "a deny default denies regardless of URL information: {error:?}"
    );
}

#[tokio::test]
async fn a_cookie_approval_asks_about_the_cookies_it_writes() {
    // The regression: a cookie write had no `Action` variant to carry, so
    // the request went out as `reload` and a person approving "reload"
    // authorized the write instead. The brief must name its own effect.
    let session = session_with_policy(
        Arc::new(MockContext::new()),
        RuleSet::default_set().with_approval_timeout(Duration::from_millis(100)),
    );
    let cookie = rutter_core::cookie::Cookie {
        name: "session".to_owned(),
        value: "1".to_owned(),
        domain: "shop.example".to_owned(),
        path: None,
        secure: false,
        http_only: false,
        same_site: None,
        expires: None,
    };

    let error = session
        .set_cookies(std::slice::from_ref(&cookie))
        .await
        .expect_err("the default set parks cookie writes");
    assert!(
        matches!(
            error,
            SessionError::Action(ActionError::ApprovalTimedOut { .. })
        ),
        "nobody answers in this test, so the write times out: {error:?}"
    );

    let brief = session
        .backbone()
        .replay(&SessionId::new("s-test"))
        .iter()
        .find_map(|envelope| match &envelope.event {
            Event::ApprovalRequested { brief, .. } => Some(brief.clone()),
            _ => None,
        })
        .expect("a human was asked");
    assert_eq!(brief.class, rutter_policy::ActionClass::Cookies);
    assert_eq!(
        brief.effect,
        ApprovalEffect::Cookies { count: 1 },
        "the request says what a grant authorizes"
    );
    assert!(
        !matches!(brief.effect, ApprovalEffect::Action { .. }),
        "no unrelated action stands in for a cookie write"
    );
}

#[tokio::test]
async fn a_supervised_decision_leaves_an_audit_line() {
    // The backbone is the live view and a closed session's ring is dropped
    // with it; the audit is what answers "what was asked, and how did it
    // end" after the fact (docs/policy.md).
    let dir = tempfile::tempdir().expect("temp dir");
    let session = Session::new(
        SessionId::new("s-audit"),
        Arc::new(MockContext::new()),
        Arc::new(Backbone::new()),
        SessionConfig::default(),
        Arc::new(RuleSet::default_set().with_approval_timeout(Duration::from_millis(50))),
        Arc::new(ApprovalBroker::new()),
        Some(dir.path().join("s-audit.storage.json")),
    );
    let cookie = rutter_core::cookie::Cookie {
        name: "session".to_owned(),
        value: "1".to_owned(),
        domain: "shop.example".to_owned(),
        path: None,
        secure: false,
        http_only: false,
        same_site: None,
        expires: None,
    };

    session
        .set_cookies(std::slice::from_ref(&cookie))
        .await
        .expect_err("nobody answers this one");

    let written =
        std::fs::read_to_string(dir.path().join("approvals.jsonl")).expect("the audit file exists");
    let lines: Vec<&str> = written.lines().collect();
    assert_eq!(lines.len(), 1, "one decision, one line");
    let record: Value = serde_json::from_str(lines[0]).expect("one JSON object per line");
    assert_eq!(record["session"], "s-audit");
    assert_eq!(record["class"], "cookies");
    assert_eq!(record["outcome"], "timed_out");
    assert_eq!(record["effect"], "1 cookie write(s)");
    assert_eq!(record["basis"], "rule 1");
    assert!(
        record["request_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("apr-")),
        "the broker id is recorded: {record}"
    );
    assert!(
        record["at"].as_str().is_some_and(|at| at.contains('T')),
        "timestamps are RFC 3339: {record}"
    );
}

#[tokio::test]
async fn a_cancelled_wait_is_audited_and_resolved() {
    // The case control flow alone cannot cover: the client went away, so the
    // parking future was dropped. The card must still be retired and the
    // trail must still say the decision ended.
    let dir = tempfile::tempdir().expect("temp dir");
    let session = Arc::new(Session::new(
        SessionId::new("s-cancel"),
        Arc::new(MockContext::new()),
        Arc::new(Backbone::new()),
        SessionConfig::default(),
        Arc::new(RuleSet::default_set()),
        Arc::new(ApprovalBroker::new()),
        Some(dir.path().join("s-cancel.storage.json")),
    ));
    let cookie = rutter_core::cookie::Cookie {
        name: "session".to_owned(),
        value: "1".to_owned(),
        domain: "shop.example".to_owned(),
        path: None,
        secure: false,
        http_only: false,
        same_site: None,
        expires: None,
    };

    let parked = {
        let session = Arc::clone(&session);
        let cookie = cookie.clone();
        tokio::spawn(async move { session.set_cookies(std::slice::from_ref(&cookie)).await })
    };
    // Let it reach the park, then cancel the caller mid-wait.
    tokio::time::sleep(Duration::from_millis(80)).await;
    parked.abort();
    let _ = parked.await;

    let events = session.backbone().replay(&SessionId::new("s-cancel"));
    assert!(
        events.iter().any(|envelope| matches!(
            envelope.event,
            Event::ApprovalResolved { granted: false, .. }
        )),
        "the dashboard card is retired rather than left dangling"
    );

    let written = std::fs::read_to_string(dir.path().join("approvals.jsonl"))
        .expect("the cancelled decision is still audited");
    let record: Value =
        serde_json::from_str(written.lines().next().expect("one line")).expect("valid JSON");
    assert_eq!(record["outcome"], "cancelled");
    assert_eq!(
        record["class"], "cookies",
        "the record says what was pending: {record}"
    );
}

#[tokio::test]
async fn watching_a_page_never_opens_one() {
    // The dashboard promises observation without effect. This used to run
    // through the same lookup an action uses, which opens a tab when the
    // session has none — so asking to watch created the thing watched.
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));

    assert!(
        matches!(session.screencast().await, Err(SessionError::NoOpenPage)),
        "the refusal must name the real cause, not an engine failure"
    );
    assert!(
        context.pages().is_empty(),
        "a refused observation opened a tab anyway"
    );
    assert!(
        session.pages().await.is_empty(),
        "the session gained a page it never had"
    );
}

#[tokio::test]
async fn an_action_during_recovery_leaves_no_untracked_page() {
    // The race the registry gate exists for: recovery read the list out and
    // wrote a rebuilt one back some time later. An action that opened a
    // page in between was registered, then overwritten — so its tab stayed
    // alive in the engine with nothing tracking it.
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));
    let (page_id, _) = session.active_page_for_test().await;
    assert_eq!(context.pages().len(), 1, "the seeded page is open");

    // Stand in for the window recovery owns: read-out done, write-back not.
    session.registry().begin_recovery();
    let error = session
        .execute(Action::Back, Origin::Agent)
        .await
        .expect_err("recovery owns the page list");
    assert!(
        matches!(
            error,
            SessionError::Engine(rutter_engine::error::EngineError::Terminated)
        ),
        "the caller is told to retry, not handed a dropped page: {error:?}"
    );
    assert_eq!(
        context.pages().len(),
        1,
        "a refused action leaked a live tab into the engine"
    );
    assert_eq!(
        session.pages().await[0].id,
        page_id,
        "the tracked page survived the refusal"
    );
}
