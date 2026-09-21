//! The page registry: which pages a session tracks and which is active.
//!
//! Responsibility: own the slot list and the lock around it, and hold the
//! two invariants every caller relies on — at most one slot is active, and
//! a slot leaves the list only through an operation on this registry.
//!
//! Boundary: bookkeeping only. Opening, closing, and navigating are engine
//! operations; this module never calls one. It answers "is there an active
//! page", "what URL is tracked for this id", and "what must recovery
//! rebuild".
//!
//! Why recovery carries a gate: rebuilding used to take the whole list out
//! and write a new one back some seconds later. A lookup during that window
//! saw an empty list, opened a page of its own and registered it — and the
//! write-back then dropped that slot from the registry while its tab stayed
//! alive in the engine, untracked, capped, and invisible to `tabs_list`.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;

use rutter_core::ids::PageId;
use rutter_engine::page::PageHandle;

/// One tracked page with its last known URL.
#[derive(Clone)]
pub(crate) struct PageSlot {
    /// Stable page identifier.
    pub id: PageId,
    /// Last URL known for this slot.
    pub url: String,
    /// The handle that reaches the page.
    pub handle: Arc<dyn PageHandle>,
    /// Whether this slot is the session's active page.
    pub active: bool,
}

/// The open pages as reported to clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageInfo {
    /// Stable page identifier.
    pub id: PageId,
    /// Last known URL.
    pub url: String,
    /// Whether this page is the session's active page.
    pub active: bool,
}

/// What recovery reads out: where the session was, not which handle got
/// there. The handles of a dead engine are not worth carrying.
#[derive(Clone)]
pub(crate) struct RecoveryPoint {
    /// URL to reopen the slot at.
    pub url: String,
    /// Whether this point was the active one.
    pub active: bool,
}

/// The registry state, guarded as one unit: the gate and the list it
/// protects must never be observed out of step.
struct State {
    slots: Vec<PageSlot>,
    recovering: bool,
}

/// The session's tracked pages.
pub(crate) struct PageRegistry {
    state: Mutex<State>,
}

impl PageRegistry {
    /// An empty registry; a session starts with no pages until the first
    /// action asks for one.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                slots: Vec::new(),
                recovering: false,
            }),
        }
    }

    /// The active page, if one exists. Never opens anything: this is the
    /// lookup an observation has to use, because opening a tab is an
    /// effect (docs/architecture.md: the dashboard executes no actions).
    pub fn active(&self) -> Option<(PageId, Arc<dyn PageHandle>)> {
        let state = self.lock();
        if state.recovering {
            return None;
        }
        state
            .slots
            .iter()
            .find(|slot| slot.active)
            .map(|slot| (slot.id.clone(), Arc::clone(&slot.handle)))
    }

    /// Whether an action may register a page it just opened. `false` while
    /// recovery owns the list, so the caller fails fast instead of adding a
    /// slot that the rebuild would orphan.
    pub fn accepts_new_page(&self) -> bool {
        !self.lock().recovering
    }

    /// Registers a page the caller just opened, as the active one.
    ///
    /// The registry admits it only when no slot holds the active flag and
    /// no recovery is in flight. A `false` answer means the caller owns a
    /// page nothing tracks, and closing it is the caller's job: this is how
    /// two racing `ensure_page` calls end with one page, and how a page
    /// opened during recovery ends up closed rather than leaked.
    pub fn admit_active(&self, slot: PageSlot) -> bool {
        let mut state = self.lock();
        if state.recovering || state.slots.iter().any(|slot| slot.active) {
            return false;
        }
        state.slots.push(PageSlot {
            active: true,
            ..slot
        });
        true
    }

    /// The tracked URL of one page.
    pub fn url_of(&self, id: &PageId) -> Option<String> {
        self.lock()
            .slots
            .iter()
            .find(|slot| slot.id == *id)
            .map(|slot| slot.url.clone())
    }

    /// Refreshes the tracked URL of every slot with this id. Filtering by
    /// id, not by the current active flag, because a page switch may have
    /// moved the flag while an action was in flight.
    pub fn set_url(&self, id: &PageId, url: &str) {
        for slot in &mut self.lock().slots {
            if slot.id == *id {
                slot.url = url.to_owned();
            }
        }
    }

    /// Whether the registry tracks this page at all.
    pub fn contains(&self, id: &PageId) -> bool {
        self.lock().slots.iter().any(|slot| slot.id == *id)
    }

    /// The open pages, in registration order.
    pub fn list(&self) -> Vec<PageInfo> {
        self.lock()
            .slots
            .iter()
            .map(|slot| PageInfo {
                id: slot.id.clone(),
                url: slot.url.clone(),
                active: slot.active,
            })
            .collect()
    }

    /// Makes one page active and returns its handle; `None` when the id is
    /// not tracked, in which case nothing changed.
    pub fn activate(&self, id: &PageId) -> Option<Arc<dyn PageHandle>> {
        let mut state = self.lock();
        let found = state.slots.iter().any(|slot| slot.id == *id);
        if !found {
            return None;
        }
        for slot in &mut state.slots {
            slot.active = slot.id == *id;
        }
        state
            .slots
            .iter()
            .find(|slot| slot.id == *id)
            .map(|slot| Arc::clone(&slot.handle))
    }

    /// Removes a tracked page, promoting the first remaining slot when the
    /// active page was the one taken. Returns the removed slot so the
    /// caller can close its engine page.
    pub fn remove(&self, id: &PageId) -> Option<PageSlot> {
        let mut state = self.lock();
        let was_active = state
            .slots
            .iter()
            .position(|slot| slot.id == *id)
            .map(|position| state.slots[position].active);
        let removed = state
            .slots
            .iter()
            .position(|slot| slot.id == *id)
            .map(|position| state.slots.remove(position));
        if was_active == Some(true) && !state.slots.is_empty() {
            state.slots[0].active = true;
        }
        removed
    }

    /// The tracked pages as `(url, handle)` pairs, the shape storage-state
    /// capture and restore consume.
    pub fn pairs(&self) -> Vec<(String, Arc<dyn PageHandle>)> {
        self.lock()
            .slots
            .iter()
            .map(|slot| (slot.url.clone(), Arc::clone(&slot.handle)))
            .collect()
    }

    /// Takes the list over for recovery: lookups stop answering, new
    /// registrations are refused, and the URLs to rebuild come back.
    pub fn begin_recovery(&self) -> Vec<RecoveryPoint> {
        let mut state = self.lock();
        state.recovering = true;
        state
            .slots
            .iter()
            .map(|slot| RecoveryPoint {
                url: slot.url.clone(),
                active: slot.active,
            })
            .collect()
    }

    /// Installs the rebuilt list and reopens the registry to lookups. An
    /// empty rebuild leaves the session without an active page, which is
    /// the same state a fresh session starts in.
    pub fn finish_recovery(&self, slots: Vec<PageSlot>) {
        let mut state = self.lock();
        state.slots = slots;
        if !state.slots.is_empty() && !state.slots.iter().any(|slot| slot.active) {
            state.slots[0].active = true;
        }
        state.recovering = false;
    }

    /// Forgets every page; the session is closing.
    pub fn clear(&self) {
        let mut state = self.lock();
        state.slots.clear();
        state.recovering = false;
    }

    /// Test-only: appends a slot directly, in the shape
    /// [`PageRegistry::finish_recovery`] produces. Production reaches a
    /// second page through recovery or the engine itself, never by
    /// appending.
    #[cfg(test)]
    pub fn append_for_test(&self, slot: PageSlot) {
        self.lock().slots.push(slot);
    }

    /// Locks the state, recovering from poisoning: the list stays usable
    /// even if a caller panicked while holding it.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for PageRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    //! Registry mechanics, one behavior per test: the gate recovery holds,
    //! the single-active invariant, and the promotion rules on removal and
    //! rebuild. The doubles are the crate's own mock handles.

    use super::*;
    use crate::mock::MockPage;

    fn slot(id: &str, url: &str, active: bool) -> PageSlot {
        PageSlot {
            id: PageId::new(id),
            url: url.to_owned(),
            handle: Arc::new(MockPage::new()),
            active,
        }
    }

    #[test]
    fn a_fresh_registry_has_no_active_page() {
        let registry = PageRegistry::new();
        assert!(registry.active().is_none());
        assert!(registry.accepts_new_page());
        assert!(registry.list().is_empty());
        assert!(registry.pairs().is_empty());
    }

    #[test]
    fn active_lookup_refuses_while_recovery_owns_the_list() {
        let registry = PageRegistry::new();
        registry.admit_active(slot("p1", "https://a.example", true));
        let saved = registry.begin_recovery();
        assert_eq!(saved.len(), 1, "the read-out carries the tracked urls");
        assert!(saved[0].active);
        assert!(
            registry.active().is_none(),
            "lookups stop answering during the rebuild"
        );
        assert!(
            !registry.accepts_new_page(),
            "new registrations are refused during the rebuild"
        );
        registry.finish_recovery(vec![]);
        assert!(
            registry.active().is_none(),
            "an empty rebuild leaves no active page, like a fresh session"
        );
        assert!(registry.accepts_new_page());
    }

    #[test]
    fn admit_active_refuses_a_second_active_slot() {
        let registry = PageRegistry::new();
        assert!(registry.admit_active(slot("p1", "https://a.example", false)));
        assert!(
            !registry.admit_active(slot("p2", "https://b.example", false)),
            "two racing opens end with one admitted page"
        );
        assert_eq!(registry.list().len(), 1);
    }

    #[test]
    fn finish_recovery_promotes_the_first_slot_when_none_is_active() {
        let registry = PageRegistry::new();
        registry.admit_active(slot("p1", "https://a.example", true));
        registry.begin_recovery();
        let mut rebuilt = slot("p2", "https://b.example", false);
        rebuilt.active = false;
        registry.finish_recovery(vec![rebuilt]);
        let listed = registry.list();
        assert!(listed[0].active, "exactly one page ends up active");
    }

    #[test]
    fn removal_promotes_the_first_remaining_page() {
        let registry = PageRegistry::new();
        registry.admit_active(slot("p1", "https://a.example", true));
        registry.append_for_test(slot("p2", "https://b.example", false));
        let removed = registry.remove(&PageId::new("p1"));
        assert!(removed.is_some());
        let listed = registry.list();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].active, "the survivor takes the active flag");
        assert!(registry.remove(&PageId::new("nope")).is_none());
    }

    #[test]
    fn urls_are_tracked_per_id_and_reported_as_pairs() {
        let registry = PageRegistry::new();
        registry.admit_active(slot("p1", "https://a.example", true));
        registry.set_url(&PageId::new("p1"), "https://a.example/next");
        assert_eq!(
            registry.url_of(&PageId::new("p1")).as_deref(),
            Some("https://a.example/next")
        );
        assert!(registry.url_of(&PageId::new("nope")).is_none());
        assert!(registry.contains(&PageId::new("p1")));
        assert!(!registry.contains(&PageId::new("nope")));
        let pairs = registry.pairs();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "https://a.example/next");
    }

    #[test]
    fn activate_unknown_ids_change_nothing() {
        let registry = PageRegistry::new();
        registry.admit_active(slot("p1", "https://a.example", true));
        assert!(registry.activate(&PageId::new("nope")).is_none());
        let handle = registry.activate(&PageId::new("p1"));
        assert!(handle.is_some());
        assert!(registry.list()[0].active);
    }

    #[test]
    fn clear_forgets_every_page_and_the_gate_with_it() {
        let registry = PageRegistry::new();
        registry.admit_active(slot("p1", "https://a.example", true));
        registry.begin_recovery();
        registry.clear();
        assert!(registry.list().is_empty());
        assert!(registry.accepts_new_page());
        assert!(registry.active().is_none());
    }
}
