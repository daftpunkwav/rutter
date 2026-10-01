//! Storage persistence: what reaches the file, and in what order.
//!
//! Driven against the crate's mock engine, so the engine round trips are
//! instant and controllable: the ordering test needs a capture that
//! parks mid-flight, which a real browser would only ever produce by
//! accident.

use super::*;
use crate::storage::StorageState;
use rutter_engine::error::EngineError;

/// A cookie whose value identifies the capture that read it.
fn cookie(value: &str) -> Cookie {
    Cookie {
        name: "session".to_owned(),
        value: value.to_owned(),
        domain: "example.com".to_owned(),
        path: Some("/".to_owned()),
        secure: false,
        http_only: false,
        same_site: None,
        expires: None,
    }
}

/// A context whose first `cookies()` read parks after taking the
/// cookies it read: the window in which a second, overlapping persist
/// used to capture, publish, and finish ahead of the parked one.
struct GatedCaptureContext {
    cookies: Mutex<Vec<Cookie>>,
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl GatedCaptureContext {
    fn set_cookies(&self, cookies: Vec<Cookie>) {
        *self
            .cookies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = cookies;
    }
}

#[async_trait::async_trait]
impl ContextHandle for GatedCaptureContext {
    fn id(&self) -> ContextId {
        ContextId::new("ctx-gated")
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

    async fn set_cookies(&self, _cookies: &[Cookie]) -> Result<(), EngineError> {
        Ok(())
    }

    async fn cookies(&self) -> Result<Vec<Cookie>, EngineError> {
        let read = self
            .cookies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        // Read first, park second: what the capture holds is decided
        // before it can be overtaken.
        let gate = self.release.lock().await.take();
        if let Some(receiver) = gate {
            let _ = self.entered.send(());
            let _ = receiver.await;
        }
        Ok(read)
    }

    async fn close(&self) -> Result<(), EngineError> {
        Ok(())
    }
}

/// A session that persists what it captures, with two overlapping
/// captures: the parked one reads `v1`, the one that runs while it is
/// parked reads `v2`. The file must end up holding `v2`.
#[tokio::test]
async fn an_older_capture_never_publishes_over_a_newer_one() {
    // Capture, comparison, and write are one decision behind one
    // guard. Split apart, the parked capture published after the one
    // that overtook it and left the file holding older state than the
    // session had already recorded as persisted — a login that nothing
    // would ever rewrite again.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("s.storage.json");
    let (entered, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release, release_rx) = tokio::sync::oneshot::channel();
    let context = Arc::new(GatedCaptureContext {
        cookies: Mutex::new(vec![cookie("v1")]),
        entered,
        release: tokio::sync::Mutex::new(Some(release_rx)),
    });
    let session = Arc::new(session_persisting_to(
        Arc::clone(&context) as Arc<dyn ContextHandle>,
        path.clone(),
    ));

    let parked = tokio::spawn({
        let session = Arc::clone(&session);
        async move { session.persist_storage().await }
    });
    entered_rx
        .recv()
        .await
        .expect("the parked capture reached its cookie read");

    context.set_cookies(vec![cookie("v2")]);
    let overtaking = tokio::spawn({
        let session = Arc::clone(&session);
        async move { session.persist_storage().await }
    });
    // Let the overtaking capture run as far as it can before the parked
    // one is released: without the shared guard it completes here, and
    // the parked capture lands on top of its result.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    release.send(()).expect("release the parked capture");

    parked.await.expect("the parked persist finished");
    overtaking.await.expect("the overtaking persist finished");

    let state = StorageState::read(&path);
    assert_eq!(
        state.cookies[0].value, "v2",
        "the file must hold the newest capture, not the last writer"
    );
}

/// One storage capture per action, whatever page the action ran on: a
/// session's state is every tab's, so persisting only the acting page's
/// localStorage silently dropped the other origins on every write.
#[tokio::test]
async fn every_open_tab_reaches_the_persisted_state() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("s.storage.json");
    let context = Arc::new(MockContext::new());
    let session =
        session_persisting_to(Arc::clone(&context) as Arc<dyn ContextHandle>, path.clone());

    session
        .open_page(Some("https://a.example".to_owned()))
        .await
        .expect("first tab");
    session
        .open_page(Some("https://b.example".to_owned()))
        .await
        .expect("second tab");
    for page in session.pages().await {
        context
            .page_mock(page.id.clone())
            .expect("the mock page")
            .set_storage(&page.url, &[("token", "kept")]);
    }

    session.save_storage().await.expect("save");

    let state = StorageState::read(&path);
    let origins: Vec<&str> = state
        .origins
        .iter()
        .map(|origin| origin.origin.as_str())
        .collect();
    assert_eq!(
        origins,
        vec!["https://a.example", "https://b.example"],
        "every tracked origin survives one persistence run"
    );
}

/// A tabs_open navigation persists on change like an executed action:
/// the storage a page already carried reaches the file when the next
/// tabs_open runs its persist-on-change pass, without an explicit
/// save — matching what `Session::execute` does after a `navigate`.
#[tokio::test]
async fn a_tabs_open_navigation_persists_on_change() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("s.storage.json");
    let context = Arc::new(MockContext::new());
    let session =
        session_persisting_to(Arc::clone(&context) as Arc<dyn ContextHandle>, path.clone());

    session
        .open_page(Some("https://opened.example".to_owned()))
        .await
        .expect("first tab");
    for page in session.pages().await {
        context
            .page_mock(page.id.clone())
            .expect("the mock page")
            .set_storage(&page.url, &[("token", "kept")]);
    }

    // The second open's persist-on-change pass is what must publish the
    // first page's storage; the first open captured before the mock
    // carried anything.
    session
        .open_page(Some("https://other.example".to_owned()))
        .await
        .expect("second tab");

    let state = StorageState::read(&path);
    let origins: Vec<&str> = state
        .origins
        .iter()
        .map(|origin| origin.origin.as_str())
        .collect();
    assert_eq!(
        origins,
        vec!["https://opened.example"],
        "tabs_open's navigation persists the storage it changed"
    );
}

/// A context whose cookie read can be made to fail: the shape of an
/// engine that died (or wedged) between two persistence runs. Its one
/// page is a mock, so `save_storage`'s `ensure_page` still opens one.
struct SwitchableJar {
    page: MockPage,
    cookies: Mutex<Vec<Cookie>>,
    fail: std::sync::atomic::AtomicBool,
}

impl SwitchableJar {
    fn set_fail(&self, fail: bool) {
        self.fail.store(fail, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl ContextHandle for SwitchableJar {
    fn id(&self) -> ContextId {
        ContextId::new("ctx-jar")
    }

    fn pages(&self) -> Vec<PageId> {
        Vec::new()
    }

    async fn open_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
        Ok((
            PageId::new("ctx-jar:page-0"),
            Arc::new(self.page.clone()) as Arc<dyn PageHandle>,
        ))
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
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(EngineError::Terminated);
        }
        Ok(self
            .cookies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone())
    }

    async fn close(&self) -> Result<(), EngineError> {
        Ok(())
    }
}

/// A capture whose cookie read fails must not be published at all: the
/// empty state it would degrade to overwrites the last known one — the
/// state recovery replays after an engine restart — and the session's
/// login is gone. The known state stands until a readable capture
/// replaces it.
#[tokio::test]
async fn a_failed_cookie_read_keeps_the_last_known_state() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("s.storage.json");
    let jar = Arc::new(SwitchableJar {
        page: MockPage::new(),
        cookies: Mutex::new(vec![cookie("v1")]),
        fail: std::sync::atomic::AtomicBool::new(false),
    });
    let session = session_persisting_to(Arc::clone(&jar) as Arc<dyn ContextHandle>, path.clone());

    session.save_storage().await.expect("first save");
    assert_eq!(
        StorageState::read(&path).cookies[0].value,
        "v1",
        "the readable capture reaches the file"
    );

    jar.set_fail(true);
    session
        .save_storage()
        .await
        .expect("a failed read is not an error");
    assert_eq!(
        StorageState::read(&path).cookies[0].value,
        "v1",
        "the failed read must not overwrite the file with an empty state"
    );
    assert!(
        session.last_storage_is_persisted().await,
        "the file is still known to hold the last good state"
    );
}

/// A capture identical to the one already on disk is not rewritten —
/// unless the last write failed. "Persist on change" then has to
/// publish again, or a state the file never received is remembered as
/// persisted forever.
#[tokio::test]
async fn a_failed_write_is_retried_on_the_next_capture() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("s.storage.json");
    let context = Arc::new(MockContext::new());
    let session =
        session_persisting_to(Arc::clone(&context) as Arc<dyn ContextHandle>, path.clone());
    // A blank tab: no navigation, so tabs_open's persist-on-change pass
    // has nothing to publish and the state file does not exist yet —
    // the test needs the first blocked write to be the explicit save.
    session.open_page(None).await.expect("one tab");

    // A directory where the file belongs: the write cannot rename onto
    // it, which is the shape of a write that failed for any other
    // reason too (a full disk, a read-only state directory). The
    // explicit save is the caller's lens on that failure: it comes
    // back as `StorageWrite`, and `persisted` stays false so the
    // retry below has something to publish.
    std::fs::create_dir(&path).expect("block the state file");
    let error = session
        .save_storage()
        .await
        .expect_err("a failed write is the caller's to see");
    assert!(
        matches!(error, SessionError::StorageWrite { .. }),
        "the failure names the write: {error:?}"
    );
    assert!(
        !path.join("x").is_file(),
        "the blocked path is a directory, not a state file"
    );

    std::fs::remove_dir(&path).expect("unblock the state file");
    session.save_storage().await.expect("second save");
    assert!(
        path.is_file(),
        "an unchanged state still republishes after a failed write"
    );
}
