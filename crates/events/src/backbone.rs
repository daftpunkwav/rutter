//! The backbone: bus and rings behind one fire-and-forget front door.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::MutexGuard;

use rutter_core::ids::SessionId;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::broadcast::Receiver;

use crate::bus::EventBus;
use crate::envelope::Envelope;
use crate::event::Event;
use crate::ring::RingBuffer;

/// Default per-session history size.
pub const DEFAULT_RING_CAPACITY: usize = 1000;

/// The numbered history: a server-wide counter and the per-session rings
/// it indexes.
///
/// They live in one guard because numbering, recording, and fan-out are a
/// single decision. A counter taken before the ring insert lets two
/// publishers interleave so the ring reads `[newer, older]`, which breaks
/// replay's "oldest first" contract and makes a consumer that tracks the
/// highest sequence it has seen drop the older envelope as a duplicate.
#[derive(Debug)]
struct History {
    next_seq: u64,
    ring_capacity: usize,
    rings: HashMap<SessionId, RingBuffer>,
}

impl History {
    /// Numbers one event and records it in its session ring, returning the
    /// envelope for fan-out. The ring evicts its oldest entry past capacity.
    fn record(&mut self, session: SessionId, event: Event) -> Envelope {
        let envelope = Envelope {
            seq: self.next_seq,
            recorded_at: now_rfc3339(),
            session,
            event,
        };
        self.next_seq += 1;
        self.rings
            .entry(envelope.session.clone())
            .or_insert_with(|| RingBuffer::new(self.ring_capacity))
            .push(envelope.clone());
        envelope
    }
}

/// Publishes events to live subscribers and per-session history.
///
/// `publish` never blocks and never fails (docs/events.md): the envelope
/// always lands in the ring and reaches every live subscriber that keeps
/// up. Semantic events survive for late joiners through [`Backbone::replay`].
#[derive(Debug)]
pub struct Backbone {
    bus: EventBus,
    history: Mutex<History>,
}

impl Backbone {
    /// Creates a backbone with the default ring capacity.
    pub fn new() -> Self {
        Self::with_ring_capacity(DEFAULT_RING_CAPACITY)
    }

    /// Creates a backbone with an explicit per-session ring capacity.
    pub fn with_ring_capacity(capacity: usize) -> Self {
        Self {
            bus: EventBus::new(),
            history: Mutex::new(History {
                next_seq: 0,
                ring_capacity: capacity.max(1),
                rings: HashMap::new(),
            }),
        }
    }

    /// Records and fans out one event; fire-and-forget.
    pub fn publish(&self, session: SessionId, event: Event) {
        let mut history = self.lock_history();
        // Fan-out stays inside the guard for the same reason numbering and
        // recording share it: sending after the drop would let a
        // later-numbered envelope reach live subscribers first.
        // `EventBus::publish` never blocks, so this cannot widen the
        // critical section beyond the ring insert.
        self.bus.publish(history.record(session, event));
    }

    /// Subscribes to live envelopes.
    pub fn subscribe(&self) -> Receiver<Envelope> {
        self.bus.subscribe()
    }

    /// History of one session, oldest first; empty for unknown sessions.
    pub fn replay(&self, session: &SessionId) -> Vec<Envelope> {
        self.lock_history()
            .rings
            .get(session)
            .map(RingBuffer::history)
            .unwrap_or_default()
    }

    /// Drops a session's history. Called when a session closes: replay
    /// consumers only read open sessions, so the ring would otherwise sit
    /// unread in the map forever and a serve process that churns through
    /// session ids would grow the map without bound.
    pub fn forget(&self, session: &SessionId) {
        self.lock_history().rings.remove(session);
    }
}

impl Default for Backbone {
    fn default() -> Self {
        Self::new()
    }
}

impl Backbone {
    /// Locks the numbered history, recovering from poisoning: the rings
    /// stay usable even if a publisher panicked mid-publish.
    fn lock_history(&self) -> MutexGuard<'_, History> {
        self.history
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Current UTC time in RFC 3339; degrades to the epoch string rather
/// than failing a publish over clock trouble.
fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Event;

    #[tokio::test]
    async fn publish_reaches_subscriber_and_ring() {
        let backbone = Backbone::new();
        let session = SessionId::new("s1");
        let mut receiver = backbone.subscribe();

        backbone.publish(session.clone(), Event::SessionStarted);
        backbone.publish(session.clone(), Event::SessionClosed);

        let first = receiver.recv().await.expect("envelope");
        assert_eq!(first.seq, 0);
        assert_eq!(first.event, Event::SessionStarted);

        let replayed = backbone.replay(&session);
        let seqs: Vec<u64> = replayed.iter().map(|envelope| envelope.seq).collect();
        assert_eq!(seqs, vec![0, 1]);
        assert!(
            replayed[0].recorded_at.contains('T'),
            "timestamps are RFC 3339"
        );
    }

    #[tokio::test]
    async fn replay_is_per_session() {
        let backbone = Backbone::new();
        backbone.publish(SessionId::new("s1"), Event::SessionStarted);
        backbone.publish(SessionId::new("s2"), Event::SessionStarted);

        assert_eq!(backbone.replay(&SessionId::new("s1")).len(), 1);
        assert_eq!(backbone.replay(&SessionId::new("s2")).len(), 1);
        assert!(backbone.replay(&SessionId::new("s3")).is_empty());
    }

    #[test]
    fn sequence_numbers_are_monotonic_across_sessions() {
        let backbone = Backbone::new();
        backbone.publish(SessionId::new("s1"), Event::SessionStarted);
        backbone.publish(SessionId::new("s2"), Event::SessionStarted);

        let s1 = backbone.replay(&SessionId::new("s1"))[0].seq;
        let s2 = backbone.replay(&SessionId::new("s2"))[0].seq;
        assert!(s2 > s1);
    }

    #[test]
    fn concurrent_publishes_keep_ring_and_bus_in_allocation_order() {
        // Numbering, recording, and fan-out share one critical section.
        // Taking the counter outside it lets two publishers interleave so
        // the ring and the live stream disagree about order — and a
        // consumer that tracks the highest sequence it has seen then drops
        // the older envelope as a duplicate.
        const THREADS: usize = 8;
        const PER_THREAD: usize = 50;
        const TOTAL: u64 = (THREADS * PER_THREAD) as u64;

        let backbone = std::sync::Arc::new(Backbone::new());
        let session = SessionId::new("contended");
        let mut receiver = backbone.subscribe();
        let writers: Vec<_> = (0..THREADS)
            .map(|_| {
                let backbone = std::sync::Arc::clone(&backbone);
                let session = session.clone();
                std::thread::spawn(move || {
                    for _ in 0..PER_THREAD {
                        backbone.publish(session.clone(), Event::SessionStarted);
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().expect("publisher thread");
        }

        let expected: Vec<u64> = (0..TOTAL).collect();
        let ring: Vec<u64> = backbone
            .replay(&session)
            .iter()
            .map(|envelope| envelope.seq)
            .collect();
        assert_eq!(ring, expected, "the ring records in allocation order");

        let mut live: Vec<u64> = Vec::new();
        while let Ok(envelope) = receiver.try_recv() {
            live.push(envelope.seq);
        }
        assert_eq!(live, expected, "the bus fans out in the same order");
    }

    #[test]
    fn ring_capacity_is_honored_per_session() {
        let backbone = Backbone::with_ring_capacity(2);
        let session = SessionId::new("s1");
        for _ in 0..5 {
            backbone.publish(session.clone(), Event::SessionStarted);
        }
        assert_eq!(backbone.replay(&session).len(), 2);
    }

    #[tokio::test]
    async fn forget_drops_the_history_but_not_live_delivery() {
        let backbone = Backbone::new();
        let session = SessionId::new("s1");

        backbone.publish(session.clone(), Event::SessionStarted);
        assert!(!backbone.replay(&session).is_empty());

        // The subscriber joins before the forget: forgetting history
        // must not disturb the live bus it is parked on.
        let mut subscriber = backbone.subscribe();
        backbone.forget(&session);
        assert!(backbone.replay(&session).is_empty());

        backbone.publish(session.clone(), Event::SessionClosed);
        assert_eq!(
            subscriber.recv().await.expect("envelope").event,
            Event::SessionClosed
        );
    }

    #[tokio::test]
    async fn lagged_subscriber_can_resync_from_rings() {
        // Premise of the dashboard's Lagged handling: a subscriber that
        // fell so far behind the bus lost envelopes, but the per-session
        // rings still hold the semantic events it missed, so a replay
        // after the Lagged error fills the gap.
        let backbone = Backbone::new();
        let session = SessionId::new("s1");
        let mut subscriber = backbone.subscribe();

        // Overflow the broadcast channel with a session this subscriber
        // then stops reading; more publishes than CHANNEL_CAPACITY force
        // the receiver into a Lagged state.
        let flood = SessionId::new("flood");
        for _ in 0..(crate::bus::CHANNEL_CAPACITY * 2) {
            backbone.publish(flood.clone(), Event::SessionStarted);
        }
        backbone.publish(session.clone(), Event::SessionClosed);

        // The subscriber misses the SessionClosed live envelope (Lagged
        // on the next recv), but replay still delivers it.
        loop {
            match subscriber.try_recv() {
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => break,
                Err(error) => panic!("expected Lagged, got {error:?}"),
            }
        }
        let replayed = backbone.replay(&session);
        assert_eq!(replayed.len(), 1, "the ring kept the missed event");
        assert_eq!(replayed[0].event, Event::SessionClosed);
    }
}
