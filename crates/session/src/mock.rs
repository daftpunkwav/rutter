//! Test doubles for the engine traits (compiled for tests only).
//!
//! The page mock answers `evaluate` by recognizing the marker text of
//! the `rutter-observe` page scripts, so session-layer tests run without
//! a browser. Answers are plain JSON matching each script's contract
//! (resolver box, wait flag, serializer envelope).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use serde_json::{Value, json};

use rutter_core::cookie::Cookie;
use rutter_core::ids::{ContextId, PageId};
use rutter_engine::context::{ContextHandle, ForeignPage};
use rutter_engine::error::EngineError;
use rutter_engine::input::InputEvent;
use rutter_engine::page::{ImageFormat, PageHandle, ScreencastStream, Screenshot};

fn lock<T, R>(mutex: &Mutex<T>, f: impl FnOnce(&mut T) -> R) -> R {
    let mut guard = mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

/// A resolver-script answer for a live element box.
pub fn mock_box(hidden: bool, disabled: bool, x: f64, y: f64, width: f64, height: f64) -> Value {
    json!({
        "missing": false,
        "x": x, "y": y, "width": width, "height": height,
        "disabled": disabled, "hidden": hidden
    })
}

/// The ready box the mocks answer with once the queued answers run out.
fn ready_box() -> Value {
    mock_box(false, false, 10.0, 20.0, 100.0, 30.0)
}

/// A resolver-script answer meaning the reference no longer resolves.
pub fn missing_answer() -> Value {
    json!({ "missing": true })
}

#[derive(Default)]
struct PageInner {
    resolve_answers: Mutex<VecDeque<Value>>,
    default_answer: Mutex<Option<Value>>,
    cycle: AtomicBool,
    url: Mutex<String>,
    found: AtomicBool,
    inputs: Mutex<Vec<InputEvent>>,
}

/// A scriptable page handle. Queued resolve answers are consumed one per
/// resolver-script evaluation; the last one repeats (or the whole queue
/// loops when cycling is on) so tests never run dry.
#[derive(Clone, Default)]
pub struct MockPage {
    inner: Arc<PageInner>,
}

impl MockPage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues one answer for the next resolver-script evaluations.
    pub fn push_resolve_answer(&self, answer: Value) {
        lock(&self.inner.resolve_answers, |answers| {
            answers.push_back(answer)
        });
    }

    /// Sets the answer used once the queue runs dry (default: a ready
    /// element box).
    pub fn set_default_answer(&self, answer: Value) {
        lock(&self.inner.default_answer, |default| {
            *default = Some(answer)
        });
    }

    /// Loops the queued answers instead of consuming them.
    pub fn set_cycle(&self, cycle: bool) {
        self.inner.cycle.store(cycle, Ordering::SeqCst);
    }

    /// Sets the URL `location.href` reports.
    pub fn set_url(&self, url: &str) {
        lock(&self.inner.url, |current| *current = url.to_owned());
    }

    /// Sets what the text-wait script reports.
    pub fn set_found(&self, found: bool) {
        self.inner.found.store(found, Ordering::SeqCst);
    }

    /// The input events dispatched so far, in order.
    pub fn inputs(&self) -> Vec<InputEvent> {
        lock(&self.inner.inputs, |inputs| inputs.clone())
    }

    fn next_resolve_answer(&self) -> Value {
        lock(&self.inner.resolve_answers, |answers| {
            if self.inner.cycle.load(Ordering::SeqCst) && !answers.is_empty() {
                let head = answers.pop_front().expect("non-empty queue");
                answers.push_back(head.clone());
                return head;
            }
            match answers.pop_front() {
                Some(answer) => answer,
                // Queue exhausted: fall back to the configured default
                // (a ready element box) so tests never run dry.
                None => lock(&self.inner.default_answer, |default| {
                    default.clone().unwrap_or_else(ready_box)
                }),
            }
        })
    }
}

#[async_trait]
impl PageHandle for MockPage {
    async fn navigate(&self, url: &str) -> Result<String, EngineError> {
        self.set_url(url);
        Ok(url.to_owned())
    }

    async fn reload(&self) -> Result<(), EngineError> {
        Ok(())
    }

    async fn go_back(&self) -> Result<String, EngineError> {
        Ok(lock(&self.inner.url, |url| url.clone()))
    }

    async fn go_forward(&self) -> Result<String, EngineError> {
        Ok(lock(&self.inner.url, |url| url.clone()))
    }

    async fn evaluate(&self, expression: &str) -> Result<Value, EngineError> {
        // Dispatch on the markers each rutter-observe script carries.
        // Scripts share substrings (the select script carries both
        // `var REF = ` and `var VALUES = `, the serializer carries
        // `innerWidth`), so the most specific markers are matched
        // first.
        if expression.contains("var VALUES = ") {
            Ok(json!({ "missing": false, "not_select": false, "matched": 1 }))
        } else if expression.contains("var REF = ") {
            Ok(self.next_resolve_answer())
        } else if expression.contains("var NEEDLE = ") {
            Ok(json!({ "found": self.inner.found.load(Ordering::SeqCst) }))
        } else if expression.contains("var MAX_NODES = ") {
            Ok(json!({
                "version": 1,
                "truncated": false,
                "root": {
                    "role": "root",
                    "children": [{ "role": "button", "name": "Ok", "ref": "e1" }]
                }
            }))
        } else if expression.contains("location.href") {
            Ok(json!(lock(&self.inner.url, |url| url.clone())))
        } else if expression.contains("innerWidth") {
            Ok(json!({ "x": 400.0, "y": 300.0 }))
        } else if expression.contains("localStorage") {
            Ok(json!({ "unavailable": true }))
        } else {
            Ok(Value::Null)
        }
    }

    async fn dispatch_input(&self, event: InputEvent) -> Result<(), EngineError> {
        lock(&self.inner.inputs, |inputs| inputs.push(event));
        Ok(())
    }

    async fn capture_screenshot(&self) -> Result<Screenshot, EngineError> {
        Ok(Screenshot {
            format: ImageFormat::Png,
            data: vec![0; 1024],
        })
    }

    async fn start_screencast(&self) -> Result<ScreencastStream, EngineError> {
        let (_sender, receiver) = tokio::sync::mpsc::channel(1);
        Ok(ScreencastStream::new(receiver))
    }
}

struct ContextInner {
    id: ContextId,
    pages: Mutex<Vec<(PageId, Arc<MockPage>)>>,
    counter: AtomicU64,
    closed: AtomicBool,
    set_cookie_calls: Mutex<Vec<Vec<Cookie>>>,
    /// Page surfaces reported by [`ContextHandle::foreign_pages`] and
    /// moved into `pages` by [`ContextHandle::adopt_page`], the way the
    /// real context tracks engine-owned windows. The URL travels with
    /// the entry because `MockPage` has no public URL getter.
    foreign: Mutex<Vec<(PageId, String, Arc<MockPage>)>>,
    /// Names the next planted foreign surface uniquely.
    foreign_counter: AtomicU64,
    /// When set, `foreign_pages` answers the way a wedged engine does —
    /// an error — so tests can cover the degraded listing.
    fail_foreign: AtomicBool,
}

/// A context handle whose pages are [`MockPage`]s; clones share state,
/// so a test can hold one clone and observe closes through it.
#[derive(Clone)]
pub struct MockContext {
    inner: Arc<ContextInner>,
}

impl MockContext {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ContextInner {
                id: ContextId::new("ctx-mock"),
                pages: Mutex::new(Vec::new()),
                counter: AtomicU64::new(0),
                closed: AtomicBool::new(false),
                set_cookie_calls: Mutex::new(Vec::new()),
                foreign: Mutex::new(Vec::new()),
                foreign_counter: AtomicU64::new(0),
                fail_foreign: AtomicBool::new(false),
            }),
        }
    }

    /// The cookie batches handed to [`ContextHandle::set_cookies`].
    pub fn set_cookie_calls(&self) -> Vec<Vec<Cookie>> {
        lock(&self.inner.set_cookie_calls, |calls| calls.clone())
    }

    /// The typed mock page registered under `id`, if any.
    pub fn page_mock(&self, id: PageId) -> Option<Arc<MockPage>> {
        lock(&self.inner.pages, |pages| {
            pages
                .iter()
                .find(|(page_id, _)| *page_id == id)
                .map(|(_, handle)| Arc::clone(handle))
        })
    }

    /// Plants a page surface the context did not open, the way a
    /// `window.open` tab or a human's window appears in the engine.
    /// Returns the stable foreign id `tabs_list` reports.
    pub fn add_foreign_page(&self, url: &str) -> PageId {
        let page = Arc::new(MockPage::new());
        page.set_url(url);
        let serial = self.inner.foreign_counter.fetch_add(1, Ordering::SeqCst);
        let id = PageId::new(format!("target:foreign-{serial}"));
        lock(&self.inner.foreign, |foreign| {
            foreign.push((id.clone(), url.to_owned(), page))
        });
        id
    }

    /// Makes `foreign_pages` fail, the way a wedged engine does.
    pub fn fail_foreign_pages(&self, fail: bool) {
        self.inner.fail_foreign.store(fail, Ordering::SeqCst);
    }
}

impl Default for MockContext {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ContextHandle for MockContext {
    fn id(&self) -> ContextId {
        self.inner.id.clone()
    }

    fn pages(&self) -> Vec<PageId> {
        lock(&self.inner.pages, |pages| {
            pages.iter().map(|(id, _)| id.clone()).collect()
        })
    }

    async fn open_page(&self) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(EngineError::Terminated);
        }
        let serial = self.inner.counter.fetch_add(1, Ordering::SeqCst);
        let id = PageId::new(format!("{}:page-{}", self.inner.id, serial));
        let page = Arc::new(MockPage::new());
        lock(&self.inner.pages, |pages| {
            pages.push((id.clone(), Arc::clone(&page)))
        });
        Ok((id, page))
    }

    fn page(&self, id: PageId) -> Option<Arc<dyn PageHandle>> {
        lock(&self.inner.pages, |pages| {
            pages
                .iter()
                .find(|(page_id, _)| *page_id == id)
                .map(|(_, handle)| Arc::clone(handle) as Arc<dyn PageHandle>)
        })
    }

    async fn close_page(&self, id: PageId) -> Result<(), EngineError> {
        lock(&self.inner.pages, |pages| {
            pages.retain(|(page_id, _)| *page_id != id)
        });
        Ok(())
    }

    async fn set_cookies(&self, cookies: &[Cookie]) -> Result<(), EngineError> {
        lock(&self.inner.set_cookie_calls, |calls| {
            calls.push(cookies.to_vec())
        });
        Ok(())
    }

    async fn cookies(&self) -> Result<Vec<Cookie>, EngineError> {
        Ok(Vec::new())
    }

    async fn close(&self) -> Result<(), EngineError> {
        self.inner.closed.store(true, Ordering::SeqCst);
        lock(&self.inner.pages, |pages| pages.clear());
        Ok(())
    }

    async fn foreign_pages(&self) -> Result<Vec<ForeignPage>, EngineError> {
        if self.inner.fail_foreign.load(Ordering::SeqCst) {
            return Err(EngineError::Terminated);
        }
        Ok(lock(&self.inner.foreign, |foreign| {
            foreign
                .iter()
                .map(|(id, url, _)| ForeignPage {
                    id: id.clone(),
                    url: url.clone(),
                })
                .collect()
        }))
    }

    async fn adopt_page(&self, id: &PageId) -> Result<(PageId, Arc<dyn PageHandle>), EngineError> {
        let adopted = lock(&self.inner.foreign, |foreign| {
            foreign
                .iter()
                .position(|(foreign_id, _, _)| foreign_id == id)
                .map(|position| foreign.remove(position))
        });
        match adopted {
            Some((id, _url, page)) => {
                lock(&self.inner.pages, |pages| {
                    pages.push((id.clone(), Arc::clone(&page)))
                });
                Ok((id, page as Arc<dyn PageHandle>))
            }
            None => Err(EngineError::Internal {
                detail: format!("the engine reports no adoptable page '{id}'"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    //! The mock's own contract: every script marker dispatches where the
    //! session layer expects it, queued answers cycle or fall back as
    //! documented, and the context tracks closes so tests can observe
    //! them. A mock that lies here would invalidate every test built on it.

    use super::*;

    #[tokio::test]
    async fn navigation_updates_the_reported_url() {
        let page = MockPage::new();
        assert_eq!(
            page.navigate("https://a.example").await.unwrap(),
            "https://a.example"
        );
        page.reload().await.expect("reload");
        assert_eq!(page.go_back().await.unwrap(), "https://a.example");
        assert_eq!(page.go_forward().await.unwrap(), "https://a.example");
        // `location.href` reads the same stored URL.
        let href = page.evaluate("location.href").await.expect("evaluate");
        assert_eq!(href, json!("https://a.example"));
    }

    #[tokio::test]
    async fn resolver_answers_queue_default_and_cycle() {
        let page = MockPage::new();
        page.push_resolve_answer(json!({"missing": true}));
        page.push_resolve_answer(mock_box(true, false, 0.0, 0.0, 1.0, 1.0));
        let first = page.evaluate("var REF = 'e1';").await.expect("evaluate");
        assert_eq!(
            first,
            json!({"missing": true}),
            "the queue is consumed in order"
        );
        let second = page.evaluate("var REF = 'e1';").await.expect("evaluate");
        assert_eq!(second["hidden"], json!(true));

        // With nothing queued the documented default (a ready box) answers.
        let fallback = page.evaluate("var REF = 'e1';").await.expect("evaluate");
        assert_eq!(fallback["hidden"], json!(false));
        assert_eq!(fallback["disabled"], json!(false));

        // A custom default replaces the ready box.
        page.set_default_answer(missing_answer());
        let custom = page.evaluate("var REF = 'e1';").await.expect("evaluate");
        assert_eq!(custom, json!({"missing": true}));

        // Cycling loops the queue instead of draining it.
        let cycling = MockPage::new();
        cycling.set_cycle(true);
        cycling.push_resolve_answer(json!({"n": 1}));
        cycling.push_resolve_answer(json!({"n": 2}));
        assert_eq!(
            cycling.evaluate("var REF = 'e1';").await.unwrap(),
            json!({"n": 1})
        );
        assert_eq!(
            cycling.evaluate("var REF = 'e1';").await.unwrap(),
            json!({"n": 2})
        );
        assert_eq!(
            cycling.evaluate("var REF = 'e1';").await.unwrap(),
            json!({"n": 1}),
            "the queue loops"
        );
    }

    #[tokio::test]
    async fn every_observe_marker_dispatches_to_its_answer() {
        let page = MockPage::new();
        // The select script matches before the resolver script: it
        // carries both `var REF = ` and `var VALUES = `.
        let select = page
            .evaluate("var REF = 'e1'; var VALUES = ['a'];")
            .await
            .expect("evaluate");
        assert_eq!(select["matched"], json!(1));
        let wait = page.evaluate("var NEEDLE = 'x';").await.expect("evaluate");
        assert_eq!(wait["found"], json!(false));
        page.set_found(true);
        let wait = page.evaluate("var NEEDLE = 'x';").await.expect("evaluate");
        assert_eq!(wait["found"], json!(true));
        let serializer = page.evaluate("var MAX_NODES = 1;").await.expect("evaluate");
        assert_eq!(serializer["version"], json!(1));
        let viewport = page
            .evaluate("({ x: innerWidth / 2 })")
            .await
            .expect("evaluate");
        assert_eq!(viewport, json!({"x": 400.0, "y": 300.0}));
        let storage = page.evaluate("dump localStorage").await.expect("evaluate");
        assert_eq!(storage["unavailable"], json!(true));
        let unknown = page.evaluate("something else").await.expect("evaluate");
        assert_eq!(unknown, Value::Null);
    }

    #[tokio::test]
    async fn inputs_are_recorded_in_dispatch_order() {
        let page = MockPage::new();
        page.dispatch_input(InputEvent::InsertText {
            text: "hi".to_owned(),
        })
        .await
        .expect("dispatch");
        page.dispatch_input(InputEvent::KeyPressed {
            key: "Enter".to_owned(),
        })
        .await
        .expect("dispatch");
        let inputs = page.inputs();
        assert_eq!(inputs.len(), 2);
        assert!(matches!(inputs[0], InputEvent::InsertText { .. }));
        assert!(matches!(inputs[1], InputEvent::KeyPressed { .. }));
    }

    #[tokio::test]
    async fn capture_and_screencast_answer_minimally() {
        let page = MockPage::new();
        let shot = page.capture_screenshot().await.expect("capture");
        assert_eq!(shot.format, ImageFormat::Png);
        assert_eq!(shot.data.len(), 1024);
        let mut cast = page.start_screencast().await.expect("screencast");
        // The stream closes immediately: the mock never produces frames.
        assert!(cast.next_frame().await.is_none());
    }

    #[tokio::test]
    async fn the_context_tracks_pages_cookies_and_closes() {
        let context = MockContext::new();
        assert_eq!(context.id(), ContextId::new("ctx-mock"));
        let (first, _) = context.open_page().await.expect("first page");
        let (second, second_handle) = context.open_page().await.expect("second page");
        assert_eq!(context.pages().len(), 2);
        assert!(
            context.page(second.clone()).is_some(),
            "handles are reachable by id"
        );
        assert!(context.page_mock(second).is_some());

        let cookies = vec![Cookie {
            name: "s".to_owned(),
            value: "1".to_owned(),
            domain: "example.com".to_owned(),
            path: None,
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        }];
        context.set_cookies(&cookies).await.expect("set cookies");
        assert_eq!(context.set_cookie_calls(), vec![cookies.clone()]);
        assert!(context.cookies().await.expect("cookies").is_empty());

        context.close_page(first).await.expect("close page");
        assert_eq!(context.pages().len(), 1);

        context.close().await.expect("close");
        assert!(
            matches!(context.open_page().await, Err(EngineError::Terminated)),
            "a closed context refuses new pages"
        );
        assert!(context.pages().is_empty());
        let _ = second_handle;
    }
}
