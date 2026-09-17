//! The TOML policy configuration format.
//!
//! Boundary: parsing only. File reading is the caller's job (the CLI);
//! parse errors carry the TOML position so operators can fix configs.
//! Semantics: rules match in document order, the first match wins, and
//! `default` covers everything no rule matched.

use serde::Deserialize;

use crate::class::ActionClass;
use crate::pattern::Pattern;
use crate::rules::{PolicyRule, RuleSet, Verdict};

/// The parsed policy configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PolicyConfig {
    /// Verdict for actions no rule matched; `allow` when omitted.
    pub default: Option<String>,
    /// Approval window in milliseconds; 120 000 when omitted.
    pub approval_timeout_ms: Option<u64>,
    /// Ordered rules; the first match wins.
    #[serde(default)]
    pub rules: Vec<ConfigRule>,
}

/// One `[[rules]]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ConfigRule {
    /// Class the rule applies to; see [`ActionClass::parse`].
    pub action_class: Option<String>,
    /// URL pattern with `*` wildcards.
    pub url_pattern: Option<String>,
    /// `allow`, `deny`, or `require_approval`.
    pub verdict: String,
}

/// Parses a policy configuration into a rule set.
pub fn parse_policy(toml_text: &str) -> Result<RuleSet, ConfigError> {
    let config: PolicyConfig = toml::from_str(toml_text).map_err(|error| ConfigError {
        detail: error.to_string(),
    })?;

    let default_verdict = match config.default.as_deref() {
        None => Verdict::Allow,
        Some(name) => Verdict::parse(name).ok_or_else(|| unknown_verdict("default", name))?,
    };

    let mut rules = Vec::with_capacity(config.rules.len());
    for rule in &config.rules {
        let verdict = Verdict::parse(&rule.verdict)
            .ok_or_else(|| unknown_verdict("a rule", &rule.verdict))?;
        if let Some(name) = &rule.action_class {
            if ActionClass::parse(name).is_none() {
                return Err(ConfigError {
                    detail: format!(
                        "unknown action_class '{name}'; use navigation, pointer,                          keyboard, selection, scroll, or cookies"
                    ),
                });
            }
        }
        if rule.action_class.is_none() && rule.url_pattern.is_none() {
            return Err(ConfigError {
                detail: "a rule must set action_class, url_pattern, or both".to_owned(),
            });
        }
        rules.push(PolicyRule {
            action_class: rule.action_class.clone(),
            url_pattern: rule.url_pattern.as_deref().and_then(Pattern::parse),
            verdict,
        });
    }

    let ruleset = RuleSet::new(rules, default_verdict);
    Ok(match config.approval_timeout_ms {
        Some(ms) => ruleset.with_approval_timeout(std::time::Duration::from_millis(ms)),
        None => ruleset,
    })
}

/// Why a policy configuration was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid policy configuration: {detail}")]
pub struct ConfigError {
    /// What was wrong with the configuration.
    pub detail: String,
}

fn unknown_verdict(where_: &str, name: &str) -> ConfigError {
    ConfigError {
        detail: format!(
            "unknown verdict '{name}' in {where_}; use allow, deny, or require_approval"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::class::ActionClass;

    #[test]
    fn parses_rules_and_default() {
        let text = r#"
default = "require_approval"

[[rules]]
action_class = "navigation"
verdict = "allow"

[[rules]]
url_pattern = "https://bank.example/*"
verdict = "deny"
"#;
        let rules = parse_policy(text).expect("parses");
        assert_eq!(
            rules.evaluate(ActionClass::Navigation, "https://any.example/"),
            Verdict::Allow
        );
        assert_eq!(
            rules.evaluate(ActionClass::Pointer, "https://bank.example/pay"),
            Verdict::Deny,
            "the pattern rule matches every class"
        );
        assert_eq!(
            rules.evaluate(ActionClass::Pointer, "https://other.example/"),
            Verdict::RequireApproval
        );
    }

    #[test]
    fn empty_config_is_the_permissive_default() {
        let rules = parse_policy("").expect("empty config");
        assert_eq!(
            rules.evaluate(ActionClass::Cookies, "https://x.example/"),
            Verdict::Allow,
            "an explicit empty config overrides the conservative built-in"
        );
    }

    #[test]
    fn unknown_verdict_names_are_rejected() {
        let error = parse_policy("default = \"yolo\"").expect_err("bad default");
        assert!(error.to_string().contains("yolo"));
        let error = parse_policy("[[rules]]\naction_class = \"cookies\"\nverdict = \"maybe\"\n")
            .expect_err("bad rule verdict");
        assert!(error.to_string().contains("maybe"));
    }

    #[test]
    fn unknown_action_classes_are_rejected() {
        let error = parse_policy("[[rules]]\naction_class = \"teleport\"\nverdict = \"allow\"\n")
            .expect_err("unknown class");
        assert!(error.to_string().contains("teleport") || error.to_string().contains("invalid"));
    }

    #[test]
    fn empty_rules_are_rejected() {
        let error =
            parse_policy("[[rules]]\nverdict = \"allow\"\n").expect_err("rule without axis");
        assert!(error.to_string().contains("action_class"));
    }
}
