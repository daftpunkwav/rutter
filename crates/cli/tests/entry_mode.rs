//! Tests for `EntryMode`: the modes display by their CLI names,
//! which error messages and logs rely on.

// Restriction lints are denied workspace-wide; tests may use plain
// assertions and unwrapping on fixtures.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use rutter_cli::entry::EntryMode;

#[test]
fn modes_display_by_name() {
    assert_eq!(EntryMode::Browse.to_string(), "browse");
    assert_eq!(EntryMode::Serve { headed: false }.to_string(), "serve");
    assert_eq!(
        EntryMode::Open {
            url: "https://example.com".to_owned()
        }
        .to_string(),
        "open"
    );
}

#[test]
fn modes_compare_by_value() {
    // The dispatch maps CLI output onto these variants; equality keeps
    // that mapping testable without running a mode.
    assert_eq!(EntryMode::Browse, EntryMode::Browse);
    assert_ne!(
        EntryMode::Serve { headed: false },
        EntryMode::Serve { headed: true }
    );
    assert_ne!(
        EntryMode::Open {
            url: "a".to_owned()
        },
        EntryMode::Open {
            url: "b".to_owned()
        }
    );
}
