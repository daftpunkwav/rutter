//! The approval brief exactly as the dashboard reads it.
//!
//! `frontend/src/app.js` has no build step and shares no schema file with
//! Rust, so the key names and the internal tags pinned here are the only
//! contract between what policy decides and what a human is shown. Change
//! one side and this test is where the mismatch surfaces.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use rutter_core::action::Action;
use rutter_core::reference::Reference;
use rutter_policy::{ActionClass, ApprovalBrief, ApprovalEffect, VerdictBasis};
use serde_json::json;

fn value(brief: &ApprovalBrief) -> serde_json::Value {
    serde_json::to_value(brief).expect("a brief always serializes")
}

#[test]
fn an_action_approval_carries_its_class_url_rule_and_effect() {
    let brief = ApprovalBrief {
        class: ActionClass::Pointer,
        judged_url: Some("https://shop.example/checkout".to_owned()),
        basis: VerdictBasis::Rule {
            index: 3,
            url_pattern: Some("https://*.shop.example/*".to_owned()),
        },
        effect: ApprovalEffect::Action {
            action: Action::Click {
                reference: Reference::new("e17"),
            },
        },
    };

    assert_eq!(
        value(&brief),
        json!({
            "class": "pointer",
            "judged_url": "https://shop.example/checkout",
            "basis": {
                "kind": "rule",
                "index": 3,
                "url_pattern": "https://*.shop.example/*"
            },
            "effect": { "kind": "action", "action": { "type": "click", "reference": "e17" } }
        })
    );
}

#[test]
fn a_cookie_approval_says_cookies_rather_than_an_action() {
    let brief = ApprovalBrief {
        class: ActionClass::Cookies,
        judged_url: None,
        basis: VerdictBasis::MissingUrl,
        effect: ApprovalEffect::Cookies { count: 2 },
    };

    assert_eq!(
        value(&brief),
        json!({
            "class": "cookies",
            "judged_url": null,
            "basis": { "kind": "missing_url" },
            "effect": { "kind": "cookies", "count": 2 }
        }),
        "the fail-closed path is visible as a null target, not a placeholder URL"
    );
}

#[test]
fn a_default_verdict_is_labeled_as_such() {
    let brief = ApprovalBrief {
        class: ActionClass::Navigation,
        judged_url: Some("https://any.example/".to_owned()),
        basis: VerdictBasis::SetDefault,
        effect: ApprovalEffect::Action {
            action: Action::Back,
        },
    };

    assert_eq!(
        value(&brief)["basis"],
        json!({ "kind": "set_default" }),
        "the frontend branches on this tag"
    );
}
