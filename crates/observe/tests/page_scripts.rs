//! The embedded page scripts are executed, not just read.
//!
//! `assets.rs` can prove the scripts are embedded and ASCII, but only
//! a JavaScript runtime can prove that the ref store sweeps, that a
//! colspan header does not clip the rows under it, or that a readout
//! past its budget owns up to being cut. The behaviour lives in
//! `tests/js/` (see `scripts/check_js.sh`); this test runs the two
//! observe suites from `cargo test` so the proof travels with the
//! crate instead of living only in a CI step.
//!
//! The run is skipped, not failed, when Node is missing or too old:
//! the scripts still compile into the binary without it, and a
//! contributor without Node must not be blocked by a test they cannot
//! act on. CI runs this on every platform.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::path::PathBuf;
use std::process::Command;

/// The repository root, from this crate's manifest directory.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("the workspace root exists")
}

/// `Some(major)` when a new enough `node` is on the path.
fn node_major() -> Option<u32> {
    let output = Command::new("node").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// The suites this test runs, with the number of tests each holds.
const SUITES: [(&str, usize); 3] = [
    ("tests/js/serializer.test.mjs", 13),
    ("tests/js/reader.test.mjs", 17),
    ("tests/js/select.test.mjs", 4),
];

#[test]
fn the_injected_page_scripts_behave_as_their_contracts_say() {
    let Some(major) = node_major() else {
        eprintln!("skipping: node is not installed");
        return;
    };
    if major < 18 {
        eprintln!("skipping: node {major} is older than 18");
        return;
    }

    let root = repo_root();
    let paths: Vec<&str> = SUITES.iter().map(|(path, _)| *path).collect();
    let output = Command::new("node")
        .current_dir(&root)
        .args(["--test", "--test-reporter=tap"])
        .args(paths)
        .output()
        .expect("node --test runs");
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "the injected page scripts must behave as tests/js says:\n{report}"
    );

    // A runner pointed at a file that no longer holds tests exits
    // zero, so the pass count is what proves the suites really ran.
    let expected: usize = SUITES.iter().map(|(_, count)| count).sum();
    let passed = report
        .lines()
        .filter(|line| line.starts_with("ok ") && line.contains(" - "))
        .count();
    assert_eq!(
        passed, expected,
        "every suite test must have run; the counts are what keep this honest:\n{report}"
    );
}
