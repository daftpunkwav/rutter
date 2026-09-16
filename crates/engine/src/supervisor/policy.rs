//! Restart policy: capped backoff plus a window-based circuit breaker.
//!
//! Boundary: pure decisions over restart timestamps. No I/O, no clock
//! access of its own — callers pass `Instant::now()` so tests stay
//! deterministic. The breaker implements the blueprint rule: too many
//! restarts within a time window fails callers with a clear error
//! instead of thrashing the host with relaunch attempts.

use std::time::{Duration, Instant};

use crate::backoff::Backoff;

/// What the policy says about starting another launch attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartDecision {
    /// An attempt may start after waiting out the backoff.
    Allowed(Duration),
    /// The breaker is open: too many restarts inside the window; the
    /// caller must stop retrying until the window drains.
    Open,
}

/// Restart attempt timestamps inside a sliding window.
#[derive(Debug, Default)]
pub struct RestartHistory {
    attempts: Vec<Instant>,
}

impl RestartHistory {
    /// Records one launch attempt.
    pub fn record(&mut self, now: Instant) {
        self.attempts.push(now);
    }

    /// Drops attempts older than `window`.
    pub fn prune(&mut self, now: Instant, window: Duration) {
        self.attempts
            .retain(|attempt| now.duration_since(*attempt) < window);
    }

    /// Number of attempts still inside the window.
    pub fn count_in_window(&mut self, now: Instant, window: Duration) -> usize {
        self.prune(now, window);
        self.attempts.len()
    }
}

/// Backoff and breaker parameters for one supervised engine.
#[derive(Debug, Clone)]
pub struct RestartPolicy {
    backoff: Backoff,
    max_restarts: u32,
    window: Duration,
}

impl RestartPolicy {
    /// Production defaults: 1 s..30 s backoff, breaker opens after
    /// `max_restarts` attempts within `window`.
    pub fn new(max_restarts: u32, window: Duration) -> Self {
        Self {
            backoff: Backoff::new(Duration::from_secs(1), Duration::from_secs(30)),
            max_restarts,
            window,
        }
    }

    /// Overrides the backoff shape; used by tests and callers hosting
    /// engines with unusual startup latency.
    pub fn with_backoff(
        max_restarts: u32,
        window: Duration,
        base: Duration,
        max: Duration,
    ) -> Self {
        Self {
            backoff: Backoff::new(base, max),
            max_restarts,
            window,
        }
    }

    /// Decides whether another launch attempt may start now. The first
    /// attempt inside an empty window starts immediately; later attempts
    /// wait out the backoff.
    pub fn decide(&self, history: &mut RestartHistory, now: Instant) -> RestartDecision {
        let in_window = history.count_in_window(now, self.window);
        if in_window >= self.max_restarts as usize {
            return RestartDecision::Open;
        }
        if in_window == 0 {
            return RestartDecision::Allowed(Duration::ZERO);
        }
        RestartDecision::Allowed(self.backoff.delay(in_window as u32 - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_attempt_starts_immediately_then_backs_off() {
        let mut history = RestartHistory::default();
        let policy = RestartPolicy::with_backoff(
            4,
            Duration::from_secs(60),
            Duration::from_millis(10),
            Duration::from_millis(40),
        );
        let now = Instant::now();

        assert_eq!(
            policy.decide(&mut history, now),
            RestartDecision::Allowed(Duration::ZERO)
        );
        history.record(now);
        assert_eq!(
            policy.decide(&mut history, now),
            RestartDecision::Allowed(Duration::from_millis(10))
        );
        history.record(now);
        assert_eq!(
            policy.decide(&mut history, now),
            RestartDecision::Allowed(Duration::from_millis(20))
        );
        history.record(now);
        assert_eq!(
            policy.decide(&mut history, now),
            RestartDecision::Allowed(Duration::from_millis(40))
        );
    }

    #[test]
    fn breaker_opens_after_max_restarts_in_window() {
        let mut history = RestartHistory::default();
        let policy = RestartPolicy::with_backoff(
            3,
            Duration::from_secs(60),
            Duration::from_millis(1),
            Duration::from_millis(1),
        );
        let now = Instant::now();
        for _ in 0..3 {
            history.record(now);
        }
        assert_eq!(policy.decide(&mut history, now), RestartDecision::Open);
    }

    #[test]
    fn breaker_reopens_after_window_drains() {
        let mut history = RestartHistory::default();
        let window = Duration::from_millis(50);
        let policy = RestartPolicy::with_backoff(
            2,
            window,
            Duration::from_millis(1),
            Duration::from_millis(1),
        );
        let now = Instant::now();
        history.record(now);
        history.record(now);
        assert_eq!(policy.decide(&mut history, now), RestartDecision::Open);

        let later = now + window + Duration::from_millis(1);
        assert_eq!(
            policy.decide(&mut history, later),
            RestartDecision::Allowed(Duration::ZERO)
        );
    }
}
