# core/ agent rules

The type map lives in [README.md](README.md) and
[src/README.md](src/README.md). Workspace crate rules live in
[../AGENTS.md](../AGENTS.md). The glossary is
[../../docs/glossary.md](../../docs/glossary.md).

## Boundary

- No workspace dependencies.
- No I/O, no async, no engine types, no transport types.
- New domain vocabulary lands here before a consumer uses it.

## Shapes

- `Action` and `ActionError` serialize with serde tag `"type"` and
  snake_case.
- `Origin` serializes as a plain snake_case string.
- `Snapshot` text follows
  [../../docs/snapshot-format.md](../../docs/snapshot-format.md).
- `Readout` follows
  [../../docs/read-format.md](../../docs/read-format.md).
- `SessionId`, `PageId`, and `ContextId` are opaque. Callers do not
  parse meaning out of the string.

## Tests

Public-API coverage lives in `tests/domain_vocabulary.rs`.
