//! Functional tests for the MCP tool input types through the crate's
//! public API: schema-driven deserialization and the mapping onto
//! `rutter-core` vocabulary. Session lifecycle tests
//! live in `src/server/tests.rs` because they exercise private paths.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use rutter_core::action::ScrollDirection;
use rutter_core::cookie::{Cookie, SameSite};
use rutter_mcp::server::{
    CookieInput, CookiesParams, PressKeyParams, SameSiteInput, ScrollDirectionInput,
    UploadFileParams,
};

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
        expires: None,
    };
    let cookie = Cookie::try_from(&input).expect("cookie");
    assert_eq!(cookie.name, "session");
    assert_eq!(cookie.path, None);
    assert!(!cookie.secure);
    assert_eq!(cookie.same_site, Some(SameSite::Lax));
    assert_eq!(cookie.expires, None, "omitted expiry is a session cookie");
}

#[test]
fn cookie_input_carries_the_expiry_through() {
    // The write side speaks the same expiry vocabulary the read side
    // reports (`get_cookies`, storage state): seconds since the Unix
    // epoch, `None` a session cookie.
    let input = CookieInput {
        name: "session".to_owned(),
        value: "42".to_owned(),
        domain: "example.com".to_owned(),
        path: None,
        secure: None,
        http_only: None,
        same_site: None,
        expires: Some(1_800_000_000.0),
    };
    let cookie = Cookie::try_from(&input).expect("cookie");
    assert_eq!(cookie.expires, Some(1_800_000_000.0));
}

#[test]
fn unknown_same_site_is_rejected_by_deserialization() {
    // The schema enumerates strict|lax|none, so an
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
        (ScrollDirectionInput::Up, ScrollDirection::Up),
        (ScrollDirectionInput::Down, ScrollDirection::Down),
        (ScrollDirectionInput::Left, ScrollDirection::Left),
        (ScrollDirectionInput::Right, ScrollDirection::Right),
    ] {
        assert_eq!(ScrollDirection::from(input), expected);
    }
}

#[test]
fn the_schema_says_what_the_server_refuses() {
    // The server answers an empty `key`, an empty `paths`, and an empty
    // `cookies` batch with `invalid_params`; the published schema must
    // carry the same rule, or a validating client and the server
    // disagree about which inputs are legal.
    for schema in [
        serde_json::to_value(schemars::schema_for!(PressKeyParams)).expect("schema json"),
        serde_json::to_value(schemars::schema_for!(UploadFileParams)).expect("schema json"),
        serde_json::to_value(schemars::schema_for!(CookiesParams)).expect("schema json"),
    ] {
        let text = serde_json::to_string(&schema).expect("schema text");
        assert!(
            text.contains("minLength") || text.contains("minItems"),
            "the non-empty rule is in the schema: {text}"
        );
    }
}
