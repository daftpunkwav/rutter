# events/ agent rules

The module map lives in [README.md](README.md) and
[src/README.md](src/README.md). Workspace crate rules live in
[../AGENTS.md](../AGENTS.md). Delivery lives in
[../../docs/events.md](../../docs/events.md).

## Boundary

- Workspace dependencies: `rutter-core`, `rutter-policy`.
- The only production import from `rutter_policy` is `ApprovalBrief`,
  on the approval event.
- Other crates use `Backbone`. They do not publish on the bus or write
  rings themselves.
- `serde_json` is a dev-dependency. Production code does not parse
  event JSON.

## Delivery

- `publish` does not wait on a subscriber. A slow subscriber does not
  fail the publish.
- A `Lagged` subscriber resyncs with `replay`.
- Sequence allocation, ring insert, and bus send stay in one critical
  section.
- Semantic events remain in the per-session ring until capacity drops
  the oldest. Screencast frames are not events.
- `Event` uses serde `tag = "type"` and snake_case.
- `recorded_at` is RFC 3339 UTC. A clock failure records the epoch
  string and still publishes.
- A new variant updates `docs/events.md` and `docs/events.zh.md` in
  the same change.

## Close

`Backbone::forget` drops a closed session's ring. `close_session`
calls it, and so does recovery when a session closes during a
rebuild. A closed session does not keep a ring.
