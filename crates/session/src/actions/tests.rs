//! Tests for action execution and the auto-wait phase budgets, run
//! against the mock page — no real browser, no network.

use super::*;
use crate::mock::{MockPage, missing_answer, mock_box};
use serde_json::json;
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

#[tokio::test]
async fn a_flickering_enabled_state_cannot_restart_the_budget_forever() {
    // The element keeps cycling visible+stable+disabled, hidden,
    // visible+stable+disabled: every re-appearance advances the wait
    // Visible → Enabled, and each advance used to push the deadline
    // out again, so the wait could stretch one budget per flicker.
    // The restart cap bounds the total: the wait ends within a few
    // budgets, reporting the phase it last advanced to. The outer
    // timeout tells this implementation from an uncapped one, whose
    // deadline recedes forever and never gives up.
    let harness = Harness::new(Duration::from_millis(300), Duration::from_millis(20));
    harness.page.set_url("https://example.com");
    let enabled = mock_box(false, true, 10.0, 20.0, 100.0, 30.0);
    let hidden = mock_box(true, false, 0.0, 0.0, 0.0, 0.0);
    harness.page.push_resolve_answer(enabled.clone());
    harness.page.push_resolve_answer(enabled);
    harness.page.push_resolve_answer(hidden);
    harness.page.set_cycle(true);
    let started = Instant::now();
    let outcome =
        tokio::time::timeout(Duration::from_secs(8), harness.executor().run(&click("e1")))
            .await
            .expect("the wait must end in a timeout, never hang");
    match outcome.expect_err("the element never becomes clickable") {
        SessionError::Action(ActionError::TimedOut { phase, elapsed }) => {
            // The phase at expiry lands wherever the flicker cycle is when
            // the capped budget runs out — visible, stable, or enabled all
            // name an honest answer. The regression this test pins is the
            // wait *ending* (the uncapped clock recedes forever), so only
            // the budget and the wall clock are asserted.
            assert!(matches!(
                phase,
                WaitPhase::Visible | WaitPhase::Stable | WaitPhase::Enabled
            ));
            assert_eq!(elapsed, Duration::from_millis(300));
        }
        other => panic!("expected a timeout, got {other:?}"),
    }
    // Three budgets (initial plus two capped restarts) bound the wait;
    // generous slack absorbs Windows timer granularity under load.
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "the flicker must not stretch the wait past the capped budget: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_slow_but_monotonic_progression_still_finishes() {
    // The other side of the restart cap: a genuinely progressing
    // element (hidden, then visible but never twice in one place, then
    // steady but disabled, then ready) advances Visible → Stable →
    // Enabled, and each advance must restart the budget. Only the
    // shipped cap of 2 — exactly what a monotonic progression can
    // spend — lets the ladder finish and the click dispatch:
    //
    //   cap 0: no advance ever re-arms the clock, so the initial
    //          budget dies while the element is still wandering.
    //   cap 1: the second advance (Stable → Enabled) is refused a
    //          restart, and the ready flip lands after the deadline
    //          that first restart left — one budget past the first.
    //
    // The schedule keeps every boundary a full sampling window away
    // from the deadlines it must fall on either side of (t=100 first
    // advance, t≈350+4 samples second, deadline≈600 cap-1 expiry,
    // t=700 ready), so Windows timer granularity under parallel test
    // load shifts the outcome by tens of milliseconds, never past a
    // boundary.
    let harness = Harness::new(Duration::from_millis(500), Duration::from_millis(20));
    harness.page.set_url("https://example.com");
    let hidden = mock_box(true, false, 0.0, 0.0, 0.0, 0.0);
    let wander = mock_box(false, true, 10.0, 20.0, 100.0, 30.0);
    let wander_away = mock_box(false, true, 10.0, 80.0, 100.0, 30.0);
    let steady_disabled = mock_box(false, true, 10.0, 20.0, 100.0, 30.0);
    let ready = mock_box(false, false, 10.0, 20.0, 100.0, 30.0);

    harness.page.set_default_answer(hidden);
    let page = harness.page.clone();
    let schedule = async move {
        // Hidden for a fifth of a budget, then visible but jumping
        // between two places: the Stable phase pends here, and its
        // restart carries the wait past this window.
        tokio::time::sleep(Duration::from_millis(100)).await;
        page.push_resolve_answer(wander);
        page.push_resolve_answer(wander_away);
        page.set_cycle(true);
        // Steady but disabled: cycle off drains the two queued answers
        // first (both at new places, so still unstable), then two
        // samples in one place make the Enabled phase pend, spending
        // the second restart.
        tokio::time::sleep(Duration::from_millis(250)).await;
        page.set_cycle(false);
        page.set_default_answer(steady_disabled);
        // Ready after the deadline a cap of 1 would have left the wait
        // with (one budget past the first restart), well inside the
        // budget the second restart hands out.
        tokio::time::sleep(Duration::from_millis(350)).await;
        page.set_default_answer(ready);
    };
    let executor = harness.executor();
    let (snapshot, ()) = tokio::join!(
        async {
            tokio::time::timeout(Duration::from_secs(8), executor.run(&click("e1")))
                .await
                .expect("the wait must end, never hang")
                .expect("a monotonic progression finishes inside the capped budget")
        },
        schedule,
    );
    let inputs = harness.page.inputs();
    assert_eq!(inputs.len(), 2, "the click dispatches once it is ready");
    assert!(matches!(
        inputs[0],
        rutter_engine::input::InputEvent::MousePressed {
            x: 60.0,
            y: 35.0,
            ..
        }
    ));
    assert!(
        snapshot.to_string().contains("Ok"),
        "the action answers with the fresh snapshot"
    );
}

#[test]
fn wheel_deltas_follow_directions() {
    assert_eq!(wheel_deltas(ScrollDirection::Down, 300), (0.0, 300.0));
    assert_eq!(wheel_deltas(ScrollDirection::Up, 300), (0.0, -300.0));
    assert_eq!(wheel_deltas(ScrollDirection::Right, 120), (120.0, 0.0));
    assert_eq!(wheel_deltas(ScrollDirection::Left, 120), (-120.0, 0.0));
}

#[tokio::test]
async fn setting_files_targets_the_resolved_input() {
    let harness = Harness::new(Duration::from_secs(1), Duration::from_millis(1));
    harness.page.set_url("https://example.com");
    let file = std::env::temp_dir().join("rutter-upload-test.txt");
    std::fs::write(&file, b"payload").expect("fixture file writes");
    let path = file.to_string_lossy().into_owned();

    let snapshot = harness
        .executor()
        .run(&Action::SetInputFiles {
            reference: Reference::new("e1"),
            paths: vec![path.clone()],
        })
        .await
        .expect("the upload succeeds");
    assert!(!snapshot.truncated, "a fresh snapshot comes back");
    assert_eq!(
        harness.page.input_files_calls(),
        vec![("e1".to_owned(), vec![path])]
    );
    let _ = std::fs::remove_file(&file);
}

#[tokio::test]
async fn an_upload_without_files_is_refused_before_the_page_is_touched() {
    // An empty `paths` list has nothing to set; the refusal happens in
    // the executor, before the resolver or the engine is involved, so
    // no upload call can reach the page.
    let harness = Harness::new(Duration::from_secs(1), Duration::from_millis(1));
    harness.page.set_url("https://example.com");

    let error = harness
        .executor()
        .run(&Action::SetInputFiles {
            reference: Reference::new("e1"),
            paths: vec![],
        })
        .await
        .expect_err("an empty upload is refused");
    let SessionError::Action(ActionError::NotInteractable { reason, .. }) = error else {
        panic!("unexpected error: {error}")
    };
    assert!(
        reason.contains("no files"),
        "the refusal says why: {reason}"
    );
    assert!(harness.page.input_files_calls().is_empty());
}

#[tokio::test]
async fn setting_files_on_a_non_file_input_is_refused() {
    let harness = Harness::new(Duration::from_secs(1), Duration::from_millis(1));
    harness
        .page
        .set_file_check(json!({ "missing": false, "not_file": true }));
    let error = harness
        .executor()
        .run(&Action::SetInputFiles {
            reference: Reference::new("e1"),
            paths: vec!["whatever.txt".to_owned()],
        })
        .await
        .expect_err("a non-input is refused");
    let SessionError::Action(ActionError::NotInteractable { reason, .. }) = error else {
        panic!("unexpected error: {error}")
    };
    assert!(reason.contains("not a file input"));
    assert!(harness.page.input_files_calls().is_empty());
}

#[tokio::test]
async fn a_single_file_input_refuses_two_files() {
    let harness = Harness::new(Duration::from_secs(1), Duration::from_millis(1));
    harness.page.set_file_check(
        json!({ "missing": false, "not_file": false, "ok": true, "multiple": false }),
    );
    let error = harness
        .executor()
        .run(&Action::SetInputFiles {
            reference: Reference::new("e1"),
            paths: vec!["a.txt".to_owned(), "b.txt".to_owned()],
        })
        .await
        .expect_err("two files on a single input are refused");
    let SessionError::Action(ActionError::NotInteractable { reason, .. }) = error else {
        panic!("unexpected error: {error}")
    };
    assert!(reason.contains("single file"));
    assert!(harness.page.input_files_calls().is_empty());
}

#[tokio::test]
async fn a_missing_path_is_caller_feedback_not_an_engine_failure() {
    let harness = Harness::new(Duration::from_secs(1), Duration::from_millis(1));
    let error = harness
        .executor()
        .run(&Action::SetInputFiles {
            reference: Reference::new("e1"),
            paths: vec!["Z:/definitely/missing/rutter.txt".to_owned()],
        })
        .await
        .expect_err("a missing file is refused");
    let SessionError::Action(ActionError::NotInteractable { reason, .. }) = error else {
        panic!("unexpected error: {error}")
    };
    assert!(reason.contains("file not found"));
    assert!(harness.page.input_files_calls().is_empty());
}

#[tokio::test]
async fn an_expired_reference_fails_the_upload_fast() {
    let harness = Harness::new(Duration::from_secs(1), Duration::from_millis(1));
    harness.page.set_file_check(json!({ "missing": true }));
    let error = harness
        .executor()
        .run(&Action::SetInputFiles {
            reference: Reference::new("e1"),
            paths: vec!["a.txt".to_owned()],
        })
        .await
        .expect_err("a missing reference is refused");
    assert!(matches!(
        error,
        SessionError::Action(ActionError::ReferenceExpired { .. })
    ));
}
