# tests/ — per-crate functional tests

English | [中文](README.zh.md)

Public-API tests for this crate only.

| File | Covers |
|---|---|
| `policy_flow.rs` | TOML parsing, first-match rule evaluation with URL patterns, the approval broker's park/decide/timeout lifecycle |
| `brief_wire.rs` | The approval brief's serde wire shape — the contract with the schema-less dashboard frontend |
