//! Access control for every dashboard endpoint (docs/dashboard.md): the
//! `Host` header must name loopback (DNS rebinding) and the request
//! must carry the per-launch token, exchanged for an HttpOnly cookie
//! on the first visit.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::collections::HashMap;

use axum::http::HeaderMap;

use crate::Dashboard;

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
    name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost") || name == "::1"
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
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Gate every endpoint shares: the `Host` header must name loopback and
/// the request must carry the token (query parameter or cookie). Any
/// new route must pass through this check.
pub(crate) fn access_allowed(
    state: &Dashboard,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> bool {
    host_allowed(headers) && token_ok(state, headers, query.get("token"))
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
