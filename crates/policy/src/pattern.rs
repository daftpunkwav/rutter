//! URL patterns for policy rules: `*` wildcards, nothing else.

use serde::{Deserialize, Serialize};

/// Why a pattern string was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatternError {
    /// The pattern is empty or whitespace-only.
    #[error("patterns must not be blank")]
    Blank,
    /// A `*` sits inside the URL authority with no later `/` to anchor
    /// the authority boundary, so it would match across host-label dots
    /// into sibling domains (`https://bank.example*` also matches
    /// `bank.example.evil.com`, which anyone can register).
    #[error(
        "a wildcard in the authority is not anchored by a later '/'; \
         write 'https://host/*' so it cannot cross into sibling domains"
    )]
    UnanchoredAuthorityWildcard,
}

/// A URL pattern with `*` wildcards; `https://*.example.com/*` matches
/// scheme, host, and any path. Anything a wildcard must consume is
/// matched non-greedily by segments of literal text between wildcards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pattern(String);

impl Pattern {
    /// Wraps a pattern string without validation; use [`Pattern::parse`]
    /// to reject blank and unanchored patterns.
    pub fn new(pattern: impl Into<String>) -> Self {
        Self(pattern.into())
    }

    /// Parses a pattern, rejecting blank ones and authority wildcards
    /// that no later `/` anchors.
    pub fn parse(pattern: &str) -> Result<Self, PatternError> {
        if pattern.trim().is_empty() {
            return Err(PatternError::Blank);
        }
        if unanchored_authority_wildcard(pattern) {
            return Err(PatternError::UnanchoredAuthorityWildcard);
        }
        Ok(Self(pattern.to_owned()))
    }

    /// The pattern source text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `url` matches this pattern. Matching is case-sensitive
    /// except for the scheme and host, which URLs lowercase anyway.
    ///
    /// Precondition: `url` is the canonical form to judge — the output
    /// of [`crate::canonical::canonical_url`] (lowercased host, default
    /// port gone, trailing root dot normalized). A raw agent-supplied
    /// string passed here verbatim can carry spellings a pattern was
    /// never written for (`EVIL.example:443`, a `user@host` decoy);
    /// the review path canonicalizes every judgment URL itself, and a
    /// direct `evaluate` caller must hand over the same shape.
    pub fn matches(&self, url: &str) -> bool {
        wildcard_match(self.0.as_bytes(), url.as_bytes())
    }
}

/// The README promise ("blank patterns are rejected, not widened") is a
/// deserialization contract, not just a `parse` one: the derived impl
/// would bypass [`Pattern::parse`] and admit a blank pattern as a dead
/// rule that silently matches nothing.
impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        Pattern::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// Whether a `*` sits in the URL authority with no later literal `/`
/// anywhere after it. This closes the cheapest wildcard escape — a
/// suffix rule matching sibling domains with no crafting
/// (`https://bank.example*` also matches `bank.example.evil.com`). A
/// spelling check, not a host guarantee: `*` consumes `/` too, so an
/// anchored pattern can still be steered by a URL that carries its
/// literals in the path or query. Patterns without a scheme name no
/// host and are never checked.
fn unanchored_authority_wildcard(pattern: &str) -> bool {
    let Some(scheme_end) = pattern.find("://") else {
        return false;
    };
    let authority_start = scheme_end + 3;
    let authority = match pattern[authority_start..].find('/') {
        Some(offset) => &pattern[authority_start..authority_start + offset],
        None => &pattern[authority_start..],
    };
    match authority.find('*') {
        None => false,
        Some(star_at) => !pattern[authority_start + star_at..].contains('/'),
    }
}

/// Iterative wildcard match over bytes: `*` consumes any run (including
/// empty), literals must match exactly.
fn wildcard_match(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut star_t) = (None::<usize>, 0usize);
    while t < text.len() {
        if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            star_t = t;
            p += 1;
        } else if p < pattern.len() && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if let Some(star_pos) = star {
            p = star_pos + 1;
            star_t += 1;
            t = star_t;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_match_prefix_host_and_path() {
        let pattern = Pattern::new("https://*.example.com/*");
        assert!(pattern.matches("https://api.example.com/v1/users"));
        assert!(pattern.matches("https://www.example.com/"));
        assert!(!pattern.matches("https://example.com/"));
        assert!(!pattern.matches("http://api.example.com/"));
    }

    #[test]
    fn exact_patterns_require_full_match() {
        let pattern = Pattern::new("https://example.com/login");
        assert!(pattern.matches("https://example.com/login"));
        assert!(!pattern.matches("https://example.com/login/extra"));
    }

    #[test]
    fn star_alone_matches_everything() {
        assert!(Pattern::new("*").matches("https://anything.example/"));
        assert!(Pattern::new("*").matches(""));
    }

    #[test]
    fn parse_rejects_blank_patterns() {
        assert_eq!(Pattern::parse(""), Err(PatternError::Blank));
        assert_eq!(Pattern::parse("  "), Err(PatternError::Blank));
        assert!(Pattern::parse("https://x/*").is_ok());
    }

    #[test]
    fn authority_wildcards_must_be_anchored_by_a_later_slash() {
        // `*` consumes any bytes, host-label dots included: these
        // spellings match sibling domains an attacker can register
        // without any crafting, so `parse` refuses them instead of
        // trusting the rule author to know.
        assert!(Pattern::new("https://bank.example*").matches("https://bank.example.evil.com/"));
        assert_eq!(
            Pattern::parse("https://bank.example*"),
            Err(PatternError::UnanchoredAuthorityWildcard)
        );
        assert_eq!(
            Pattern::parse("https://*"),
            Err(PatternError::UnanchoredAuthorityWildcard)
        );
        assert_eq!(
            Pattern::parse("https://*.example*"),
            Err(PatternError::UnanchoredAuthorityWildcard)
        );

        // Anchored forms stay legal, and at least the crafted-path
        // sibling escape is out: matching `.example.com/` needs a
        // literal in the target, which `bank.example.evil.com` (the
        // domain anyone can register) does not carry.
        assert!(Pattern::parse("https://*.example.com/*").is_ok());
        assert!(Pattern::parse("https://bank.example/*").is_ok());
        assert!(
            !Pattern::new("https://*.example.com/*").matches("https://api.example.com.evil.com/"),
            "the crafted-path escape does not fire here"
        );
        // This is a spelling check, not a host guarantee: `*` consumes
        // `/` too, so a URL steering the literals into its query still
        // matches. Allow rules that need a host guarantee use exact
        // hosts.
        assert!(
            Pattern::new("https://*.example.com/*")
                .matches("https://evil.example/x?q=.example.com/"),
            "text matching remains steerable by crafted URLs"
        );
        assert!(
            Pattern::parse("*").is_ok(),
            "a bare pattern names no host and is an explicit catch-all"
        );
    }

    #[test]
    fn deserialization_goes_through_parse() {
        // The derived Deserialize used to bypass `parse` and admit a
        // blank pattern as a dead rule that matches nothing.
        assert!(serde_json::from_str::<Pattern>("\"\"").is_err());
        assert!(serde_json::from_str::<Pattern>("\"   \"").is_err());
        assert!(serde_json::from_str::<Pattern>("\"https://bank.example*\"").is_err());
        assert!(serde_json::from_str::<Pattern>("\"https://bank.example/*\"").is_ok());
    }
}
