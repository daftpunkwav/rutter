# snapshot_builder/ — snapshot pipeline

English | [中文](README.zh.md)

| File | Role |
|---|---|
| `mod.rs` | `build()`: convert → cut depth → fold sibling runs → viewport culling → fit to the character budget → render |
| `tests.rs` | Golden snapshots (insta), property tests (proptest), spec-rule unit tests |

The budgets and rules follow docs/snapshot-format.md verbatim; changing them is
a contract change. `tests.rs` pins the spec so the pipeline can be
reworked without drift.
