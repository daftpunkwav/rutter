//! The rule set and verdict evaluation (docs/policy.md).

use serde::{Deserialize, Serialize};

use crate::brief::{ApprovalBrief, ApprovalEffect, VerdictBasis};
use crate::canonical::canonical_url;
use crate::class::ActionClass;
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

/// What the rule set says about one operation, and what a human needs in
/// the case where a human has to be asked.
///
/// The brief rides on the verdict instead of sitting next to it: the only
/// moment it is worth building is the one where someone is about to be
/// asked, and a caller cannot forget to attach it.
#[derive(Debug, Clone, PartialEq)]
pub enum Review {
    /// Run it; no supervision overhead.
    Allowed,
    /// Refuse it; nobody is asked.
    Denied,
    /// Park it until a human answers, carrying the material they answer
    /// from.
    NeedsApproval(Box<ApprovalBrief>),
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
/// and the window a human has to answer approvals (docs/policy.md:
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
        self.judge(class, url).0
    }

    /// Reviews one operation for supervision. This is the entry point a
    /// caller reaches a verdict through: it canonicalizes the judgment URL
    /// itself, so no caller can reach a verdict while skipping
    /// canonicalization, and it hands back the brief a human decides from
    /// whenever the answer is to ask one (docs/policy.md).
    ///
    /// A URL that will not canonicalize — not a URL at all, or one hiding
    /// credentials behind a `user@host` decoy — counts as no URL at all,
    /// which fails closed rather than matching a placeholder.
    pub fn review(&self, effect: ApprovalEffect, raw_url: Option<&str>) -> Review {
        let judged_url = raw_url.and_then(canonical_url);
        let class = effect.class();
        let (verdict, basis) = match &judged_url {
            Some(url) => self.judge(class, url),
            None => match self.judge(class, "") {
                (Verdict::Allow, _) => (Verdict::RequireApproval, VerdictBasis::MissingUrl),
                (verdict, basis) => (verdict, basis),
            },
        };
        match verdict {
            Verdict::Allow => Review::Allowed,
            Verdict::Deny => Review::Denied,
            Verdict::RequireApproval => Review::NeedsApproval(Box::new(ApprovalBrief {
                class,
                judged_url,
                basis,
                effect,
            })),
        }
    }

    /// The verdict plus which rule reached it. `evaluate` is the
    /// projection that drops the explanation.
    fn judge(&self, class: ActionClass, url: &str) -> (Verdict, VerdictBasis) {
        for (position, rule) in self.rules.iter().enumerate() {
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
                return (
                    rule.verdict,
                    VerdictBasis::Rule {
                        index: position + 1,
                        url_pattern: rule
                            .url_pattern
                            .as_ref()
                            .map(|pattern| pattern.as_str().to_owned()),
                    },
                );
            }
        }
        (self.default_verdict, VerdictBasis::SetDefault)
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
    fn review_classifies_the_effect_it_was_given() {
        let rules = RuleSet::default_set();
        let pointer = rules.review(
            ApprovalEffect::Action {
                action: Action::Click {
                    reference: Reference::new("e1"),
                },
            },
            Some("https://x.example/"),
        );
        assert_eq!(pointer, Review::Allowed, "a pointer is not gated here");

        let parked = rules.review(
            ApprovalEffect::Cookies { count: 2 },
            Some("https://x.example/"),
        );
        match parked {
            Review::NeedsApproval(brief) => {
                assert_eq!(brief.class, ActionClass::Cookies);
                assert_eq!(brief.effect, ApprovalEffect::Cookies { count: 2 });
            }
            other => panic!("cookies must be parked, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_url_upgrades_allow_to_approval() {
        let rules = RuleSet::new(vec![], Verdict::Allow);
        let review = rules.review(
            ApprovalEffect::Action {
                action: Action::Back,
            },
            Some("not a url at all"),
        );
        match review {
            Review::NeedsApproval(brief) => {
                assert_eq!(brief.basis, VerdictBasis::MissingUrl);
                assert_eq!(
                    brief.judged_url, None,
                    "an unparseable URL is judged as no URL, never as a placeholder"
                );
            }
            other => panic!("a bare allow fails closed, got {other:?}"),
        }
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
        let cookies = rules.review(ApprovalEffect::Cookies { count: 1 }, None);
        assert_eq!(
            cookies,
            Review::Denied,
            "a class-only deny does not need URL information"
        );
        assert!(matches!(
            rules.review(
                ApprovalEffect::Action {
                    action: Action::Back
                },
                None
            ),
            Review::NeedsApproval(_)
        ));
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
            rules.review(
                ApprovalEffect::Action {
                    action: Action::Back
                },
                None
            ),
            Review::Denied,
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
        assert!(
            matches!(
                rules.review(
                    ApprovalEffect::Action {
                        action: Action::Back
                    },
                    None
                ),
                Review::NeedsApproval(_)
            ),
            "the old about:blank degradation let a deny rule 'match' a placeholder; without a URL the verdict must be the fail-closed upgrade instead"
        );
    }

    #[test]
    fn the_brief_names_the_rule_and_the_canonical_url() {
        let rules = RuleSet::new(
            vec![
                PolicyRule {
                    action_class: Some("scroll".to_owned()),
                    url_pattern: None,
                    verdict: Verdict::Deny,
                },
                PolicyRule {
                    action_class: Some("navigation".to_owned()),
                    url_pattern: Some(Pattern::new("https://*.bank.example/*")),
                    verdict: Verdict::RequireApproval,
                },
            ],
            Verdict::Allow,
        );
        let review = rules.review(
            ApprovalEffect::Action {
                action: Action::Navigate {
                    url: "https://PAY.bank.example/confirm".to_owned(),
                },
            },
            // The raw target, as an agent supplied it: the brief must
            // carry what was judged, not what was typed.
            Some("https://PAY.bank.example/confirm"),
        );
        match review {
            Review::NeedsApproval(brief) => {
                assert_eq!(
                    brief.basis,
                    VerdictBasis::Rule {
                        index: 2,
                        url_pattern: Some("https://*.bank.example/*".to_owned()),
                    },
                    "the human sees which rule stopped the action"
                );
                assert_eq!(
                    brief.judged_url.as_deref(),
                    Some("https://pay.bank.example/confirm"),
                    "the judged URL is the canonical form"
                );
            }
            other => panic!("the second rule parks the navigation, got {other:?}"),
        }
    }

    #[test]
    fn a_cookie_write_is_never_borrowed_from_another_action() {
        // The regression this guards: cookie approvals rode on
        // `Action::Reload`, so a human approving "reload" authorized a
        // cookie write. The effect must say what it is.
        let rules = RuleSet::default_set();
        let Review::NeedsApproval(brief) = rules.review(
            ApprovalEffect::Cookies { count: 3 },
            Some("https://x.example/"),
        ) else {
            panic!("cookies require a human");
        };
        assert_eq!(brief.effect, ApprovalEffect::Cookies { count: 3 });
        assert!(!matches!(brief.effect, ApprovalEffect::Action { .. }));
    }
}
