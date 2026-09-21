//! Session-level tests for the paths the public flow tests leave dark:
//! storage capture/save/load, the direct `recover` rebuild, the event
//! error taxonomy, the persistence failure path, and the read-only
//! observation tools. Doubles are local to this module where the shared
//! mock's scripted answers do not reach (cookie-bearing contexts,
//! localStorage dumps).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rutter_core::cookie::Cookie;
use rutter_core::error::TransportCause;
use rutter_core::error::{ActionError, WaitPhase};
use rutter_core::ids::{ContextId, PageId};
use rutter_engine::context::ContextHandle;
use rutter_engine::error::EngineError;
use rutter_engine::input::InputEvent;
use rutter_engine::page::{ImageFormat, PageHandle, ScreencastStream, Screenshot};
use rutter_events::Event;
use rutter_policy::{ApprovalBroker, RuleSet, Verdict};
use serde_json::{Value, json};

use super::Session;
use crate::config::SessionConfig;
use crate::error::SessionError;
use crate::mock::MockContext;
use crate::storage::StorageState;

fn poisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A page double answering the storage-dump script with real entries and
/// recording every evaluate, so restore scripts are observable.
struct StorageLens {
    url: Mutex<String>,
    dump: Value,
    evaluated: Mutex<Vec<String>>,
}

impl StorageLens {
    fn new(dump: Value) -> Arc<Self> {
        Arc::new(Self {
            url: Mutex::new("https://shop.example/".to_owned()),
            dump,
            evaluated: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl PageHandle for StorageLens {
    async fn navigate(&self, url: &str) -> Result<String, EngineError> {
        *poisoned(&self.url) = url.to_owned();
        Ok(url.to_owned())
    }

    async fn reload(&self) -> Result<(), EngineError> {
        Ok(())
    }

    async fn go_back(&self) -> Result<String, EngineError> {
        Ok(poisoned(&self.url).clone())
    }

    async fn go_forward(&self) -> Result<String, EngineError> {
        Ok(poisoned(&self.url).clone())
    }

    async fn evaluate(&self, expression: &str) -> Result<Value, EngineError> {
        poisoned(&self.evaluated).push(expression.to_owned());
        if expression.contains("localStorage") {
            return Ok(self.dump.clone());
        }
        if expression.contains("location.href") {
            return Ok(json!(poisoned(&self.url).clone()));
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

    async fn dispatch_input(&self, _event: InputEvent) -> Result<(), EngineError> {
        Ok(())
    }

    async fn capture_screenshot(&self) -> Result<Screenshot, EngineError> {
        Ok(Screenshot {
            format: ImageFormat::Png,
            data: Vec::new(),
        })
    }

    async fn start_screencast(&self) -> Result<ScreencastStream, EngineError> {
        let (_sender, receiver) = tokio::sync::mpsc::channel(1);
        Ok(ScreencastStream::new(receiver))
    }
}

/// A context double holding one shared page and a cookie jar, recording
/// every `set_cookies` batch.
struct StorageContext {
    page: Arc<StorageLens>,
    cookies: Mutex<Vec<Cookie>>,
    set_calls: Mutex<Vec<Vec<Cookie>>>,
    counter: Mutex<u64>,
}

impl StorageContext {
    fn new(page: Arc<StorageLens>) -> Arc<Self> {
        Arc::new(Self {
            page,
            cookies: Mutex::new(Vec::new()),
            set_calls: Mutex::new(Vec::new()),
            counter: Mutex::new(0),
        })
    }
}

#[async_trait::async_trait]
impl ContextHandle for StorageContext {
    fn id(&self) -> ContextId {
        ContextId::new("ctx-storage")
    }

    fn pages(&self) -> Vec<PageId> {
        Vec::new()
    }

    async fn open_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
        let mut counter = poisoned(&self.counter);
        let id = PageId::new(format!("ctx-storage:page-{counter}"));
        *counter += 1;
        Ok((id, Arc::clone(&self.page) as Arc<dyn PageHandle>))
    }

    fn page(&self, _id: PageId) -> Option<Arc<dyn PageHandle>> {
        Some(Arc::clone(&self.page) as Arc<dyn PageHandle>)
    }

    async fn close_page(&self, _id: PageId) -> Result<(), EngineError> {
        Ok(())
    }

    async fn set_cookies(&self, cookies: &[Cookie]) -> Result<(), EngineError> {
        *poisoned(&self.cookies) = cookies.to_vec();
        poisoned(&self.set_calls).push(cookies.to_vec());
        Ok(())
    }

    async fn cookies(&self) -> Result<Vec<Cookie>, EngineError> {
        Ok(poisoned(&self.cookies).clone())
    }

    async fn close(&self) -> Result<(), EngineError> {
        Ok(())
    }
}

fn session_cookie(name: &str, value: &str) -> Cookie {
    Cookie {
        name: name.to_owned(),
        value: value.to_owned(),
        domain: "shop.example".to_owned(),
        path: None,
        secure: false,
        http_only: false,
        same_site: None,
        expires: None,
    }
}

fn session_with_state(
    context: Arc<dyn ContextHandle>,
    state_path: Option<std::path::PathBuf>,
) -> Session {
    Session::new(
        rutter_core::ids::SessionId::new("s-storage"),
        context,
        Arc::new(rutter_events::Backbone::new()),
        SessionConfig::default(),
        Arc::new(RuleSet::default_set()),
        Arc::new(ApprovalBroker::new()),
        state_path,
    )
}

#[tokio::test]
async fn capture_storage_collects_cookies_and_local_storage() {
    let page = StorageLens::new(json!({ "data": { "token": "abc", "theme": "dark" } }));
    let context = StorageContext::new(Arc::clone(&page));
    *poisoned(&context.cookies) = vec![session_cookie("session", "42")];
    let session = session_with_state(Arc::clone(&context) as Arc<dyn ContextHandle>, None);
    session
        .execute(
            rutter_core::action::Action::Navigate {
                url: "https://shop.example/".to_owned(),
            },
            rutter_core::action::Origin::Human,
        )
        .await
        .expect("navigate gives the page its origin");

    let state = session.capture_storage().await;
    assert_eq!(state.cookies, vec![session_cookie("session", "42")]);
    assert_eq!(state.origins.len(), 1, "one origin's storage is captured");
    assert_eq!(state.origins[0].origin, "https://shop.example/");
    let mut entries = state.origins[0].entries.clone();
    entries.sort();
    assert_eq!(
        entries,
        vec![
            ("theme".to_owned(), "dark".to_owned()),
            ("token".to_owned(), "abc".to_owned())
        ]
    );
    // A read-only probe persists nothing and touches nothing.
    assert!(poisoned(&context.set_calls).is_empty());
}

#[tokio::test]
async fn save_and_load_storage_round_trip_through_the_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state_path = dir.path().join("s-storage.storage.json");
    let page = StorageLens::new(json!({ "data": { "token": "abc" } }));
    let context = StorageContext::new(Arc::clone(&page));
    *poisoned(&context.cookies) = vec![session_cookie("session", "42")];
    let session = session_with_state(
        Arc::clone(&context) as Arc<dyn ContextHandle>,
        Some(state_path.clone()),
    );

    session
        .execute(
            rutter_core::action::Action::Navigate {
                url: "https://shop.example/".to_owned(),
            },
            rutter_core::action::Origin::Human,
        )
        .await
        .expect("navigate seeds the page and the url");
    session.save_storage().await.expect("explicit save");
    let written = StorageState::read(&state_path);
    assert_eq!(written.cookies, vec![session_cookie("session", "42")]);
    assert_eq!(written.origins.len(), 1);

    // A different process wrote new state: load applies it.
    let external = StorageState {
        cookies: vec![session_cookie("other", "99")],
        origins: vec![crate::storage::OriginStorage {
            origin: "https://shop.example/".to_owned(),
            entries: vec![("token".to_owned(), "xyz".to_owned())],
        }],
    };
    external.write(&state_path).expect("external write");
    session.load_storage().await.expect("explicit load");

    let calls = poisoned(&context.set_calls);
    assert_eq!(calls.len(), 1, "the loaded cookies reach the context once");
    assert_eq!(calls[0], vec![session_cookie("other", "99")]);
    let evaluated = poisoned(&page.evaluated);
    assert!(
        evaluated
            .iter()
            .any(|expression| expression.contains("localStorage")),
        "the restore script runs on the matching origin's page"
    );
}

#[tokio::test]
async fn load_storage_without_a_state_directory_fails_loudly() {
    let page = StorageLens::new(json!({ "unavailable": true }));
    let session = session_with_state(StorageContext::new(page) as Arc<dyn ContextHandle>, None);
    let error = session.load_storage().await;
    assert!(
        matches!(
            error,
            Err(SessionError::Action(ActionError::Internal { .. }))
        ),
        "the caller must learn there is nothing to load: {error:?}"
    );
}

#[tokio::test]
async fn recover_rebuilds_the_session_on_a_fresh_context() {
    let session = session_with_state(Arc::new(MockContext::new()), None);
    session
        .execute(
            rutter_core::action::Action::Navigate {
                url: "https://a.example/start".to_owned(),
            },
            rutter_core::action::Origin::Human,
        )
        .await
        .expect("navigate seeds the tracked url");

    // The in-memory state a dead engine cannot take with it.
    *session.lock_last_storage() = StorageState {
        cookies: vec![session_cookie("session", "42")],
        origins: Vec::new(),
    };

    let rebuilt = Arc::new(MockContext::new());
    session
        .recover(Arc::clone(&rebuilt) as Arc<dyn ContextHandle>)
        .await;

    // Both replay paths carry the cookies: the direct context replay and
    // the per-page storage restore (idempotent by design).
    let calls = rebuilt.set_cookie_calls();
    assert!(
        calls
            .iter()
            .all(|call| *call == vec![session_cookie("session", "42")]),
        "every replay path carries the remembered cookie: {calls:?}"
    );
    assert!(!calls.is_empty());

    let pages = session.pages().await;
    assert_eq!(pages.len(), 1, "the tracked page is reopened");
    assert_eq!(pages[0].url, "https://a.example/start");
    assert!(pages[0].active, "exactly one restored page is active");
    assert_eq!(
        rebuilt.pages().len(),
        1,
        "the restored page lives in the new context"
    );

    let id = rutter_core::ids::SessionId::new("s-storage");
    let replay = session.backbone().replay(&id);
    assert!(
        replay
            .iter()
            .any(|envelope| matches!(envelope.event, Event::EngineRestarted)),
        "the restart is announced"
    );
    assert!(
        replay
            .iter()
            .any(|envelope| matches!(envelope.event, Event::PageOpened { .. })),
        "the restored page is announced"
    );
}

#[tokio::test]
async fn a_failed_action_publishes_the_taxonomy_error() {
    // Queue a "missing" answer on the page the session owns, then click:
    // the failure must surface to the caller and land on the backbone
    // with the same taxonomy error in its payload.
    let context = Arc::new(MockContext::new());
    let session = session_with_state(Arc::clone(&context) as Arc<dyn ContextHandle>, None);
    let (page_id, _) = session.active_page_for_test().await;
    let page = context.page_mock(page_id).expect("the owned page");
    page.push_resolve_answer(crate::mock::missing_answer());

    let error = session
        .execute(
            rutter_core::action::Action::Click {
                reference: rutter_core::reference::Reference::new("e9"),
            },
            rutter_core::action::Origin::Human,
        )
        .await;
    assert!(
        matches!(
            error,
            Err(SessionError::Action(ActionError::ReferenceExpired { .. }))
        ),
        "the click fails with the taxonomy error: {error:?}"
    );
    let id = rutter_core::ids::SessionId::new("s-storage");
    let replay = session.backbone().replay(&id);
    let failed = replay
        .iter()
        .find_map(|envelope| match &envelope.event {
            Event::ActionFailed { error, .. } => Some(error.clone()),
            _ => None,
        })
        .expect("the failure lands on the backbone");
    assert!(
        matches!(failed, ActionError::ReferenceExpired { .. }),
        "the event payload carries the same taxonomy error"
    );
}

#[tokio::test]
async fn observation_tools_never_mutate_but_answer() {
    let context = Arc::new(MockContext::new());
    let session = session_with_state(Arc::clone(&context) as Arc<dyn ContextHandle>, None);
    let (_, _) = session.active_page_for_test().await;

    let shot = session.screenshot().await.expect("the capture works");
    assert_eq!(shot.format, ImageFormat::Png);
    let mut cast = session.screencast().await.expect("the stream opens");
    assert!(
        cast.next_frame().await.is_none(),
        "the mock sends no frames"
    );
    assert_eq!(context.pages().len(), 1, "observation opened no second tab");
}

#[tokio::test]
async fn a_persistence_failure_is_remembered_and_retried() {
    // The state path's parent is a file: every write fails. The session
    // must keep working, remember the failure, and try again on the next
    // change instead of believing the file is current.
    let dir = tempfile::tempdir().expect("tempdir");
    let blocker = dir.path().join("a-file");
    std::fs::write(&blocker, "not a directory").expect("blocker file");
    let session = session_with_state(
        Arc::new(MockContext::new()),
        Some(blocker.join("s.storage.json")),
    );
    session
        .execute(
            rutter_core::action::Action::Navigate {
                url: "https://a.example".to_owned(),
            },
            rutter_core::action::Origin::Human,
        )
        .await
        .expect("the action itself still succeeds");
    assert!(
        !*session.lock_last_persist_ok(),
        "a failed write must not mark the file current"
    );
}

#[tokio::test]
async fn broker_and_backbone_are_the_shared_instances() {
    let context = Arc::new(MockContext::new());
    let session = session_with_state(Arc::clone(&context) as Arc<dyn ContextHandle>, None);
    assert!(
        Arc::ptr_eq(&session.broker(), &session.broker()),
        "the broker handle is stable"
    );
    // The backbone the session publishes on is the one a consumer
    // replaying its history reads.
    session.active_page_for_test().await;
    let id = rutter_core::ids::SessionId::new("s-storage");
    assert!(
        session
            .backbone()
            .replay(&id)
            .iter()
            .any(|envelope| matches!(envelope.event, Event::PageOpened { .. }))
    );
}

#[test]
fn engine_failures_map_through_the_event_taxonomy() {
    let session = rutter_core::ids::SessionId::new("s-map");
    let cases: Vec<(SessionError, Box<dyn Fn(&ActionError) -> bool>)> = vec![
        (
            SessionError::Engine(EngineError::Terminated),
            Box::new(|error| matches!(error, ActionError::EngineTerminated { .. })),
        ),
        (
            SessionError::Engine(EngineError::NavigationFailed {
                url: "https://a.example".to_owned(),
                cause: TransportCause::ConnectionFailed,
                detail: "connection refused".to_owned(),
            }),
            Box::new(|error| matches!(error, ActionError::NavigationFailed { .. })),
        ),
        (
            SessionError::Engine(EngineError::Timeout {
                operation: "navigate".to_owned(),
                elapsed: Duration::from_millis(5),
            }),
            Box::new(|error| {
                matches!(
                    error,
                    ActionError::TimedOut {
                        phase: WaitPhase::Act,
                        ..
                    }
                )
            }),
        ),
        (
            SessionError::Engine(EngineError::Unsupported {
                operation: "screencast".to_owned(),
                reason: "not headed".to_owned(),
            }),
            Box::new(|error| matches!(error, ActionError::Internal { .. })),
        ),
        (
            SessionError::Capacity {
                detail: "full".to_owned(),
            },
            Box::new(|error| matches!(error, ActionError::Internal { .. })),
        ),
        (
            SessionError::Internal {
                detail: "bug".to_owned(),
            },
            Box::new(|error| matches!(error, ActionError::Internal { .. })),
        ),
        (
            SessionError::NoOpenPage,
            Box::new(|error| matches!(error, ActionError::NotInteractable { .. })),
        ),
    ];
    for (source, expect) in cases {
        let mapped = super::as_action_error(&session, &source);
        assert!(
            expect(&mapped),
            "the mapping covers every manager-level failure: {source:?} -> {mapped:?}"
        );
    }
}

#[tokio::test]
async fn a_denied_effect_names_its_own_reference() {
    // The reference in the refusal names the element that was refused;
    // operations without a target keep the empty reference.
    async fn refusal(action: rutter_core::action::Action) -> ActionError {
        let context = MockContext::new();
        let session = Session::new(
            rutter_core::ids::SessionId::new("s-deny"),
            Arc::new(context),
            Arc::new(rutter_events::Backbone::new()),
            SessionConfig::default(),
            Arc::new(RuleSet::new(Vec::new(), Verdict::Deny)),
            Arc::new(ApprovalBroker::new()),
            None,
        );
        session.active_page_for_test().await;
        match session
            .execute(action, rutter_core::action::Origin::Agent)
            .await
        {
            Err(SessionError::Action(error @ ActionError::ApprovalDenied { .. })) => error,
            other => panic!("expected an approval denial, got {other:?}"),
        }
    }

    let click = refusal(rutter_core::action::Action::Click {
        reference: rutter_core::reference::Reference::new("e9"),
    })
    .await;
    assert!(
        matches!(
            click,
            ActionError::ApprovalDenied { ref reference } if reference.as_str() == "e9"
        ),
        "the refusal names the clicked element: {click:?}"
    );

    let scrolled = refusal(rutter_core::action::Action::Scroll {
        reference: Some(rutter_core::reference::Reference::new("e4")),
        direction: rutter_core::action::ScrollDirection::Down,
        amount: 100,
    })
    .await;
    assert!(
        matches!(
            scrolled,
            ActionError::ApprovalDenied { ref reference } if reference.as_str() == "e4"
        ),
        "a scroll refusal names the scrolled element: {scrolled:?}"
    );

    let keyed = refusal(rutter_core::action::Action::PressKey {
        key: "Enter".to_owned(),
    })
    .await;
    assert!(
        matches!(
            keyed,
            ActionError::ApprovalDenied { ref reference } if reference.as_str().is_empty()
        ),
        "a key press has no target element: {keyed:?}"
    );

    // Cookie denials carry the empty reference: a write names no element.
    let context = MockContext::new();
    let session = Session::new(
        rutter_core::ids::SessionId::new("s-deny"),
        Arc::new(context),
        Arc::new(rutter_events::Backbone::new()),
        SessionConfig::default(),
        Arc::new(RuleSet::new(Vec::new(), Verdict::Deny)),
        Arc::new(ApprovalBroker::new()),
        None,
    );
    session.active_page_for_test().await;
    let cookie = session
        .set_cookies(std::slice::from_ref(&session_cookie("s", "1")))
        .await;
    assert!(
        matches!(
            cookie,
            Err(SessionError::Action(ActionError::ApprovalDenied { .. }))
        ),
        "a denied cookie write fails, not passes: {cookie:?}"
    );
}
