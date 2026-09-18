//! The per-session ring buffer: bounded history for replay.

use std::collections::VecDeque;

use crate::envelope::Envelope;

/// Bounded history of one session's envelopes, oldest first.
#[derive(Debug)]
pub struct RingBuffer {
    capacity: usize,
    entries: VecDeque<Envelope>,
}

impl RingBuffer {
    /// Creates a ring holding at most `capacity` envelopes.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: VecDeque::new(),
        }
    }

    /// Appends an envelope, evicting the oldest on overflow.
    pub fn push(&mut self, envelope: Envelope) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(envelope);
    }

    /// History of the ring, oldest first.
    pub fn history(&self) -> Vec<Envelope> {
        self.entries.iter().cloned().collect()
    }

    /// Number of envelopes currently held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the ring holds nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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

    #[test]
    fn replay_preserves_order_and_caps_history() {
        let mut ring = RingBuffer::new(3);
        for seq in 1..=5 {
            ring.push(envelope(seq));
        }

        let replayed = ring.history();
        assert_eq!(ring.len(), 3);
        let seqs: Vec<u64> = replayed.iter().map(|envelope| envelope.seq).collect();
        assert_eq!(seqs, vec![3, 4, 5], "oldest evicted, order preserved");
    }
}
