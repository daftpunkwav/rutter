# src/ — file map

| File | Role |
|---|---|
| `action.rs` | `Action` enum, `Origin`, `ScrollDirection` — the request vocabulary |
| `snapshot.rs` | `Snapshot`/`SnapshotNode` and the YAML rendering (SNAPSHOT_SPEC) |
| `reference.rs` | `Reference` — opaque per-page element handle |
| `cookie.rs` | `Cookie`/`SameSite` as passed to engine backends |
| `ids.rs` | Newtype ids (`SessionId`, `PageId`, `ContextId`, `ApprovalId`-shaped) |
| `error.rs` | `ActionError` taxonomy; serde tag matches the event convention |

Add new domain vocabulary here first, then teach one consumer at a
time. Anything with I/O or async does not belong in this crate.
