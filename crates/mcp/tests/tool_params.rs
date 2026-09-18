//! Functional tests for the MCP tool input types through the crate's
//! public API: schema-driven deserialization and the mapping onto
//! `rutter-core` vocabulary (TOOL_SPEC §4). Session lifecycle tests
//! live in `src/server/tests.rs` because they exercise private paths.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use rutter_core::action::ScrollDirection;
use rutter_core::cookie::{Cookie, SameSite};
use rutter_mcp::server::{CookieInput, Direction, SameSiteInput};

#[test]
fn cookie_inputs_map_with_defaults() {
    let input = CookieInput {
        name: "session".to_owned(),
        value: "42".to_owned(),
        domain: "example.com".to_owned(),
        path: None,
        secure: None,
        http_only: None,
        same_site: Some(SameSiteInput::Lax),
    };
    let cookie = Cookie::try_from(&input).expect("cookie");
    assert_eq!(cookie.name, "session");
    assert_eq!(cookie.path, None);
    assert!(!cookie.secure);
    assert_eq!(cookie.same_site, Some(SameSite::Lax));
}

#[test]
fn unknown_same_site_is_rejected_by_deserialization() {
    // The schema enumerates strict|lax|none (TOOL_SPEC §4), so an
    // unknown policy is invalid_params at the deserialization layer.
    let error = serde_json::from_str::<CookieInput>(
        r#"{"name":"s","value":"42","domain":"example.com","same_site":"sloppy"}"#,
    )
    .expect_err("unknown policy");
    let message = error.to_string();
    assert!(message.contains("sloppy"), "names the bad value: {message}");
    assert!(
        message.contains("unknown variant"),
        "names the enum rejection: {message}"
    );
}

#[test]
fn scroll_directions_map_onto_the_core_vocabulary() {
    // The scroll tool's validation relies on this mapping staying
    // identity-shaped.
    for (input, expected) in [
        (Direction::Up, ScrollDirection::Up),
        (Direction::Down, ScrollDirection::Down),
        (Direction::Left, ScrollDirection::Left),
        (Direction::Right, ScrollDirection::Right),
    ] {
        assert_eq!(ScrollDirection::from(input), expected);
    }
}
