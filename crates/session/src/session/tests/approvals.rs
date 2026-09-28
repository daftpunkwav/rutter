//! Policy-judgment and approval tests: which URL a verdict is reached
//! at, what a grant authorizes, how a parked operation settles, and
//! what the audit trail records.

use super::*;

/// One cookie write as the set-cookies paths under test send it.
fn cookie_write(domain: &str, value: &str) -> Cookie {
    Cookie {
        name: "session".to_owned(),
        value: value.to_owned(),
        domain: domain.to_owned(),
        path: None,
        secure: false,
        http_only: false,
        same_site: None,
        expires: None,
    }
}

#[tokio::test]
async fn a_grant_does_not_follow_a_page_that_moved_mid_park() {
    // The window the approval opens: `execute` takes &self, so a
    // concurrent navigation (or a page-side redirect) can move the page
    // while a click sits parked. The grant answers for the page the
    // human was shown; re-validation must send the click back through
    // review, which now reads the denied host and refuses.
    let context = MockContext::new();
    let session = Arc::new(session_with_policy(
        Arc::new(context.clone()),
        RuleSet::new(
            vec![
                rutter_policy::rules::PolicyRule {
                    action_class: Some("pointer".to_owned()),
                    url_pattern: rutter_policy::Pattern::parse("https://denied.example/*").ok(),
                    verdict: Verdict::Deny,
                },
                rutter_policy::rules::PolicyRule {
                    action_class: Some("pointer".to_owned()),
                    url_pattern: None,
                    verdict: Verdict::RequireApproval,
                },
            ],
            Verdict::Allow,
        )
        .with_approval_timeout(Duration::from_secs(5)),
    ));
    let (page_id, _) = session.active_page_for_test().await;
    let page = context.page_mock(page_id).expect("the active page");
    page.set_url("https://a.example/start");

    let running = tokio::spawn({
        let session = Arc::clone(&session);
        async move {
            session
                .execute(
                    Action::Click {
                        reference: rutter_core::reference::Reference::new("e1"),
                    },
                    Origin::Agent,
                )
                .await
        }
    });
    let request_id = pending_request_id(&session).await;

    // The page moves while the click is parked, and the human grants
    // what they were shown.
    page.set_url("https://denied.example/pay");
    assert!(
        session.broker().decide(
            &rutter_policy::ApprovalId::new(request_id),
            rutter_policy::Decision::Grant
        ),
        "the grant is accepted"
    );

    let outcome = running
        .await
        .expect("the action task joins")
        .expect_err("the moved page invalidates the grant");
    assert!(
        matches!(
            outcome,
            SessionError::Action(ActionError::ApprovalDenied { .. })
        ),
        "the re-review denies the page the human never approved: {outcome:?}"
    );
}

#[tokio::test]
async fn a_grant_survives_a_move_to_a_url_the_rules_allow() {
    // Re-validation re-judges, it does not refuse: a page that moved to
    // a host no rule objects to proceeds under the new verdict.
    let context = MockContext::new();
    let session = Arc::new(session_with_policy(
        Arc::new(context.clone()),
        RuleSet::new(
            vec![rutter_policy::rules::PolicyRule {
                action_class: Some("pointer".to_owned()),
                url_pattern: rutter_policy::Pattern::parse("https://a.example/*").ok(),
                verdict: Verdict::RequireApproval,
            }],
            Verdict::Allow,
        )
        .with_approval_timeout(Duration::from_secs(5)),
    ));
    let (page_id, _) = session.active_page_for_test().await;
    let page = context.page_mock(page_id).expect("the active page");
    page.set_url("https://a.example/start");

    let running = tokio::spawn({
        let session = Arc::clone(&session);
        async move {
            session
                .execute(
                    Action::Click {
                        reference: rutter_core::reference::Reference::new("e1"),
                    },
                    Origin::Agent,
                )
                .await
        }
    });
    let request_id = pending_request_id(&session).await;

    page.set_url("https://b.example/other");
    assert!(
        session.broker().decide(
            &rutter_policy::ApprovalId::new(request_id),
            rutter_policy::Decision::Grant
        ),
        "the grant is accepted"
    );

    running
        .await
        .expect("the action task joins")
        .expect("the moved page re-reviews as allowed");
}

/// Waits until the session's backbone shows a parked request and
/// returns its id.
async fn pending_request_id(session: &Session) -> String {
    pending_request_ids(session)
        .await
        .into_iter()
        .next()
        .expect("the operation parks")
}

/// Waits until the session's backbone shows at least one parked request
/// and returns every request id in history, oldest first. Re-validation
/// tests need the whole set: an earlier request stays in the ring, so a
/// single-id lookup would hand back the stale one.
async fn pending_request_ids(session: &Session) -> Vec<String> {
    crate::wait::poll_until(
        || async {
            let ids: Vec<String> = session
                .backbone()
                .replay(&SessionId::new("s-test"))
                .iter()
                .filter_map(|envelope| match &envelope.event {
                    Event::ApprovalRequested { request_id, .. } => Some(request_id.clone()),
                    _ => None,
                })
                .collect();
            (!ids.is_empty()).then_some(ids)
        },
        Duration::from_secs(5),
        Duration::from_millis(20),
    )
    .await
    .expect("the operation parks")
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
        .set_cookies(&[cookie_write("example.com", "42")])
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
    let cookie = cookie_write("shop.example", "1");

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
    // end" after the fact.
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
    let cookie = cookie_write("shop.example", "1");

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
    let cookie = cookie_write("shop.example", "1");

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
async fn a_denied_upload_names_the_targeted_input() {
    let context = MockContext::new();
    let session = session_with_policy(
        Arc::new(context),
        RuleSet::new(
            vec![rutter_policy::rules::PolicyRule {
                action_class: Some("file_upload".to_owned()),
                url_pattern: None,
                verdict: rutter_policy::Verdict::Deny,
            }],
            Verdict::Allow,
        ),
    );
    session.active_page_for_test().await;

    let error = session
        .execute(
            Action::SetInputFiles {
                reference: rutter_core::reference::Reference::new("e1"),
                paths: vec!["report.pdf".to_owned()],
            },
            Origin::Agent,
        )
        .await
        .expect_err("the class-only deny rule blocks the upload");
    let SessionError::Action(ActionError::ApprovalDenied { reference }) = error else {
        panic!("unexpected error: {error}")
    };
    assert_eq!(
        reference.as_str(),
        "e1",
        "the denial names the element the action targeted"
    );
}

#[tokio::test]
async fn a_cookie_grant_does_not_follow_a_page_that_moved_mid_park() {
    // The cookie write is judged at the page, so the re-validation
    // after a grant re-reads the page too: a move during the window
    // sends the write back through review, which parks again and asks
    // the human about the page as it now is.
    let context = MockContext::new();
    let _ = context;
    let (_entered_tx, _entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (_release_tx, release_rx) = tokio::sync::oneshot::channel();
    let gated = Arc::new(GatedPage {
        url: Mutex::new("https://shop.example/start".to_owned()),
        entered: _entered_tx,
        release: tokio::sync::Mutex::new(Some(release_rx)),
    });
    let session = Arc::new(Session::new(
        SessionId::new("s-test"),
        Arc::new(SinglePageContext(gated.clone())),
        Arc::new(Backbone::new()),
        SessionConfig::default(),
        Arc::new(RuleSet::default_set().with_approval_timeout(Duration::from_millis(500))),
        Arc::new(ApprovalBroker::new()),
        None,
    ));
    session.active_page_for_test().await;

    let parked = tokio::spawn({
        let session = Arc::clone(&session);
        async move {
            session
                .set_cookies(&[cookie_write("shop.example", "1")])
                .await
        }
    });
    let first = pending_request_id(&session).await;

    gated.move_page("https://other.example/moved");
    assert!(
        session.broker().decide(
            &rutter_policy::ApprovalId::new(first.clone()),
            rutter_policy::Decision::Grant
        ),
        "the first grant is accepted"
    );

    // The re-review parks a fresh request instead of riding the first
    // grant, and with nobody answering it times out.
    // The decide only wakes the parked writer; wait until the
    // fresh request the re-review must raise lands on the backbone.
    let second = crate::wait::poll_until(
        || async {
            let ids = pending_request_ids(&session).await;
            (ids.len() >= 2).then(|| ids[ids.len() - 1].clone())
        },
        Duration::from_secs(5),
        Duration::from_millis(20),
    )
    .await
    .expect("the re-review parks again");
    assert_ne!(
        second, first,
        "the moved page is asked about afresh, not waved through"
    );
    let outcome = parked.await.expect("the write task joins");
    assert!(
        matches!(
            outcome,
            Err(SessionError::Action(ActionError::ApprovalTimedOut { .. }))
        ),
        "nobody answers the second park: {outcome:?}"
    );
}

#[tokio::test]
async fn a_grant_without_a_readable_url_fails_closed_to_a_fresh_park() {
    // An unreadable page reports no URL; the re-validation must not
    // wave the write through on the strength of the first grant. The
    // empty URL here is the GatedPage stand-in for a page that stopped
    // answering `location.href`, and it canonicalizes to nothing — the
    // same fail-closed path a dead page takes.
    let (_entered_tx, _entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (_release_tx, release_rx) = tokio::sync::oneshot::channel();
    let gated = Arc::new(GatedPage {
        url: Mutex::new("https://shop.example/start".to_owned()),
        entered: _entered_tx,
        release: tokio::sync::Mutex::new(Some(release_rx)),
    });
    let session = Arc::new(Session::new(
        SessionId::new("s-test"),
        Arc::new(SinglePageContext(gated.clone())),
        Arc::new(Backbone::new()),
        SessionConfig::default(),
        Arc::new(RuleSet::default_set().with_approval_timeout(Duration::from_millis(500))),
        Arc::new(ApprovalBroker::new()),
        None,
    ));
    session.active_page_for_test().await;

    let parked = tokio::spawn({
        let session = Arc::clone(&session);
        async move {
            session
                .set_cookies(&[cookie_write("shop.example", "1")])
                .await
        }
    });
    let first = pending_request_id(&session).await;

    gated.move_page("");
    assert!(
        session.broker().decide(
            &rutter_policy::ApprovalId::new(first.clone()),
            rutter_policy::Decision::Grant
        ),
        "the first grant is accepted"
    );

    // The decide only wakes the parked writer; wait until the
    // fresh request the re-review must raise lands on the backbone.
    let second = crate::wait::poll_until(
        || async {
            let ids = pending_request_ids(&session).await;
            (ids.len() >= 2).then(|| ids[ids.len() - 1].clone())
        },
        Duration::from_secs(5),
        Duration::from_millis(20),
    )
    .await
    .expect("the re-review parks again");
    assert_ne!(
        second, first,
        "no URL means fail closed, which means asking again"
    );
    let outcome = parked.await.expect("the write task joins");
    assert!(
        matches!(
            outcome,
            Err(SessionError::Action(ActionError::ApprovalTimedOut { .. }))
        ),
        "the second park is unanswered: {outcome:?}"
    );
}
