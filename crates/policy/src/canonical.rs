//! Canonical URL form for policy judgment (blueprint §7.6).
//!
//! Rules match on the URL the browser will actually load, and
//! agent-supplied navigation targets are raw strings: host case, a
//! default port, or a userinfo trick (`https://good.example@evil.example/`)
//! must not slip a denied host past a textual pattern, so every
//! judgment URL goes through the WHATWG parser first. `location.href`
//! is already canonical; reparsing it is a no-op that also guards
//! against engines that skip normalization.

use url::Url;

/// Parses `raw` into its canonical text form. `None` means the string
/// carries no usable URL information; callers must treat that as
/// fail-closed, not as a pattern-match failure. Two cases return
/// `None`: the string is not a URL at all, or it embeds credentials —
/// a `user@host` target would put the decoy before the `@` into the
/// judgment string while the browser contacts the host after it, so
/// such navigations go to a human instead of a pattern.
pub fn canonical_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    if url.username().is_empty() && url.password().is_none() {
        return Some(String::from(url));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_form_survives_case_and_default_ports() {
        assert_eq!(
            canonical_url("https://EVIL.example:443/a"),
            Some("https://evil.example/a".to_owned())
        );
        assert_eq!(
            canonical_url("http://Good.Example:80/x?Y=1#Z"),
            Some("http://good.example/x?Y=1#Z".to_owned())
        );
    }

    #[test]
    fn userinfo_targets_fail_closed_instead_of_masquerading() {
        // Serialization keeps the decoy before the `@`, so the only
        // honest canonical form is none at all: the caller fails
        // closed and a human sees the navigation.
        assert_eq!(canonical_url("https://good.example@evil.example/"), None);
        assert_eq!(canonical_url("https://user:pw@example.com/"), None);
    }

    #[test]
    fn non_urls_have_no_canonical_form() {
        assert_eq!(canonical_url("not a url"), None);
        assert_eq!(canonical_url(""), None);
        // A scheme-relative fragment is not an absolute URL either.
        assert_eq!(canonical_url("//example.com/x"), None);
    }

    #[test]
    fn script_urls_canonicalize_so_rules_can_target_them() {
        assert_eq!(
            canonical_url("javascript:void(0)"),
            Some("javascript:void(0)".to_owned())
        );
    }
}
