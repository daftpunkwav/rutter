//! Tests for `Settings::resolve`: flag values win over defaults, and
//! the navigation deadline is pinned to the documented default.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::path::PathBuf;
use std::time::Duration;

use rutter_cli::config::Settings;

#[test]
fn explicit_values_win_over_defaults() {
    let settings = Settings::resolve(
        Some(PathBuf::from("browser.exe")),
        Some(PathBuf::from("cache")),
        Vec::new(),
    )
    .expect("resolve");
    assert_eq!(
        settings.engine_executable,
        Some(PathBuf::from("browser.exe"))
    );
    assert_eq!(settings.cache_root, PathBuf::from("cache"));
    // Navigation deadline pinned by docs/sessions.md (context caps).
    assert_eq!(settings.navigation_timeout, Duration::from_secs(30));
}

#[test]
fn extra_engine_args_pass_through_verbatim() {
    let settings = Settings::resolve(
        None,
        Some(PathBuf::from("cache")),
        vec!["--no-sandbox".to_owned(), "--lang=en".to_owned()],
    )
    .expect("resolve");
    assert_eq!(
        settings.extra_engine_args,
        vec!["--no-sandbox", "--lang=en"]
    );
}
