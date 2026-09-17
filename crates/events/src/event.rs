//! The event vocabulary: structured facts published on the backbone.
//!
//! Boundary: events are data, derived from what the orchestration layer
//! does. Policy and approval events arrive with M2; screencast frame
//! events arrive with the dashboard and follow the latest-wins
//! backpressure rule (blueprint §7.5).

use serde::{Deserialize, Serialize};

use rutter_core::action::{Action, Origin};
use rutter_core::error::ActionError;
use rutter_core::ids::PageId;

/// A structured fact published on the event backbone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A session workspace came into being.
    SessionStarted,
    /// A session workspace was closed by its client.
    SessionClosed,
    /// The engine process was launched for the first time.
    EngineStarted {
        /// Backend identity, for example `chrome-headless-shell`.
        backend: String,
        /// Backend version string.
        version: String,
    },
    /// The supervisor replaced a dead engine; state before the restart
    /// is gone and time has passed (blueprint §7.4).
    EngineRestarted,
    /// A page opened inside the session's context.
    PageOpened {
        /// The page that opened.
        page: PageId,
    },
    /// A page closed inside the session's context.
    PageClosed {
        /// The page that closed.
        page: PageId,
    },
    /// The active page navigated to a URL.
    PageNavigated {
        /// The page that navigated.
        page: PageId,
        /// Effective URL after redirects.
        url: String,
    },
    /// An action was requested; recorded before execution starts.
    ActionRequested {
        /// Page the action targets.
        page: PageId,
        /// Who initiated the action.
        origin: Origin,
        /// The action itself.
        action: Action,
    },
    /// An action finished successfully.
    ActionCompleted {
        /// Page the action targeted.
        page: PageId,
        /// Who initiated the action.
        origin: Origin,
        /// The action itself.
        action: Action,
    },
    /// An action failed; `error` carries the agent-readable taxonomy.
    ActionFailed {
        /// Page the action targeted.
        page: PageId,
        /// Who initiated the action.
        origin: Origin,
        /// The action itself.
        action: Action,
        /// Why the action failed.
        error: ActionError,
    },
    /// Policy parked an action until a human decides.
    ApprovalRequested {
        /// Broker-minted identifier humans answer with.
        request_id: String,
        /// Page the parked action targets.
        page: PageId,
        /// The parked action.
        action: Action,
    },
    /// A human answered (or the window timed out).
    ApprovalResolved {
        /// Identifier of the resolved request.
        request_id: String,
        /// Whether the action may proceed.
        granted: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use rutter_core::action::Action;

    #[test]
    fn events_round_trip_through_json() {
        let event = Event::ActionFailed {
            page: PageId::new("ctx-1:page-0"),
            origin: Origin::Agent,
            action: Action::Click {
                reference: rutter_core::reference::Reference::new("e17"),
            },
            error: ActionError::ReferenceExpired {
                reference: rutter_core::reference::Reference::new("e17"),
            },
        };

        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, event);
        assert!(json.contains("action_failed"), "tagged enum: {json}");
    }
}
