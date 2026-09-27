//! `--policy` loading: a broken file is a `CliError` like every other
//! startup failure — a hint line and the shared exit path — where it
//! used to bypass `CliError` for a bare `exit(2)` with no hint.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use rutter_cli::error::CliError;

/// Writes a policy fixture under a per-test name (the harness runs
/// tests in parallel threads) and returns its path.
fn write_policy(test: &str, content: &str) -> std::path::PathBuf {
    let path =
        std::env::temp_dir().join(format!("rutter-policy-{test}-{}.toml", std::process::id()));
    std::fs::write(&path, content).expect("write the policy fixture");
    path
}

#[test]
fn a_missing_policy_file_fails_with_a_hint() {
    let error = rutter_cli::policy_file(std::path::Path::new("/definitely/not/here.toml"))
        .expect_err("the file does not exist");
    assert!(matches!(error, CliError::PolicyFile { .. }));
    assert!(!error.hint().trim().is_empty(), "the hint is actionable");
}

#[test]
fn a_malformed_policy_file_fails_with_a_hint() {
    let path = write_policy("malformed", "default = \"yolo\"\n");
    let error = rutter_cli::policy_file(&path).expect_err("unknown verdict");
    let _ = std::fs::remove_file(&path);
    assert!(matches!(error, CliError::PolicyFile { .. }));
    assert!(
        error.to_string().contains("yolo"),
        "names the problem: {error}"
    );
    assert!(!error.hint().trim().is_empty(), "the hint is actionable");
}

#[test]
fn a_valid_policy_file_parses_into_the_rule_set() {
    let path = write_policy(
        "valid",
        "[[rules]]\naction_class = \"cookies\"\nverdict = \"deny\"\n",
    );
    let rules = rutter_cli::policy_file(&path).expect("a valid policy parses");
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        rules.evaluate(rutter_policy::ActionClass::Cookies, "https://x.example/"),
        rutter_policy::Verdict::Deny
    );
}
