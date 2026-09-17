//! Cookies as part of the domain vocabulary.
//!
//! Boundary: pure data passed to engine backends. Persistence across
//! restarts (storage state) is a session-layer concern (blueprint §7.4)
//! and arrives in M2.

use serde::{Deserialize, Serialize};

/// One cookie scoped to a domain, as agents set them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cookie {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// Domain the cookie belongs to, for example `example.com`.
    pub domain: String,
    /// Path scope; defaults to `/` when absent.
    pub path: Option<String>,
    /// Whether the cookie is sent over secure transports only.
    pub secure: bool,
    /// Whether the cookie is hidden from page scripts.
    pub http_only: bool,
    /// Cross-site sending policy.
    pub same_site: Option<SameSite>,
    /// Expiry as seconds since the Unix epoch; `None` is a session
    /// cookie. Captured storage state preserves this so logins survive
    /// restarts (blueprint §7.4).
    pub expires: Option<f64>,
}

/// Cross-site sending policy of a cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SameSite {
    /// Sent only in first-party contexts.
    Strict,
    /// Sent with top-level navigations.
    Lax,
    /// Sent cross-site (requires `secure`).
    None,
}
