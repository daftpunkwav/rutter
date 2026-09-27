//! Access control for every dashboard endpoint (docs/dashboard.md): the
//! `Host` header must name loopback (DNS rebinding), a present `Origin`
//! must name loopback too (cross-site request forgery; the `Host` check
//! alone cannot see who sent the request), and the request must carry
//! the per-launch token, exchanged for an HttpOnly cookie on the first
//! visit.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::collections::HashMap;

use axum::http::HeaderMap;

use crate::Dashboard;

/// Whether `name` (a host with any port already stripped) is a loopback
/// name. Shared by the `Host` and `Origin` checks so the two gates can
/// never drift apart.
fn loopback_name(name: &str) -> bool {
    name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost") || name == "::1"
}

/// Host header validation: only loopback names pass (docs/dashboard.md,
/// DNS rebinding). The host name is compared after stripping any port;
/// a prefix match would let `127.0.0.1.evil.com` through, and the
/// check runs on every endpoint, including the WebSocket upgrade.
fn host_allowed(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(axum::http::header::HOST)
        .and_then(|host| host.to_str().ok())
    else {
        return false;
    };
    let name = if let Some(rest) = host.strip_prefix('[') {
        // Bracketed IPv6 literal: "[::1]:port" -> "::1".
        rest.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    loopback_name(name)
}

/// Origin header validation: a request that carries an `Origin` is a
/// browser submission, and the dashboard answers state-changing ones
/// (`POST /api/decisions` skips CORS preflight with a simple
/// content-type), so the origin must be the dashboard's own loopback —
/// matched exactly, the same way as the `Host` check, since a prefix
/// match would pass `http://localhost.evil.com`. No `Origin` header
/// means the caller is not a browser form or fetch; the `Host` and
/// token checks still apply. `Origin: null` (a sandboxed frame) is not
/// a same-origin submission.
fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|origin| origin.to_str().ok())
    else {
        return true;
    };
    let Some((_scheme, authority)) = origin.split_once("://") else {
        return false;
    };
    let name = if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    loopback_name(name)
}

fn token_ok(state: &Dashboard, headers: &HeaderMap, provided: Option<&String>) -> bool {
    let provided = provided
        .map(String::to_owned)
        .or_else(|| cookie_token(headers));
    provided.is_some_and(|token| constant_time_eq(&token, &state.token))
}

/// Name of the HttpOnly cookie carrying the dashboard token after the
/// first visit (docs/dashboard.md: the query token is exchanged for a
/// cookie on first connect).
const TOKEN_COOKIE: &str = "rutter_token";

/// Extracts the token cookie from `Cookie` headers, if present.
fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|header| header.split(';'))
        .find_map(|part| {
            let part = part.trim();
            part.strip_prefix(&format!("{TOKEN_COOKIE}="))
                .map(str::to_owned)
        })
}

/// Whether the token can travel in a cookie value: generated tokens
/// are hex, so an automation override with characters outside the
/// cookie-safe set keeps using the query parameter only.
pub(crate) fn cookie_safe(token: &str) -> bool {
    token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
}

/// The `Set-Cookie` header exchanging the query token for a session
/// cookie; `HttpOnly` keeps page scripts from reading it back.
pub(crate) fn token_cookie_header(token: &str) -> Option<axum::http::HeaderValue> {
    if !cookie_safe(token) {
        return None;
    }
    axum::http::HeaderValue::from_str(&format!(
        "{TOKEN_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict"
    ))
    .ok()
}

/// Equality without early exit; tokens are short so this is cheap.
/// The loop always runs over the longer input and the length difference
/// folds into the accumulator, so neither a matching prefix nor the
/// secret's length can shortcut the comparison.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = (a.len() ^ b.len()) as u32;
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= u32::from(x ^ y);
    }
    diff == 0
}

/// Gate every endpoint shares: the `Host` header must name loopback, a
/// present `Origin` must name loopback too, and the request must carry
/// the token (query parameter or cookie). Any new route must pass
/// through this check.
pub(crate) fn access_allowed(
    state: &Dashboard,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> bool {
    host_allowed(headers) && origin_allowed(headers) && token_ok(state, headers, query.get("token"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie_headers(parts: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for part in parts {
            headers.append(
                axum::http::header::COOKIE,
                axum::http::HeaderValue::from_str(part).expect("cookie header"),
            );
        }
        headers
    }

    #[test]
    fn cookie_token_is_extracted_from_mixed_headers() {
        let headers = cookie_headers(&["a=1; rutter_token=abc123", "b=2"]);
        assert_eq!(cookie_token(&headers).as_deref(), Some("abc123"));
        assert_eq!(cookie_token(&HeaderMap::new()), None);
    }

    #[test]
    fn constant_time_eq_accepts_only_exact_matches() {
        assert!(constant_time_eq("0123abcd", "0123abcd"));
        assert!(constant_time_eq("", ""));
        // Mismatched lengths, including a proper prefix of the secret,
        // must not compare equal.
        assert!(!constant_time_eq("0123abcd", "0123abc"));
        assert!(!constant_time_eq("0123abc", "0123abcd"));
        assert!(!constant_time_eq("", "x"));
        assert!(!constant_time_eq("0123abcd", "0123abce"));
    }

    #[test]
    fn cookie_safety_rejects_separator_characters() {
        assert!(cookie_safe("0123abcdEF-_.~"));
        assert!(!cookie_safe("a b"));
        assert!(!cookie_safe("a;b"));
    }

    #[test]
    fn cookie_header_carries_httponly_and_samesite() {
        let header = token_cookie_header("0123abcd").expect("safe token");
        let text = header.to_str().expect("ascii header");
        assert!(text.contains("rutter_token=0123abcd"));
        assert!(text.contains("HttpOnly"));
        assert!(text.contains("SameSite=Strict"));
        assert!(token_cookie_header("not safe").is_none());
    }

    #[test]
    fn origin_allows_loopback_and_rejects_cross_site() {
        let headers = |origin: Option<&str>| {
            let mut map = HeaderMap::new();
            if let Some(origin) = origin {
                map.insert(
                    axum::http::header::ORIGIN,
                    axum::http::HeaderValue::from_str(origin).expect("ascii origin"),
                );
            }
            map
        };
        // No Origin header: not a browser submission; Host and token
        // still gate the request.
        assert!(origin_allowed(&headers(None)));
        assert!(origin_allowed(&headers(Some("http://127.0.0.1:7700"))));
        assert!(origin_allowed(&headers(Some("http://localhost:7700"))));
        assert!(origin_allowed(&headers(Some("http://[::1]:7700"))));

        // The host is matched exactly, like the Host check: an origin
        // that embeds a loopback prefix is a cross-site submission from
        // a sibling domain.
        assert!(!origin_allowed(&headers(Some(
            "http://localhost.evil.com:7700"
        ))));
        assert!(!origin_allowed(&headers(Some(
            "http://127.0.0.1.evil.com/"
        ))));
        assert!(!origin_allowed(&headers(Some("https://evil.com"))));
        // `Origin: null` (a sandboxed frame) is not a same-origin
        // submission, and a scheme-less value is not an origin at all.
        assert!(!origin_allowed(&headers(Some("null"))));
        assert!(!origin_allowed(&headers(Some("evil.com"))));
    }

    #[test]
    fn host_allowed_accepts_loopback_names_only() {
        let headers = |host: &str| {
            let mut map = HeaderMap::new();
            map.insert(
                axum::http::header::HOST,
                axum::http::HeaderValue::from_str(host).expect("ascii host"),
            );
            map
        };
        assert!(host_allowed(&headers("127.0.0.1:7700")));
        assert!(host_allowed(&headers("127.0.0.1")));
        assert!(host_allowed(&headers("localhost:7700")));
        assert!(host_allowed(&headers("LOCALHOST")));
        assert!(host_allowed(&headers("[::1]:7700")));

        // Prefix matches must fail: a rebinding host like
        // `127.0.0.1.evil.com` would otherwise pass a starts-with check.
        assert!(!host_allowed(&headers("127.0.0.1.evil.com")));
        assert!(!host_allowed(&headers("localhost.evil.com")));
        // Suffix matches must fail too, and the check runs without a
        // Host header at all.
        assert!(!host_allowed(&headers("evil.com")));
        assert!(!host_allowed(&headers("127.0.0.2")));
        assert!(!host_allowed(&HeaderMap::new()));
    }
}
