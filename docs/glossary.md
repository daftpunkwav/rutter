# Glossary

English | [中文](glossary.zh.md)

The normative domain vocabulary. One concept, one term — code, docs,
and UI use these exact terms, and no synonyms exist for them. The
identifiers in [`crates/core/src/ids.rs`](../crates/core/src/ids.rs)
carry the type-level versions of the Context, Page, and Session
entries.

| Term | Meaning |
|---|---|
| Engine | A browser process managed by rutter (Chromium headless shell) |
| Context | Isolated cookie/storage unit inside an engine |
| Page | One tab/target inside a context |
| Session | One MCP client's workspace: its context, pages, and state |
| Snapshot | Token-budgeted accessibility-tree view of a page, with refs |
| Reference | Stable handle to an element, usable across snapshots |
| Action | Typed operation requested by an agent (click, type, …) |
| Verdict | Policy decision: `Allow` / `Deny` / `RequireApproval` |
| Event | Structured fact published on the event backbone |
| Origin | Attribution of an action's initiator: `Agent` or `Human` |
| Browse mode | Entry mode: headed engine session operated directly by a human |

## Identifiers

Identifiers are opaque strings ([`ids.rs`](../crates/core/src/ids.rs)):
callers must not parse or construct meaning from their content. The
layer that owns an object's lifecycle mints its identifier.

| Type | Names | Minted by |
|---|---|---|
| `SessionId` | `stdio-<pid>` over stdio; `http-<pid>-<n>` over streamable HTTP | the MCP layer at connection start |
| `ContextId` | engine-assigned | `rutter-engine-cdp` |
| `PageId` | engine-assigned | `rutter-engine-cdp` |
| `ApprovalId` | `apr-<n>`, serial per broker | [`ApprovalBroker`](../crates/policy/src/broker.rs) |
| `Reference` | `e<n>`, per-page counter reset on navigation | the in-page serializer ([snapshot format §4](snapshot-format.md#4-reference-minting-v1)) |

## Serialization conventions

Types that travel on the wire or inside event payloads share one
convention, so consumers name everything the same way
([`action.rs`](../crates/core/src/action.rs),
[`event.rs`](../crates/events/src/event.rs),
[`error.rs`](../crates/core/src/error.rs)):

- Tagged enums serialize with a `"type"` tag and `snake_case` names:
  `Event` (`"action_failed"`), `Action` (`"click"`), and `ActionError`
  (`"reference_expired"`). Untagged enums serialize as bare
  `snake_case` strings: `Verdict` (`"allow"`), `Origin` (`"agent"`),
  and `ScrollDirection` (`"up"`).
- Timestamps are RFC 3339 UTC strings; no locale-dependent formatting.
- Everything travels as UTF-8 JSON.
