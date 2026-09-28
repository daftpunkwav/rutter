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
    session
        .open_page(Some("https://a.example".to_owned()))
        .await
        .expect("one tab");

    // A directory where the file belongs: the write cannot rename onto
    // it, which is the shape of a write that failed for any other
    // reason too (a full disk, a read-only state directory).
    std::fs::create_dir(&path).expect("block the state file");
    session
        .save_storage()
        .await
        .expect("a failed write is not an error");
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
