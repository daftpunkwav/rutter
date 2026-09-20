//! The envelope: an event plus its identity and recording time.

use serde::{Deserialize, Serialize};

use rutter_core::ids::SessionId;

use crate::event::Event;

/// One published event, stamped with a server-wide sequence number and
/// the RFC 3339 UTC time it was recorded (docs/glossary.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// Server-wide monotonic sequence number.
    pub seq: u64,
    /// RFC 3339 UTC recording time.
    pub recorded_at: String,
    /// Session the event belongs to.
    pub session: SessionId,
    /// The event itself.
    pub event: Event,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Event;

    #[test]
    fn envelopes_round_trip_through_json() {
        let envelope = Envelope {
            seq: 7,
            recorded_at: "2026-09-17T00:00:00Z".to_owned(),
            session: SessionId::new("s1"),
            event: Event::SessionStarted,
        };
        let json = serde_json::to_string(&envelope).expect("serialize");
        let back: Envelope = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, envelope);
    }
}
