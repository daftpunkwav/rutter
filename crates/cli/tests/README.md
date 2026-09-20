# tests/ — per-crate functional tests

English | [中文](README.zh.md)

Tests the library target (`rutter_cli`) through its public API. The
binary-level acceptance tests live in the workspace-root `tests/`
package because they span every crate.

| File | Covers |
|---|---|
| `settings.rs` | `Settings::resolve`: flag precedence, pass-through engine args |
| `errors.rs` | `CliError`: hints present, engine errors keep their identity |
| `entry_mode.rs` | `EntryMode` display names and value equality |
