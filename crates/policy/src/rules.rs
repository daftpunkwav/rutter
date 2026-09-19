//! The rule set and verdict evaluation (blueprint §7.6).

use serde::{Deserialize, Serialize};

use crate::class::{ActionClass, class_of};
use crate::pattern::Pattern;

/// The decision for an action against a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Run without supervision overhead.
    Allow,
    /// Refuse outright; the caller sees `ActionError::ApprovalDenied`.
    Deny,
    /// Park the action until a human grants or denies it, or the
    /// approval window times out.
    RequireApproval,
}

impl Verdict {
    /// Parses a verdict from its snake_case TOML name.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            "require_approval" => Some(Self::RequireApproval),
            _ => None,
        }
    }
}

/// One rule: an optional action class and an optional URL pattern, both
/// optional so a rule can target every action of one class or every URL
/// of one pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Class the rule applies to; `None` means every class.
    pub action_class: Option<String>,
    /// URL pattern the rule applies to; `None` means every URL.
    pub url_pattern: Option<Pattern>,
    /// Verdict when the rule matches.
    pub verdict: Verdict,
}

/// An ordered rule set plus the verdict for actions no rule matches,
/// and the window a human has to answer approvals (blueprint §7.6:
/// configurable, default 120 s).
///
/// The default set is conservative for the sensitive class: cookie
/// manipulation always requires approval, everything else is allowed
/// unless a loaded configuration says otherwise.
#[derive(Debug, Clone)]
pub struct RuleSet {
    rules: Vec<PolicyRule>,
    default_verdict: Verdict,
    approval_timeout: std::time::Duration,
}

impl RuleSet {
    /// The built-in default: allow, except cookies require approval,
    /// with the 120 s approval window.
    pub fn default_set() -> Self {
        Self {
            rules: vec![PolicyRule {
                action_class: Some(ActionClass::Cookies.name().to_owned()),
                url_pattern: None,
                verdict: Verdict::RequireApproval,
            }],
            default_verdict: Verdict::Allow,
            approval_timeout: std::time::Duration::from_secs(120),
        }
    }

    /// Builds a rule set from parsed rules and a default verdict.
    pub fn new(rules: Vec<PolicyRule>, default_verdict: Verdict) -> Self {
        Self {
            rules,
            default_verdict,
            approval_timeout: std::time::Duration::from_secs(120),
        }
    }

    /// Overrides the approval window.
    pub fn with_approval_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.approval_timeout = timeout;
        self
    }

    /// The window a human has to answer an approval.
    pub fn approval_timeout(&self) -> std::time::Duration {
        self.approval_timeout
    }

    /// Evaluates one action class against a URL: the first matching rule
    /// wins (rules keep document order), then the default.
    pub fn evaluate(&self, class: ActionClass, url: &str) -> Verdict {
        for rule in &self.rules {
            let class_matches = rule
                .action_class
                .as_deref()
                .map(|name| ActionClass::parse(name) == Some(class))
                .unwrap_or(true);
            let url_matches = rule
                .url_pattern
                .as_ref()
                .map(|pattern| pattern.matches(url))
                .unwrap_or(true);
            if class_matches && url_matches {
                return rule.verdict;
            }
        }
        self.default_verdict
    }

    /// Convenience for actions: classifies then evaluates.
    pub fn evaluate_action(&self, action: &rutter_core::action::Action, url: &str) -> Verdict {
        self.evaluate(class_of(action), url)
    }

    /// Evaluates one action class when no usable URL is known: the page
    /// URL is unreadable, a navigation target is not a URL at all, or
    /// the target embeds credentials. URL-scoped rules cannot match,
    /// class-only rules still do, and a bare `Allow` upgrades to
    /// `RequireApproval`, so missing URL information never passes an
    /// action unsupervised (blueprint §7.6).
    pub fn evaluate_without_url(&self, class: ActionClass) -> Verdict {
        match self.evaluate(class, "") {
            Verdict::Allow => Verdict::RequireApproval,
            verdict => verdict,
        }
    }
}

impl Default for RuleSet {
    fn default() -> Self {
        Self::default_set()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rutter_core::action::Action;
    use rutter_core::reference::Reference;

    #[test]
    fn default_set_requires_approval_for_cookies_only() {
        let rules = RuleSet::default_set();
        assert_eq!(
            rules.evaluate(ActionClass::Cookies, "https://any.example/"),
            Verdict::RequireApproval
        );
        assert_eq!(
            rules.evaluate(ActionClass::Pointer, "https://any.example/"),
            Verdict::Allow
        );
    }

    #[test]
    fn first_matching_rule_wins_in_order() {
        let rules = RuleSet::new(
            vec![
                PolicyRule {
                    action_class: Some("navigation".to_owned()),
                    url_pattern: Some(Pattern::new("https://bank.example/*")),
                    verdict: Verdict::Deny,
                },
                PolicyRule {
                    action_class: None,
                    url_pattern: Some(Pattern::new("https://*.example.com/*")),
                    verdict: Verdict::RequireApproval,
                },
            ],
            Verdict::Allow,
        );
        assert_eq!(
            rules.evaluate(ActionClass::Navigation, "https://bank.example/pay"),
            Verdict::Deny,
            "the more specific earlier rule wins"
        );
        assert_eq!(
            rules.evaluate(ActionClass::Pointer, "https://api.example.com/x"),
            Verdict::RequireApproval
        );
        assert_eq!(
            rules.evaluate(ActionClass::Pointer, "https://other.org/x"),
            Verdict::Allow
        );
    }

    #[test]
    fn evaluate_action_classifies_before_matching() {
        let rules = RuleSet::default_set();
        let action = Action::Click {
            reference: Reference::new("e1"),
        };
        assert_eq!(
            rules.evaluate_action(&action, "https://x.example/"),
            Verdict::Allow
        );
    }

    #[test]
    fn a_missing_url_upgrades_allow_to_approval() {
        let rules = RuleSet::new(vec![], Verdict::Allow);
        assert_eq!(
            rules.evaluate_without_url(ActionClass::Pointer),
            Verdict::RequireApproval
        );
    }

    #[test]
    fn class_rules_still_bind_without_a_url() {
        let rules = RuleSet::new(
            vec![PolicyRule {
                action_class: Some("cookies".to_owned()),
                url_pattern: None,
                verdict: Verdict::Deny,
            }],
            Verdict::Allow,
        );
        assert_eq!(
            rules.evaluate_without_url(ActionClass::Cookies),
            Verdict::Deny,
            "a class-only deny does not need URL information"
        );
        assert_eq!(
            rules.evaluate_without_url(ActionClass::Navigation),
            Verdict::RequireApproval
        );
    }

    #[test]
    fn catch_all_url_rules_still_bind_without_a_url() {
        let rules = RuleSet::new(
            vec![PolicyRule {
                action_class: None,
                url_pattern: Some(Pattern::new("*")),
                verdict: Verdict::Deny,
            }],
            Verdict::Allow,
        );
        assert_eq!(
            rules.evaluate_without_url(ActionClass::Pointer),
            Verdict::Deny,
            "a 'match everything' pattern matches the empty judgment URL"
        );
    }

    #[test]
    fn host_scoped_rules_do_not_match_a_missing_url() {
        let rules = RuleSet::new(
            vec![PolicyRule {
                action_class: None,
                url_pattern: Some(Pattern::new("https://bank.example/*")),
                verdict: Verdict::Deny,
            }],
            Verdict::Allow,
        );
        assert_eq!(
            rules.evaluate_without_url(ActionClass::Navigation),
            Verdict::RequireApproval,
            "the old about:blank degradation let a deny rule 'match' a placeholder; without a URL the verdict must be the fail-closed upgrade instead"
        );
    }
}
