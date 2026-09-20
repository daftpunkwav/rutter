//! The brief a human decides from (docs/policy.md).
//!
//! Responsibility: describe one parked operation in the same terms the
//! verdict was reached in — the class it fell under, the canonical URL it
//! was judged at, which part of the rule set spoke, and what a grant
//! actually authorizes.
//!
//! Boundary: data only. Reaching a verdict belongs to `rules`; publishing
//! this brief belongs to the orchestration layer, and rendering it belongs
//! to the dashboard.
//!
//! Why this type exists at all: the approval surface once carried a plain
//! `Action`, which left every operation without an action variant to
//! impersonate one. A cookie write went out as `reload`, so the human
//! pressed "allow reload" and cookies were written. An approval must
//! describe its own effect.

use serde::{Deserialize, Serialize};

use crate::class::{ActionClass, class_of};
use rutter_core::action::Action;

/// What approving authorizes.
///
/// `PartialEq` only: the wrapped action carries floating-point fields, so
/// a total equality would not exist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ApprovalEffect {
    /// One typed action from the shared vocabulary. A struct variant, not
    /// a newtype one: an internally tagged enum cannot add its tag inside
    /// another tagged enum.
    Action {
        /// The action a grant sets free.
        action: Action,
    },
    /// A cookie write on the session's context. The action vocabulary has
    /// no cookie variant, and borrowing an unrelated one would put a
    /// different promise in front of the human than the one they keep.
    Cookies {
        /// How many cookies the write carries.
        count: usize,
    },
}

impl ApprovalEffect {
    /// The class this effect falls under. Derived here so that no caller
    /// classifies an operation by hand and disagrees with the rule set.
    pub fn class(&self) -> ActionClass {
        match self {
            Self::Action { action } => class_of(action),
            Self::Cookies { .. } => ActionClass::Cookies,
        }
    }
}

/// Why the verdict is what it is, as far as the deciding human needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum VerdictBasis {
    /// The nth rule of the loaded set matched. Numbering is one-based and
    /// follows the document, so it can be quoted back to the configuration.
    Rule {
        /// Position of the rule in the set.
        index: usize,
        /// The rule's URL pattern, when it carried one.
        url_pattern: Option<String>,
    },
    /// No rule matched, so the set's default verdict applied.
    SetDefault,
    /// The set would have allowed the operation on an empty URL; the
    /// verdict was upgraded because no usable URL existed. The fail-closed
    /// path (docs/policy.md).
    MissingUrl,
}

/// Everything a human needs to answer one approval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalBrief {
    /// The class the verdict was reached under.
    pub class: ActionClass,
    /// The canonical URL the verdict was reached at. `None` marks the
    /// fail-closed path, where no usable URL existed.
    pub judged_url: Option<String>,
    /// Which part of the rule set spoke.
    pub basis: VerdictBasis,
    /// The operation a grant sets free.
    pub effect: ApprovalEffect,
}
