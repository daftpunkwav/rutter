# builder/ — snapshot pipeline

English | [中文](README.zh.md)

| File | Role |
|---|---|
| `mod.rs` | `build()`: convert → cut depth → fold sibling runs → viewport culling → fit to the character budget → render |
| `tests.rs` | Golden snapshots (insta), property tests (proptest), spec-rule unit tests |

The budgets and rules are SNAPSHOT_SPEC §6 verbatim; changing them is
a contract change. `tests.rs` pins the spec so the pipeline can be
reworked without drift.
