//! Page-lifecycle tests: opening, closing, selecting, adoption of
//! foreign windows, recovery ownership, and the tracked-URL bookkeeping
//! every caller relies on.

use super::*;
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
    let _gate = session.registry().begin_recovery().1;
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

#[tokio::test]
async fn tabs_list_adopts_a_window_rutter_did_not_open() {
    // A window.open tab (or a human's window) exists in the engine with
    // nothing tracking it. Listing must discover it once, keep the
    // session's focus where it was, and say so on the timeline.
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));
    let (tracked_id, _) = session.active_page_for_test().await;

    let foreign_id = context.add_foreign_page("https://popup.example");
    let listed = session.pages().await;
    assert_eq!(
        listed.len(),
        2,
        "the popup shows up next to the tracked page"
    );
    assert_eq!(listed[0].id, tracked_id);
    assert!(listed[0].active, "adoption never steals the active flag");
    assert_eq!(listed[1].id, foreign_id);
    assert!(!listed[1].active);
    assert_eq!(listed[1].url, "https://popup.example");

    // Relisting is idempotent: one discovery, one event.
    assert_eq!(session.pages().await.len(), 2);
    let popup_opens = session
        .backbone()
        .replay(&SessionId::new("s-test"))
        .iter()
        .filter(|envelope| {
            matches!(
                &envelope.event,
                Event::PageOpened { page } if *page == foreign_id
            )
        })
        .count();
    assert_eq!(
        popup_opens, 1,
        "the popup opened exactly once on the timeline"
    );
}

#[tokio::test]
async fn a_selected_foreign_page_drives_the_window_it_names() {
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));
    let _ = session.active_page_for_test().await;
    let foreign_id = context.add_foreign_page("https://popup.example");
    session.pages().await;

    let snapshot = session
        .select_page(foreign_id.clone())
        .await
        .expect("the adopted page is selectable");
    assert_eq!(
        snapshot.url, "https://popup.example",
        "selection answers the adopted page's snapshot, not the old page's"
    );
    let listed = session.pages().await;
    assert!(
        listed
            .iter()
            .find(|page| page.id == foreign_id)
            .is_some_and(|page| page.active),
        "selection moves the active flag to the adopted page"
    );
}

#[tokio::test]
async fn an_enumeration_failure_degrades_to_the_tracked_listing() {
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));
    session.active_page_for_test().await;
    context.add_foreign_page("https://popup.example");
    context.fail_foreign_pages(true);

    let listed = session.pages().await;
    assert_eq!(
        listed.len(),
        1,
        "a wedged enumeration must not hide the tracked pages"
    );
    assert!(
        listed
            .iter()
            .all(|page| !page.id.as_str().starts_with("target:")),
        "no foreign page is invented when the engine would not say"
    );
}

#[tokio::test]
async fn adoption_waits_for_recovery_to_release_the_list() {
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));
    let (tracked_id, _) = session.active_page_for_test().await;
    context.add_foreign_page("https://popup.example");

    let (_saved, gate) = session.registry().begin_recovery();
    assert_eq!(
        session.pages().await.len(),
        1,
        "recovery owns the list; discovery waits"
    );

    // The write-back rebuilds one tracked page; the gate lifts, and
    // discovery resumes with the next listing — a surface the engine
    // still reports is adopted again, one that died with the old
    // engine (the real restart case) is simply never reported.
    session.registry().finish_recovery(
        vec![PageSlot {
            id: tracked_id.clone(),
            url: "https://example.com".to_owned(),
            handle: Arc::new(MockPage::new()),
            active: true,
        }],
        gate,
    );
    let listed = session.pages().await;
    assert_eq!(listed.len(), 2, "discovery resumes after the gate lifts");
    assert_eq!(listed[0].id, tracked_id, "the rebuilt page keeps focus");
    assert!(listed[0].active, "the rebuilt page is the active one");
}

#[tokio::test]
async fn closing_an_adopted_page_untracks_it_like_any_other() {
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));
    session.active_page_for_test().await;
    let foreign_id = context.add_foreign_page("https://popup.example");
    session.pages().await;

    let confirmation = session
        .close_page(foreign_id.clone())
        .await
        .expect("the adopted page closes through the normal path");
    assert!(confirmation.contains(&foreign_id.to_string()));
    assert_eq!(session.pages().await.len(), 1);
    assert!(
        session
            .backbone()
            .replay(&SessionId::new("s-test"))
            .iter()
            .any(|envelope| matches!(envelope.event, Event::PageClosed { .. })),
        "the close is announced like any other"
    );
}

#[tokio::test]
async fn opening_a_page_takes_the_active_flag_and_tracks_its_url() {
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));
    let (first, _) = session.active_page_for_test().await;

    let snapshot = session.open_page(None).await.expect("a blank page opens");
    let listed = session.pages().await;
    assert_eq!(listed.len(), 2);
    let active: Vec<&PageInfo> = listed.iter().filter(|page| page.active).collect();
    assert_eq!(active.len(), 1, "exactly one page is active");
    assert_ne!(active[0].id, first, "the new page takes the focus");
    // The mock's blank page reports an empty href; a real engine
    // reports `about:blank` here.
    assert_eq!(snapshot.url, "");

    let snapshot = session
        .open_page(Some("https://opened.example".to_owned()))
        .await
        .expect("a navigated page opens");
    assert_eq!(snapshot.url, "https://opened.example");
    assert!(
        session
            .backbone()
            .replay(&SessionId::new("s-test"))
            .iter()
            .any(|envelope| matches!(
                envelope.event,
                Event::PageNavigated { ref url, .. } if url == "https://opened.example"
            )),
        "the navigation lands on the timeline"
    );
    let active = session
        .pages()
        .await
        .into_iter()
        .find(|page| page.active)
        .expect("one active page");
    assert_eq!(
        active.url, "https://opened.example",
        "the tracked url follows the new page"
    );
}

#[tokio::test]
async fn a_denied_tabs_open_navigation_leaves_the_new_blank_page() {
    let context = MockContext::new();
    let session = session_with_policy(
        Arc::new(context.clone()),
        RuleSet::new(
            vec![navigation_deny("https://denied.example/*")],
            Verdict::Allow,
        ),
    );
    session.active_page_for_test().await;

    let error = session
        .open_page(Some("https://denied.example/login".to_owned()))
        .await
        .expect_err("the deny rule blocks the navigation");
    assert!(
        matches!(
            error,
            SessionError::Action(ActionError::ApprovalDenied { .. })
        ),
        "unexpected error: {error}"
    );
    let listed = session.pages().await;
    assert_eq!(
        listed.len(),
        2,
        "the opened page stays so the agent can see what it got"
    );
    let active = listed.iter().find(|page| page.active).expect("one active");
    assert_eq!(active.url, "", "the denied page never navigated");
}

#[tokio::test]
async fn set_viewport_resizes_the_active_page_and_returns_a_snapshot() {
    let context = MockContext::new();
    let session = session_over(Arc::new(context.clone()));
    let (page_id, _) = session.active_page_for_test().await;

    let snapshot = session
        .set_viewport(1280, 720)
        .await
        .expect("the resize succeeds");
    assert_eq!(snapshot.url, "", "a fresh snapshot comes back");
    let mock = context.page_mock(page_id).expect("mock page");
    assert_eq!(mock.viewport_calls(), vec![(1280, 720)]);
}
