//! The broadcast bus: fan-out to live subscribers.
//!
//! Boundary: tokio broadcast semantics only. `send` never blocks and
//! never waits for receivers; lagging subscribers lose old envelopes
//! (they read a `Lagged` error), which matches the rule that
//! only semantic events must survive — and they do, through the ring
//! buffers, not through the bus.

use tokio::sync::broadcast::{self, Receiver, Sender};

use crate::envelope::Envelope;

/// Capacity of the broadcast channel in envelopes. Subscribers that fall
/// further behind read a `Lagged` error and must resync via replay.
pub const CHANNEL_CAPACITY: usize = 1024;

/// Fan-out for live subscribers; publishing is fire-and-forget.
#[derive(Debug, Clone)]
pub struct EventBus {
    sender: Sender<Envelope>,
}

impl EventBus {
    /// Creates a bus with [`CHANNEL_CAPACITY`].
    pub fn new() -> Self {
        Self {
            sender: broadcast::channel(CHANNEL_CAPACITY).0,
        }
    }

    /// Publishes an envelope; succeeds even with no subscribers.
    pub fn publish(&self, envelope: Envelope) {
        // A send error only means every receiver is gone; the ring keeps
        // the envelope, so dropping the result is the fire-and-forget
        // contract (docs/events.md).
        let _ = self.sender.send(envelope);
    }

    /// Subscribes to live envelopes.
    pub fn subscribe(&self) -> Receiver<Envelope> {
        self.sender.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Event;
    use rutter_core::ids::SessionId;

    fn envelope(seq: u64) -> Envelope {
        Envelope {
            seq,
            recorded_at: "2026-09-17T00:00:00Z".to_owned(),
            session: SessionId::new("s1"),
            event: Event::SessionStarted,
        }
    }

    #[tokio::test]
    async fn subscribers_receive_published_envelopes() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();
        bus.publish(envelope(1));

        let received = receiver.recv().await.expect("envelope");
        assert_eq!(received.seq, 1);
    }

    #[tokio::test]
    async fn publishing_without_subscribers_never_fails() {
        let bus = EventBus::new();
        bus.publish(envelope(1));
        bus.publish(envelope(2));
    }

    #[tokio::test]
    async fn default_matches_new() {
        let default: EventBus = EventBus::default();
        let constructed = EventBus::new();
        let mut default_rx = default.subscribe();
        let mut constructed_rx = constructed.subscribe();
        default.publish(envelope(1));
        constructed.publish(envelope(1));
        assert_eq!(default_rx.recv().await.expect("envelope").seq, 1);
        assert_eq!(constructed_rx.recv().await.expect("envelope").seq, 1);
    }
}
