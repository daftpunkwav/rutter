# rutter-core/ — domain vocabulary

English | [中文](README.zh.md)

The protocol-neutral language every other crate speaks: typed actions,
snapshots, element references, cookies, ids, and the error taxonomy
agents see. Pure data plus small pure functions — no I/O, no async, no
engine knowledge.

## Boundary

Depends on nothing inside the workspace. Every cross-crate signature is
phrased in these types, so changing them is a workspace-wide contract
change (docs/architecture.md).

## Key types

- `Action` + `Origin` — what an agent asked for, and who asked.
  Serialized with the event convention (`"type"` tag, snake_case)
  because they ride in events.
- `Snapshot` / `SnapshotNode` — the YAML-rendered accessibility view
  (format contract: `docs/snapshot-format.md`).
- `Reference` — stable per-page element handle minted by the
  serializer, invalidated by navigation.
- `ActionError` — the failure vocabulary; serialized with the event
  convention (`"type"` tag, snake_case) because it rides in events.
