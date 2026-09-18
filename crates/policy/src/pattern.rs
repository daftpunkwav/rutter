//! URL patterns for policy rules: `*` wildcards, nothing else.

use serde::{Deserialize, Serialize};

/// A URL pattern with `*` wildcards; `https://*.example.com/*` matches
/// scheme, host, and any path. Anything a wildcard must consume is
/// matched non-greedily by segments of literal text between wildcards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pattern(String);

impl Pattern {
    /// Wraps a pattern string without validation; use [`Pattern::parse`]
    /// to reject empty patterns.
    pub fn new(pattern: impl Into<String>) -> Self {
        Self(pattern.into())
    }

    /// Parses a pattern, rejecting empty ones.
    pub fn parse(pattern: &str) -> Option<Self> {
        (!pattern.trim().is_empty()).then(|| Self(pattern.to_owned()))
    }

    /// The pattern source text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `url` matches this pattern. Matching is case-sensitive
    /// except for the scheme and host, which URLs lowercase anyway.
    pub fn matches(&self, url: &str) -> bool {
        wildcard_match(self.0.as_bytes(), url.as_bytes())
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
    fn parse_rejects_empty_patterns() {
        assert!(Pattern::parse("").is_none());
        assert!(Pattern::parse("  ").is_none());
        assert!(Pattern::parse("https://x/*").is_some());
    }
}
