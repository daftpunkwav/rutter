//! Tests for action execution and the auto-wait phase budgets, run
//! against the mock page — no real browser, no network.

use super::*;
use crate::mock::{MockPage, missing_answer, mock_box};
use std::time::Duration;

/// Owns everything an `Executor` borrows, so tests can build one
/// without lifetime gymnastics.
struct Harness {
    session: rutter_core::ids::SessionId,
    page_id: rutter_core::ids::PageId,
    backbone: rutter_events::Backbone,
    page: MockPage,
    config: SessionConfig,
}

impl Harness {
    fn new(phase_timeout: Duration, interval: Duration) -> Self {
        Self {
            session: rutter_core::ids::SessionId::new("s-test"),
            page_id: rutter_core::ids::PageId::new("p1"),
            backbone: rutter_events::Backbone::new(),
            page: MockPage::new(),
            config: SessionConfig {
                phase_timeout,
                stability_sample_interval: interval,
                settle: Duration::ZERO,
                ..SessionConfig::default()
            },
        }
    }

    fn executor(&self) -> Executor<'_> {
        Executor {
            session: &self.session,
            page_id: &self.page_id,
            page: &self.page,
            config: &self.config,
            backbone: &self.backbone,
        }
    }
}

fn click(reference: &str) -> Action {
    Action::Click {
        reference: Reference::new(reference),
    }
}

#[tokio::test]
async fn click_a_ready_element_dispatches_at_its_center() {
    let harness = Harness::new(Duration::from_secs(1), Duration::from_millis(1));
    harness.page.set_url("https://example.com");
    // The box spans (10, 20)-(110, 50), so the click lands on (60, 35).
    harness
        .executor()
        .run(&click("e1"))
        .await
        .expect("click succeeds");
    let inputs = harness.page.inputs();
    assert_eq!(inputs.len(), 2, "press and release");
    assert!(matches!(
        inputs[0],
        rutter_engine::input::InputEvent::MousePressed {
            x: 60.0,
            y: 35.0,
            ..
        }
    ));
}

#[tokio::test]
async fn a_never_visible_element_times_out_naming_the_visible_phase() {
    let harness = Harness::new(Duration::from_millis(200), Duration::from_millis(20));
    harness.page.set_url("https://example.com");
    harness.page.set_cycle(true);
    // Hidden boxes keep failing the visibility phase.
    harness
        .page
        .push_resolve_answer(mock_box(true, false, 10.0, 20.0, 100.0, 30.0));
    let error = harness
        .executor()
        .run(&click("e1"))
        .await
        .expect_err("the element never becomes visible");
    match error {
        SessionError::Action(ActionError::TimedOut { phase, elapsed }) => {
            assert_eq!(phase, WaitPhase::Visible);
            assert_eq!(elapsed, Duration::from_millis(200));
        }
        other => panic!("expected a Visible-phase timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn a_moving_element_times_out_naming_the_stable_phase() {
    let harness = Harness::new(Duration::from_millis(200), Duration::from_millis(20));
    harness.page.set_url("https://example.com");
    harness.page.set_cycle(true);
    // Visible but never twice at the same place: the stable phase
    // cannot complete.
    harness
        .page
        .push_resolve_answer(mock_box(false, false, 10.0, 20.0, 100.0, 30.0));
    harness
        .page
        .push_resolve_answer(mock_box(false, false, 10.0, 60.0, 100.0, 30.0));
    let error = harness
        .executor()
        .run(&click("e1"))
        .await
        .expect_err("the element never stabilizes");
    match error {
        SessionError::Action(ActionError::TimedOut { phase, .. }) => {
            assert_eq!(phase, WaitPhase::Stable);
        }
        other => panic!("expected a Stable-phase timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn a_disabled_element_times_out_naming_the_enabled_phase() {
    let harness = Harness::new(Duration::from_millis(200), Duration::from_millis(20));
    harness.page.set_url("https://example.com");
    harness.page.set_cycle(true);
    // Visible and rock-steady but disabled: the enabled phase waits.
    harness
        .page
        .push_resolve_answer(mock_box(false, true, 10.0, 20.0, 100.0, 30.0));
    let error = harness
        .executor()
        .run(&click("e1"))
        .await
        .expect_err("the element never becomes enabled");
    match error {
        SessionError::Action(ActionError::TimedOut { phase, .. }) => {
            assert_eq!(phase, WaitPhase::Enabled);
        }
        other => panic!("expected an Enabled-phase timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn a_vanished_reference_fails_fast_without_burning_the_budget() {
    let harness = Harness::new(Duration::from_secs(30), Duration::from_millis(20));
    harness.page.set_url("https://example.com");
    harness.page.push_resolve_answer(missing_answer());
    let started = Instant::now();
    let error = harness
        .executor()
        .run(&click("e1"))
        .await
        .expect_err("the reference is gone");
    assert!(matches!(
        error,
        SessionError::Action(ActionError::ReferenceExpired { .. })
    ));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a gone reference must fail fast, not wait out the budget"
    );
}

#[tokio::test]
async fn falling_back_to_an_earlier_phase_does_not_restart_the_clock() {
    // The element moves for 8 samples (Stable pending), then hides
    // and stays hidden (Visible pending again). A fall-back must NOT
    // restart the budget: the wait ends one budget after the wait
    // started. An implementation that resets the clock on every
    // phase change would grant the hidden phase a fresh budget at
    // the moment of the fall-back and only give up one budget later
    // — the wall-clock assertion tells the two apart. The sample
    // count stays low because Windows timer granularity stretches
    // each sleep well past its nominal 20 ms.
    let harness = Harness::new(Duration::from_millis(600), Duration::from_millis(20));
    harness.page.set_url("https://example.com");
    for index in 0..8 {
        let y = if index % 2 == 0 { 20.0 } else { 80.0 };
        harness
            .page
            .push_resolve_answer(mock_box(false, false, 10.0, y, 100.0, 30.0));
    }
    harness
        .page
        .set_default_answer(mock_box(true, false, 0.0, 0.0, 0.0, 0.0));
    let started = Instant::now();
    let outcome =
        tokio::time::timeout(Duration::from_secs(5), harness.executor().run(&click("e1")))
            .await
            .expect("the wait must end in a timeout, never hang");
    match outcome.expect_err("the element never becomes clickable") {
        SessionError::Action(ActionError::TimedOut { phase, elapsed }) => {
            assert_eq!(phase, WaitPhase::Visible);
            assert_eq!(elapsed, Duration::from_millis(600));
        }
        other => panic!("expected a Visible-phase timeout, got {other:?}"),
    }
    assert!(
        // 950 ms still separates the two implementations (a clock
        // reset ends one full budget later) while leaving room for
        // Windows timer granularity under parallel test load.
        started.elapsed() < Duration::from_millis(950),
        "the fall-back must not extend the wait beyond the original budget"
    );
}

#[test]
fn wheel_deltas_follow_directions() {
    assert_eq!(wheel_deltas(ScrollDirection::Down, 300), (0.0, 300.0));
    assert_eq!(wheel_deltas(ScrollDirection::Up, 300), (0.0, -300.0));
    assert_eq!(wheel_deltas(ScrollDirection::Right, 120), (120.0, 0.0));
    assert_eq!(wheel_deltas(ScrollDirection::Left, 120), (-120.0, 0.0));
}
