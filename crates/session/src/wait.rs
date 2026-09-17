//! Bounded polling for auto-wait phases and `wait_for`.
//!
//! Boundary: time-boxing only. Every check runs inside the caller's
//! budget; engine hiccups during a poll (a page mid-navigation, for
//! example) are retried until the budget runs out, matching the
//! blueprint rule that every wait is bounded (§8.4).

use std::future::Future;
use std::time::{Duration, Instant};

/// Upper bound for one polling budget. Budgets beyond this clamp down:
/// caller-supplied values (for example `wait_for`'s `timeout_ms`) can
/// exceed the process clock's representable range, where
/// `Instant + Duration` would panic instead of waiting longer.
pub(crate) const MAX_POLL_BUDGET: Duration = Duration::from_secs(600);

/// Polls `check` every `interval` until it resolves to `Some` or the
/// budget expires. Returns the check's value, or `None` on timeout.
pub async fn poll_until<T, F, Fut>(mut check: F, budget: Duration, interval: Duration) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let budget = budget.min(MAX_POLL_BUDGET);
    let deadline = Instant::now() + budget;
    loop {
        if let Some(value) = check().await {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(interval.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_the_first_present_value() {
        let mut attempts = 0;
        let value = poll_until(
            || {
                attempts += 1;
                async move { if attempts >= 3 { Some(attempts) } else { None } }
            },
            Duration::from_secs(2),
            Duration::from_millis(1),
        )
        .await;
        assert_eq!(value, Some(3));
    }

    #[tokio::test]
    async fn timeout_returns_none_and_stays_bounded() {
        let started = Instant::now();
        let value = poll_until(
            || async { Option::<u8>::None },
            Duration::from_millis(50),
            Duration::from_millis(10),
        )
        .await;
        assert_eq!(value, None);
        assert!(started.elapsed() < Duration::from_secs(2), "bounded wait");
    }

    #[tokio::test]
    async fn absurd_budgets_clamp_instead_of_panicking() {
        // u64::MAX milliseconds overflows the process clock when added
        // to `Instant::now()`; the clamp must prevent that panic. The
        // check succeeds immediately, so only deadline construction and
        // the happy path run here.
        let value = poll_until(
            || async {
                tokio::time::sleep(Duration::from_millis(5)).await;
                Some(1u8)
            },
            Duration::from_millis(u64::MAX),
            Duration::from_millis(10),
        )
        .await;
        assert_eq!(value, Some(1));
    }
}
