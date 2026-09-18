//! Functional tests for the policy flow through the crate's public
//! API: TOML parsing, first-match rule evaluation, and the approval
//! broker's park/decide/timeout lifecycle.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::time::Duration;

use rutter_policy::class::ActionClass;
use rutter_policy::{ApprovalBroker, Decision, Verdict};

#[test]
fn rules_match_class_and_url_with_first_match_wins() {
    let rules = rutter_policy::parse_policy(
        "default = \"allow\"\n\
         [[rules]]\n\
         action_class = \"pointer\"\n\
         verdict = \"deny\"\n\
         [[rules]]\n\
         action_class = \"navigation\"\n\
         url_pattern = \"*example.com*\"\n\
         verdict = \"require_approval\"\n",
    )
    .expect("valid policy");

    let pointer = ActionClass::parse("pointer").expect("pointer is a class");
    let navigation = ActionClass::parse("navigation").expect("navigation is a class");

    // First match wins: pointer clicks are denied everywhere.
    assert_eq!(
        rules.evaluate(pointer, "https://example.com/page"),
        Verdict::Deny
    );
    // Navigation is unsupervised off-site, approved on example.com.
    assert_eq!(
        rules.evaluate(navigation, "https://example.com/page"),
        Verdict::RequireApproval
    );
    assert_eq!(
        rules.evaluate(navigation, "https://other.org/page"),
        Verdict::Allow
    );
}

#[test]
fn unmatched_actions_take_the_default_verdict() {
    let rules = rutter_policy::parse_policy("default = \"deny\"\n").expect("valid policy");
    let selection = ActionClass::parse("selection").expect("selection is a class");
    assert_eq!(
        rules.evaluate(selection, "https://example.com"),
        Verdict::Deny
    );
}

#[tokio::test]
async fn broker_resolves_a_granted_approval() {
    let broker = ApprovalBroker::new();
    let (id, receiver) = broker.open();

    assert_eq!(broker.pending_count(), 1);
    assert!(broker.decide(&id, Decision::Grant), "the open id decides");
    assert!(
        !broker.decide(&id, Decision::Grant),
        "a spent id is refused"
    );

    let outcome = broker.wait(&id, receiver, Duration::from_secs(1)).await;
    assert_eq!(outcome, rutter_policy::ApprovalOutcome::Granted);
}

#[tokio::test]
async fn broker_times_out_unanswered_approvals() {
    let broker = ApprovalBroker::new();
    let (id, receiver) = broker.open();

    let outcome = broker.wait(&id, receiver, Duration::from_millis(50)).await;
    assert_eq!(outcome, rutter_policy::ApprovalOutcome::TimedOut);
}
