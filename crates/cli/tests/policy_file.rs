//! `--policy` loading: a broken file is a `CliError` like every other
//! startup failure — a hint line and the shared exit path — where it
//! used to bypass `CliError` for a bare `exit(2)` with no hint.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use rutter_cli::error::CliError;

/// Writes a policy fixture to a scratch file of the harness's own name
/// (the harness runs tests in parallel threads) and returns the handle:
/// the file lives as long as the test holds it, and dropping it removes
/// it. A predictable name under the shared temp dir would be a path
/// another local user could have planted a symlink at.
fn write_policy(content: &str) -> tempfile::NamedTempFile {
    let file = tempfile::Builder::new()
        .suffix(".toml")
        .tempfile()
        .expect("create the policy fixture");
    std::fs::write(file.path(), content).expect("write the policy fixture");
    file
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
    let fixture = write_policy("default = \"yolo\"\n");
    let error = rutter_cli::policy_file(fixture.path()).expect_err("unknown verdict");
    assert!(matches!(error, CliError::PolicyFile { .. }));
    assert!(
        error.to_string().contains("yolo"),
        "names the problem: {error}"
    );
    assert!(!error.hint().trim().is_empty(), "the hint is actionable");
}

#[test]
fn a_valid_policy_file_parses_into_the_rule_set() {
    let fixture = write_policy("[[rules]]\naction_class = \"cookies\"\nverdict = \"deny\"\n");
    let rules = rutter_cli::policy_file(fixture.path()).expect("a valid policy parses");
    assert_eq!(
        rules.evaluate(rutter_policy::ActionClass::Cookies, "https://x.example/"),
        rutter_policy::Verdict::Deny
    );
}
