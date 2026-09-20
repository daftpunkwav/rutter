# tests/ — per-crate functional tests

English | [中文](README.zh.md)

Public-API tests for this crate only. Private-path unit tests live
beside their code in `src/` (Rust integration tests cannot reach crate
internals).

| File | Covers |
|---|---|
| `supervisor_flow.rs` | Supervisor lifecycle over scripted launchers: start/shutdown, heartbeat replacement, breaker, failed-start recovery |
