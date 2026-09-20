//! Policy: rule sets, verdict evaluation, and the approval broker.
//!
//! Responsibilities:
//! - Reach a verdict for one operation as a pure function: classify it,
//!   canonicalize the judgment URL, match the rule set, and answer with
//!   [`rules::Review`] (docs/policy.md).
//! - Build the [`brief::ApprovalBrief`] a human decides from whenever the
//!   answer is to ask one — the class, the URL judged, the rule that
//!   spoke, and the effect a grant authorizes.
//! - Parse the TOML policy configuration (action class x URL pattern ->
//!   verdict).
//! - Park actions that require approval and hand decisions back to the
//!   waiting executor through the [`broker::ApprovalBroker`].
//!
//! Boundary: pure computation plus parked-future bookkeeping. Rule and
//! verdict data have no I/O; dangerousness is decided by rutter's rules,
//! never by the agent's self-declaration — that is the point of
//! supervision (docs/policy.md). File reading and event publishing live
//! in the callers.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod brief;
pub mod broker;
pub mod canonical;
pub mod class;
pub mod config;
pub mod pattern;
pub mod rules;

pub use brief::{ApprovalBrief, ApprovalEffect, VerdictBasis};
pub use broker::{ApprovalBroker, ApprovalId, ApprovalOutcome, Decision};
pub use canonical::canonical_url;
pub use class::ActionClass;
pub use config::parse_policy;
pub use pattern::Pattern;
pub use rules::{Review, RuleSet, Verdict};
