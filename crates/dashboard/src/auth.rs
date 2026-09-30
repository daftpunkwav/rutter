//! Access control for every dashboard endpoint: the
//! `Host` header must name loopback (DNS rebinding), a present `Origin`
//! must name the dashboard's own loopback origin — same host name *and*
//! the port the server actually bound (cross-site request forgery; the
//! `Host` check alone cannot see who sent the request), and the request
//! must carry the per-launch token, exchanged for an HttpOnly cookie on
//! the first visit.
//!
//! The other half of the credential story lives in [`harden`]: the
//! token rides in the query string of the first visit, so every response
//! tells the browser not to keep it, not to hand it to a `Referer`, and
//! not to let the approval UI be framed.

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

/// The host and the explicit port inside an `authority` (`host` or
/// `host:port`, bracketed IPv6 included). Shared by the `Host` and
/// `Origin` checks: the splitting rule is one rule, and a fix applied
/// to only one copy would let the two gates disagree about the same
/// name. `None` is returned for an authority whose port does not
/// parse, so a malformed one never reads as "no port".
fn split_authority(authority: &str) -> Option<(&str, Option<u16>)> {
    if let Some(rest) = authority.strip_prefix('[') {
        // Bracketed IPv6 literal: "[::1]:port" -> ("::1", Some(port)).
        let (host, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            None | Some("") => None,
            Some(port) => Some(port.parse().ok()?),
        };
        return Some((host, port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host, Some(port.parse().ok()?))),
        None => Some((authority, None)),
    }
}

/// The port an origin names. A browser serialises an origin's port only
/// when it differs from the scheme's default, so an authority without
/// one is read as that default rather than as "any port".
fn effective_port(scheme: &str, port: Option<u16>) -> Option<u16> {
    port.or(match scheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    })
}

/// Host header validation: only loopback names pass (the DNS
/// rebinding defense). The host name is compared after stripping any port;
/// a prefix match would let `127.0.0.1.evil.com` through, and the
/// check runs on every endpoint, including the WebSocket upgrade.
fn host_allowed(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(axum::http::header::HOST)
        .and_then(|host| host.to_str().ok())
    else {
        return false;
    };
    split_authority(host).is_some_and(|(name, _)| loopback_name(name))
}

/// Origin header validation: a request that carries an `Origin` is a
/// browser submission, and the dashboard answers state-changing ones
/// (`POST /api/decisions` skips CORS preflight with a simple
/// content-type), so the origin must be the dashboard's own — the same
/// loopback name *and* the port the server actually bound. The port is
/// the half that carries the defence: `SameSite=Strict` is a *site*
/// check and a site spans every port on a host, so a page served from
/// `http://127.0.0.1:<any other port>` is same-site with the dashboard,
/// has its cookie attached, and answers to the loopback half of this
/// check alone. Without the port an attacker who can put a page on any
/// loopback port cross-site submits an approval grant. The host name
/// is still matched exactly, since a prefix match would pass
/// `http://localhost.evil.com`. No `Origin` header means the caller is
/// not a browser form, fetch, or WebSocket handshake; the `Host` and
/// token checks still apply. `Origin: null` (a sandboxed frame) is not
/// a same-origin submission.
fn origin_allowed(headers: &HeaderMap, bound_port: u16) -> bool {
    let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|origin| origin.to_str().ok())
    else {
        return true;
    };
    let Some((scheme, authority)) = origin.split_once("://") else {
        return false;
    };
    let Some((name, port)) = split_authority(authority) else {
        return false;
    };
    loopback_name(name) && effective_port(scheme, port) == Some(bound_port)
}

fn token_ok(state: &Dashboard, headers: &HeaderMap, provided: Option<&String>) -> bool {
    if let Some(provided) = provided {
        return constant_time_eq(provided, &state.token);
    }
    // Every candidate is answered, not just the first: cookies carry no
    // port scope, so any local server on this host can plant a
    // `rutter_token` cookie, and one planted earlier sorts ahead of the
    // dashboard's own. A first-match check would let that planted copy
    // shadow the real one and lock the operator out; a planted value can
    // never match, so answering all of them only removes the shadowing.
    cookie_tokens(headers).any(|token| constant_time_eq(&token, &state.token))
}

/// Name of the HttpOnly cookie carrying the dashboard token after the
/// first visit (the query token is exchanged for a cookie on first
/// connect).
const TOKEN_COOKIE: &str = "rutter_token";

/// Every `rutter_token` cookie value in the request, in header order.
fn cookie_tokens(headers: &HeaderMap) -> impl Iterator<Item = String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|header| header.split(';'))
        .filter_map(|part| {
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
/// present `Origin` must name the dashboard's own origin, and the
/// request must carry the token (query parameter or cookie). Any new
/// route must pass through this check.
pub(crate) fn access_allowed(
    state: &Dashboard,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> bool {
    host_allowed(headers)
        && origin_allowed(headers, state.port)
        && token_ok(state, headers, query.get("token"))
}

/// The response headers every dashboard answer carries, as
/// (name, value) pairs.
///
/// They close the three ways the token-bearing URL escapes the gate
/// that [`access_allowed`] puts around it:
///
/// - `Cache-Control: no-store` keeps the first-visit URL — the one
///   place the token itself appears — out of the browser's disk cache
///   and out of the back/forward cache.
/// - `Referrer-Policy: no-referrer` keeps it out of the `Referer` of
///   anything the page ever loads or navigates to. The dashboard loads
///   nothing cross-origin today; the header is what makes that a
///   property of the server rather than of the frontend.
/// - `X-Frame-Options: DENY` and `Content-Security-Policy:
///   frame-ancestors 'none'` keep the approval UI out of somebody
///   else's frame, where an overlay could turn a click on "Grant"
///   into the operator's consent to the wrong brief. The cookie's
///   `SameSite=Strict` does not cover the first visit: that one
///   authenticates from the query, so nothing but these headers stops
///   a holder of the URL from framing the card the token opened.
/// - `X-Content-Type-Options: nosniff` keeps a browser from reading
///   `/app.js` or `/i18n/en.json` as anything but what they are.
pub(crate) const HARDENING_HEADERS: [(&str, &str); 5] = [
    ("cache-control", "no-store"),
    ("referrer-policy", "no-referrer"),
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "DENY"),
    ("content-security-policy", "frame-ancestors 'none'"),
];

/// Stamps [`HARDENING_HEADERS`] onto one response. Applied by a single
/// middleware so a route added later inherits them instead of having
/// to remember them; `insert` (not `append`) keeps the server the only
/// writer of these names.
pub(crate) fn harden(response: &mut axum::response::Response) {
    for (name, value) in HARDENING_HEADERS {
        // Both halves are static ASCII chosen in this file, so the
        // conversion cannot fail; a header that failed to parse would
        // silently leave the response unprotected.
        let (Ok(name), Ok(value)) = (
            axum::http::HeaderName::from_bytes(name.as_bytes()),
            axum::http::HeaderValue::try_from(value),
        ) else {
            continue;
        };
        response.headers_mut().insert(name, value);
    }
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
    fn cookie_tokens_are_extracted_from_mixed_headers() {
        let headers = cookie_headers(&["a=1; rutter_token=abc123", "b=2"]);
        assert_eq!(
            cookie_tokens(&headers).collect::<Vec<_>>(),
            vec!["abc123".to_owned()]
        );
        assert_eq!(cookie_tokens(&HeaderMap::new()).count(), 0);
    }

    #[test]
    fn a_planted_cookie_cannot_shadow_the_dashboard_s_own() {
        // Cookies have no port scope: a local server on any port of this
        // host can plant a `rutter_token` cookie, and one planted earlier
        // sorts ahead of the dashboard's own in the Cookie header. The
        // gate must answer every copy — the planted value never matches,
        // and the real one behind it must still be reached.
        let headers = cookie_headers(&["rutter_token=planted", "rutter_token=0123abcd"]);
        let state = |token: &str| Dashboard {
            manager: crate::test_support::test_manager(),
            broker: std::sync::Arc::new(rutter_policy::ApprovalBroker::new()),
            token: token.to_owned(),
            port: 7700,
            keepalive: std::time::Duration::from_secs(20),
        };
        assert!(token_ok(&state("0123abcd"), &headers, None));
        // A value that matches neither cookie never authenticates.
        assert!(!token_ok(&state("not-the-token"), &headers, None));
        // No cookie and no query parameter never authenticates.
        assert!(!token_ok(&state("0123abcd"), &HeaderMap::new(), None));
        // A query parameter wins over whatever the cookies carry.
        let provided = "0123abcd".to_owned();
        assert!(token_ok(&state("0123abcd"), &headers, Some(&provided)));
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
    fn origin_allows_the_dashboards_own_origin_and_rejects_cross_site() {
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
        assert!(origin_allowed(&headers(None), 7700));
        // The dashboard's own origin, under either loopback name the
        // operator may have typed it as.
        assert!(origin_allowed(
            &headers(Some("http://127.0.0.1:7700")),
            7700
        ));
        assert!(origin_allowed(
            &headers(Some("http://localhost:7700")),
            7700
        ));
        assert!(origin_allowed(&headers(Some("http://[::1]:7700")), 7700));
        // A port the authority omits is the scheme's default, not "any
        // port" — so a dashboard on 80 still accepts its own origin.
        assert!(origin_allowed(&headers(Some("http://127.0.0.1")), 80));
        assert!(!origin_allowed(&headers(Some("http://127.0.0.1")), 7700));

        // The host is matched exactly, like the Host check: an origin
        // that embeds a loopback prefix is a cross-site submission from
        // a sibling domain.
        assert!(!origin_allowed(
            &headers(Some("http://localhost.evil.com:7700")),
            7700
        ));
        assert!(!origin_allowed(
            &headers(Some("http://127.0.0.1.evil.com/")),
            7700
        ));
        assert!(!origin_allowed(
            &headers(Some("https://evil.com:7700")),
            7700
        ));
        // `Origin: null` (a sandboxed frame) is not a same-origin
        // submission, and a scheme-less value is not an origin at all.
        assert!(!origin_allowed(&headers(Some("null")), 7700));
        assert!(!origin_allowed(&headers(Some("evil.com")), 7700));
    }

    #[test]
    fn origin_on_another_loopback_port_is_not_the_dashboards_own() {
        // The cross-site request forgery the port half of the check
        // exists for. `SameSite=Strict` is a *site* check and a site
        // spans every port on a host, so a page served from
        // `http://127.0.0.1:9999` is same-site with the dashboard, has
        // the token cookie attached to its request, and is a loopback
        // origin. Matching the host alone let it POST a grant to
        // `/api/decisions` and open a WebSocket that submits decisions
        // of its own — a different port is what makes it cross-site.
        let headers = |origin: &str| {
            let mut map = HeaderMap::new();
            map.insert(
                axum::http::header::ORIGIN,
                axum::http::HeaderValue::from_str(origin).expect("ascii origin"),
            );
            map
        };
        assert!(!origin_allowed(&headers("http://127.0.0.1:9999"), 7700));
        assert!(!origin_allowed(&headers("http://localhost:9999"), 7700));
        assert!(!origin_allowed(&headers("http://[::1]:9999"), 7700));
        // And a port that is not a number never reads as a match.
        assert!(!origin_allowed(&headers("http://127.0.0.1:7700evil"), 7700));
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

    /// Header value of `name` on `response`, as text.
    fn header(response: &axum::response::Response, name: &str) -> Option<String> {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    #[test]
    fn hardening_keeps_the_token_out_of_caches_and_referrers() {
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        harden(&mut response);
        assert_eq!(
            header(&response, "cache-control").as_deref(),
            Some("no-store")
        );
        assert_eq!(
            header(&response, "referrer-policy").as_deref(),
            Some("no-referrer")
        );
        assert_eq!(
            header(&response, "x-content-type-options").as_deref(),
            Some("nosniff")
        );
    }

    #[test]
    fn hardening_refuses_to_let_the_approval_ui_be_framed() {
        // The clickjacking half: `SameSite=Strict` protects the cookie,
        // but the first visit authenticates from the query string, so
        // a holder of the URL could otherwise frame the card its own
        // token opened and overlay the Grant button.
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        harden(&mut response);
        assert_eq!(
            header(&response, "x-frame-options").as_deref(),
            Some("DENY")
        );
        assert_eq!(
            header(&response, "content-security-policy").as_deref(),
            Some("frame-ancestors 'none'")
        );
    }

    #[test]
    fn hardening_replaces_rather_than_appends() {
        // The server is the only writer of these names: a duplicate
        // would let a later writer append a second policy and a
        // browser that honours the first one would answer differently
        // from one that honours the last.
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        response.headers_mut().insert(
            "x-frame-options",
            axum::http::HeaderValue::from_static("SAMEORIGIN"),
        );
        harden(&mut response);
        let values: Vec<_> = response
            .headers()
            .get_all("x-frame-options")
            .iter()
            .map(|value| value.to_str().expect("ascii header").to_owned())
            .collect();
        assert_eq!(values, vec!["DENY".to_owned()], "exactly one policy");
    }
}
