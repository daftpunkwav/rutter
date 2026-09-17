//! Capped exponential backoff shared by network fetches and restarts.
//!
//! Boundary: pure arithmetic on attempt counts. Callers decide retry
//! policy (how many attempts, when to give up); this type only turns an
//! attempt number into a delay. Delays are deterministic in v1; jitter
//! is deliberately absent to keep tests exact.

use std::time::Duration;

/// Exponential backoff with a cap.
#[derive(Debug, Clone)]
pub struct Backoff {
    base: Duration,
    max: Duration,
}

impl Backoff {
    /// Creates a backoff that starts at `base` and never exceeds `max`.
    pub fn new(base: Duration, max: Duration) -> Self {
        Self { base, max }
    }

    /// Delay before retry number `attempt` (0-based): `base * 2^attempt`,
    /// clamped to `max`. Doubling is saturating, so huge attempt counts
    /// stay panic-free and simply return `max`.
    pub fn delay(&self, attempt: u32) -> Duration {
        let mut delay = self.base;
        for _ in 0..attempt.min(64) {
            if delay >= self.max {
                break;
            }
            match delay.checked_mul(2) {
                Some(doubled) => delay = doubled,
                None => break,
            }
        }
        delay.min(self.max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_grow_and_cap() {
        let backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(8));
        assert_eq!(backoff.delay(0), Duration::from_secs(2));
        assert_eq!(backoff.delay(1), Duration::from_secs(4));
        assert_eq!(backoff.delay(2), Duration::from_secs(8));
        assert_eq!(backoff.delay(10), Duration::from_secs(8));
    }

    #[test]
    fn huge_attempt_counts_stay_panic_free() {
        let backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(8));
        assert_eq!(backoff.delay(u32::MAX), Duration::from_secs(8));
    }
}
