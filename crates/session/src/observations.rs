//! Per-page observation feeds: dialogs the session answers and console
//! output the session keeps.
//!
//! Boundary: one background task per tracked page consumes the page's
//! [`ObservationStream`] (rutter-engine). A dialog is dismissed
//! immediately — an unanswered dialog wedges the page — and the
//! dismissal lands on the event backbone; console lines and uncaught
//! exceptions are kept in a bounded per-page buffer that
//! [`Session::console_messages`] drains. The feed is best-effort: a
//! backend that cannot observe pages simply produces no entries and no
//! events, and nothing here may assume otherwise.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use rutter_core::ids::{PageId, SessionId};
use rutter_engine::page::{
    ConsoleEntry, ConsoleLevel, ObservationStream, PageHandle, PageObservation, RequestEntry,
};
use rutter_events::{Backbone, Event};

/// Console entries kept per page; older entries fall off the front.
pub(crate) const FEED_CAPACITY: usize = 200;

/// Network entries kept per page; older entries fall off the front.
pub(crate) const REQUEST_CAPACITY: usize = 100;

/// One tracked page's feed: the bounded console buffer plus the handle
/// that stops the background task when the page leaves the session.
struct Feed {
    entries: VecDeque<ConsoleEntry>,
    requests: VecDeque<RequestEntry>,
    stop: tokio::task::AbortHandle,
}

/// The session's observation feeds, keyed by page id.
#[derive(Default)]
pub(crate) struct ObservationFeeds {
    feeds: Mutex<HashMap<PageId, Feed>>,
}

impl ObservationFeeds {
    /// Starts the background task that consumes `page`'s observations.
    /// Safe to call twice for one page: the first feed wins and the
    /// duplicate task is aborted before its first poll.
    pub(crate) fn spawn(
        self: &Arc<Self>,
        session: SessionId,
        page: PageId,
        handle: Arc<dyn PageHandle>,
        backbone: Arc<Backbone>,
    ) {
        let mut feeds = self.lock();
        let duplicate = feeds.contains_key(&page);
        let stop = tokio::spawn(run_feed(
            session,
            page.clone(),
            handle,
            backbone,
            Arc::clone(self),
        ))
        .abort_handle();
        if duplicate {
            stop.abort();
        } else {
            // The map entry exists before the task's first poll (spawn
            // never runs the task inline), so records from the task
            // always find their buffer.
            feeds.insert(
                page,
                Feed {
                    entries: VecDeque::new(),
                    requests: VecDeque::new(),
                    stop,
                },
            );
        }
    }

    /// Records one console entry; the buffer keeps the newest
    /// [`FEED_CAPACITY`] entries.
    pub(crate) fn record(&self, page: &PageId, entry: ConsoleEntry) {
        let mut feeds = self.lock();
        if let Some(feed) = feeds.get_mut(page) {
            push(&mut feed.entries, entry);
        }
    }

    /// The page's console entries, oldest first.
    pub fn entries(&self, page: &PageId) -> Vec<ConsoleEntry> {
        match self.lock().get(page) {
            Some(feed) => feed.entries.iter().cloned().collect(),
            None => Vec::new(),
        }
    }

    /// Records one finished network request; the buffer keeps the newest
    /// [`REQUEST_CAPACITY`] entries.
    pub(crate) fn record_request(&self, page: &PageId, entry: RequestEntry) {
        let mut feeds = self.lock();
        if let Some(feed) = feeds.get_mut(page) {
            if feed.requests.len() >= REQUEST_CAPACITY {
                feed.requests.pop_front();
            }
            feed.requests.push_back(entry);
        }
    }

    /// The page's network entries, oldest first.
    pub fn requests(&self, page: &PageId) -> Vec<RequestEntry> {
        match self.lock().get(page) {
            Some(feed) => feed.requests.iter().cloned().collect(),
            None => Vec::new(),
        }
    }

    /// Forgets one page's feed and stops its task; used when the page
    /// leaves the session.
    pub fn remove(&self, page: &PageId) {
        if let Some(feed) = self.lock().remove(page) {
            feed.stop.abort();
        }
    }

    /// Forgets every feed and stops its tasks; used when a page (or the
    /// whole session) is gone.
    pub fn clear(&self) {
        let mut feeds = self.lock();
        for (_, feed) in feeds.drain() {
            feed.stop.abort();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PageId, Feed>> {
        self.feeds
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Pushes an entry with the capacity bound; free-standing so the rule
/// is unit-testable without a feed map.
fn push(entries: &mut VecDeque<ConsoleEntry>, entry: ConsoleEntry) {
    if entries.len() >= FEED_CAPACITY {
        entries.pop_front();
    }
    entries.push_back(entry);
}

/// The background task for one page: dialogs are dismissed (the event
/// is recorded first, so the timeline keeps the fact even when the
/// dismissal fails on a page that just went away), console lines and
/// uncaught exceptions land in the page's buffer.
async fn run_feed(
    session: SessionId,
    page: PageId,
    handle: Arc<dyn PageHandle>,
    backbone: Arc<Backbone>,
    feeds: Arc<ObservationFeeds>,
) {
    let mut stream: ObservationStream = match handle.observe().await {
        Ok(stream) => stream,
        // A backend without observations, or a duplicate feed attempt:
        // nothing to consume, nothing to clean up.
        Err(_) => return,
    };
    while let Some(observation) = stream.next_observation().await {
        match observation {
            PageObservation::DialogOpened { kind, message } => {
                backbone.publish(
                    session.clone(),
                    Event::DialogAutoDismissed {
                        page: page.clone(),
                        kind: kind.as_str().to_owned(),
                        message,
                    },
                );
                // The dismissal answer is best-effort: a page that
                // died with the dialog open fails here, and that is
                // not a session-level event.
                let _ = handle.handle_dialog(false, None).await;
            }
            PageObservation::ConsoleEmitted { level, text } => {
                feeds.record(&page, ConsoleEntry { level, text });
            }
            PageObservation::UncaughtException { text } => {
                feeds.record(
                    &page,
                    ConsoleEntry {
                        level: ConsoleLevel::Error,
                        text,
                    },
                );
            }
            PageObservation::RequestObserved { entry } => {
                feeds.record_request(&page, entry);
            }
        }
    }
}

#[cfg(test)]
mod tests;
