//! Shared fixtures for the cross-crate integration tests: locating the
//! built `rutter` binary.

use std::path::PathBuf;

/// Path of the built `rutter` binary. Inside the binary's own package
/// cargo injects `CARGO_BIN_EXE_rutter`; from this standalone test
/// package the workspace target layout is used, with `RUTTER_BIN` as
/// an override for non-standard build dirs (coverage runs, release).
pub fn rutter_bin() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_rutter") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("RUTTER_BIN") {
        return PathBuf::from(path);
    }
    let mut path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../target/debug/rutter"
    ));
    path.set_extension(std::env::consts::EXE_EXTENSION);
    path
}
