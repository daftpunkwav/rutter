//! The approval broker: parks actions until a human decides.
//!
//! Boundary: request bookkeeping and decision delivery only. Publishing
//! `ApprovalRequested` and mapping timeouts to the action error live in
//! the session layer; the broker never touches engines, events, or
//! files (blueprint §7.6: pure computation plus parked-future
//! bookkeeping).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::oneshot;

/// How a parked action was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// A human granted the action.
    Granted,
    /// A human denied the action.
    Denied,
    /// Nobody decided within the window.
    TimedOut,
}

/// A human's decision on one pending approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Grant the parked action.
    Grant,
    /// Deny the parked action.
    Deny,
}

/// Identifier of one pending approval, minted by the broker.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApprovalId(String);

impl ApprovalId {
    /// The identifier's text form (`apr-7`), shown to humans.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Parks actions and delivers human decisions to the waiters.
///
/// Flow: [`ApprovalBroker::open`] mints an id and hands the caller the
/// decision receiver; the caller publishes `ApprovalRequested` with the
/// id, then parks on [`ApprovalBroker::wait`]. A human answers through
/// [`ApprovalBroker::decide`], which removes the parked slot.
pub struct ApprovalBroker {
    counter: AtomicU64,
    pending: Mutex<HashMap<ApprovalId, oneshot::Sender<Decision>>>,
}

impl ApprovalBroker {
    /// Creates an empty broker.
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Opens an approval: mints its id, parks a decision sender, and
    /// hands back the receiver the caller must wait on.
    pub fn open(&self) -> (ApprovalId, oneshot::Receiver<Decision>) {
        let serial = self.counter.fetch_add(1, Ordering::Relaxed);
        let id = ApprovalId(format!("apr-{serial}"));
        let (sender, receiver) = oneshot::channel();
        self.lock_pending().insert(id.clone(), sender);
        (id, receiver)
    }

    /// Waits for a decision, giving up after `timeout`. A timeout (or a
    /// human answer) reclaims the parked slot, so a late
    /// [`ApprovalBroker::decide`] reports `false`.
    pub async fn wait(
        &self,
        id: &ApprovalId,
        receiver: oneshot::Receiver<Decision>,
        timeout: Duration,
    ) -> ApprovalOutcome {
        let outcome = match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(Decision::Grant)) => ApprovalOutcome::Granted,
            Ok(Ok(Decision::Deny)) => ApprovalOutcome::Denied,
            // The sender was dropped without a decision (manager shut
            // down); treat it like silence.
            Ok(Err(_)) | Err(_) => ApprovalOutcome::TimedOut,
        };
        self.lock_pending().remove(id);
        outcome
    }

    /// Delivers a human decision. Returns `false` when the approval is
    /// unknown (already decided, timed out, or never opened).
    pub fn decide(&self, id: &ApprovalId, decision: Decision) -> bool {
        match self.lock_pending().remove(id) {
            Some(sender) => sender.send(decision).is_ok(),
            None => false,
        }
    }

    /// Number of approvals currently parked.
    pub fn pending_count(&self) -> usize {
        self.lock_pending().len()
    }

    fn lock_pending(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<ApprovalId, oneshot::Sender<Decision>>> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for ApprovalBroker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn granted_when_a_human_answers_in_time() {
        let broker = ApprovalBroker::new();
        let (id, receiver) = broker.open();

        assert!(broker.decide(&id, Decision::Grant));
        assert_eq!(
            broker.wait(&id, receiver, Duration::from_secs(5)).await,
            ApprovalOutcome::Granted
        );
        assert_eq!(broker.pending_count(), 0);
    }

    #[tokio::test]
    async fn denied_maps_to_denied() {
        let broker = ApprovalBroker::new();
        let (id, receiver) = broker.open();
        assert!(broker.decide(&id, Decision::Deny));
        assert_eq!(
            broker.wait(&id, receiver, Duration::from_secs(1)).await,
            ApprovalOutcome::Denied
        );
    }

    #[tokio::test]
    async fn silence_times_out_and_reclaims_the_slot() {
        let broker = ApprovalBroker::new();
        let (id, receiver) = broker.open();
        assert_eq!(
            broker.wait(&id, receiver, Duration::from_millis(20)).await,
            ApprovalOutcome::TimedOut
        );
        assert_eq!(broker.pending_count(), 0);
        assert!(
            !broker.decide(&id, Decision::Grant),
            "a late decision must be rejected"
        );
    }

    #[tokio::test]
    async fn deciding_an_unknown_approval_fails() {
        let broker = ApprovalBroker::new();
        assert!(!broker.decide(&ApprovalId("apr-404".to_owned()), Decision::Grant));
    }
}
