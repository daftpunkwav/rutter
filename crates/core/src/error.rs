//! The error surface agents must be able to reason about.
//!
//! Boundary: these variants are the complete vocabulary for action
//! failures; mapping engine and transport failures into them is the
//! responsibility of the layer that observes the failure. Errors are
//! data, not logs: every variant serializes and carries an actionable,
//! English, agent-readable hint.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ids::SessionId;
use crate::reference::Reference;

/// Why a requested action failed.
#[derive(Debug, Clone, PartialEq, Error, Serialize, Deserialize)]
pub enum ActionError {
    /// The page could not be loaded over the network.
    #[error("navigation to '{url}' failed ({cause})")]
    NavigationFailed {
        /// URL that was requested.
        url: String,
        /// Transport-level classification of the failure.
        cause: TransportCause,
    },

    /// The element the reference points to no longer exists.
    #[error("reference '{reference}' expired")]
    ReferenceExpired {
        /// Reference that could not be resolved.
        reference: Reference,
    },

    /// The element exists but cannot receive the action.
    #[error("element '{reference}' is not interactable: {reason}")]
    NotInteractable {
        /// Reference to the element.
        reference: Reference,
        /// Why the element refused the action.
        reason: String,
    },

    /// An auto-wait phase did not complete in time.
    #[error("timed out during the '{phase}' phase after {elapsed:?}")]
    TimedOut {
        /// Phase of the auto-wait that did not complete.
        phase: WaitPhase,
        /// Time spent waiting before giving up.
        elapsed: Duration,
    },

    /// A human approver rejected the action.
    #[error("approval denied for action on '{reference}'")]
    ApprovalDenied {
        /// Reference to the element the action targeted.
        reference: Reference,
    },

    /// No approval decision arrived within the configured window.
    #[error("no approval decision arrived after {waited:?}")]
    ApprovalTimedOut {
        /// Time spent waiting for a decision.
        waited: Duration,
    },

    /// The engine died and the session lost its pages.
    #[error("engine terminated for session '{session}'")]
    EngineTerminated {
        /// Session whose engine died.
        session: SessionId,
    },

    /// A bug was contained at the action boundary; never a silent pass.
    #[error("internal error: {detail}")]
    Internal {
        /// What went wrong, for reporting the bug.
        detail: String,
    },
}

impl ActionError {
    /// Returns an actionable, English hint an agent can follow to decide
    /// what to do next.
    pub fn hint(&self) -> String {
        match self {
            Self::NavigationFailed { url, cause } => format!(
                "re-check that '{url}' is reachable and correctly spelled \
                 (transport cause: {cause}), then retry"
            ),
            Self::ReferenceExpired { .. } => {
                "take a fresh snapshot and retry with the new references".to_owned()
            }
            Self::NotInteractable { reason, .. } => format!(
                "re-snapshot and pick an element that is visible, stable, \
                 and enabled; the element reported: {reason}"
            ),
            Self::TimedOut { phase, elapsed } => format!(
                "the action gave up during the '{phase}' phase after \
                 {elapsed:?}; wait for readiness or raise the timeout"
            ),
            Self::ApprovalDenied { .. } => {
                "a human rejected this action; do not retry it unchanged".to_owned()
            }
            Self::ApprovalTimedOut { waited } => format!(
                "no human answered within {waited:?}; only request approval \
                 again if the operation is still needed"
            ),
            Self::EngineTerminated { .. } => {
                "the engine died and rutter is recovering it; wait for the \
                 EngineRestarted event, then take a fresh snapshot"
                    .to_owned()
            }
            Self::Internal { detail } => {
                format!("an internal bug was contained; report it, citing: {detail}")
            }
        }
    }
}

/// Transport-level classification of a navigation failure.
#[derive(Debug, Clone, PartialEq, Error, Serialize, Deserialize)]
pub enum TransportCause {
    /// The operation exceeded its deadline.
    #[error("timed out")]
    TimedOut,
    /// The connection could not be established or broke mid-transfer.
    #[error("connection failed")]
    ConnectionFailed,
    /// The host name could not be resolved.
    #[error("DNS resolution failed")]
    DnsFailed,
    /// The TLS handshake failed.
    #[error("TLS failed")]
    TlsFailed,
    /// The load was aborted, for example by a download or a redirect loop.
    #[error("aborted")]
    Aborted,
    /// The server answered with an error status.
    #[error("HTTP status {status}")]
    Http {
        /// HTTP status code returned by the server.
        status: u16,
    },
}

/// Phases of the three-phase auto-wait every mutating action runs through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum WaitPhase {
    /// Waiting for the element to become visible.
    #[error("visible")]
    Visible,
    /// Waiting for the element to stop moving or changing.
    #[error("stable")]
    Stable,
    /// Waiting for the element to become enabled.
    #[error("enabled")]
    Enabled,
    /// Executing the action itself.
    #[error("act")]
    Act,
    /// Waiting for the page to settle after the action.
    #[error("settle")]
    Settle,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_carries_a_hint() {
        let reference = Reference::new("e1");
        let session = SessionId::new("s1");
        let errors = [
            ActionError::NavigationFailed {
                url: "https://example.com".to_owned(),
                cause: TransportCause::DnsFailed,
            },
            ActionError::ReferenceExpired {
                reference: reference.clone(),
            },
            ActionError::NotInteractable {
                reference: reference.clone(),
                reason: "covered by another element".to_owned(),
            },
            ActionError::TimedOut {
                phase: WaitPhase::Stable,
                elapsed: Duration::from_secs(5),
            },
            ActionError::ApprovalDenied {
                reference: reference.clone(),
            },
            ActionError::ApprovalTimedOut {
                waited: Duration::from_secs(120),
            },
            ActionError::EngineTerminated { session },
            ActionError::Internal {
                detail: "unreachable state".to_owned(),
            },
        ];
        for error in &errors {
            assert!(!error.hint().is_empty(), "missing hint for {error}");
        }
    }

    #[test]
    fn navigation_error_mentions_url_and_cause() {
        let error = ActionError::NavigationFailed {
            url: "https://example.com".to_owned(),
            cause: TransportCause::Http { status: 500 },
        };
        let message = error.to_string();
        assert!(message.contains("https://example.com"));
        assert!(message.contains("500"));
    }
}
