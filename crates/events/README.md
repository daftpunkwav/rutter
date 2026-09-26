# rutter-events/ — event backbone

English | [中文](README.zh.md)

Typed semantic events (serde) with a global sequence number and RFC
3339 timestamp, fanned out on a tokio broadcast bus and kept in a
bounded per-session ring for replay.

## Boundary

Depends on `core`, plus `policy` for the one field an approval event
carries — the brief only policy can build (docs/policy.md). Publishing
is fire-and-forget: it never waits on consumers and never fails
(docs/events.md). Backpressure: the bus may drop
envelopes from slow subscribers (they resync from the rings), but
semantic events survive in the rings, up to ring capacity; screencast
frames are not events at all — they flow as binary dashboard frames.

## Consumers

- `session` publishes everything a session does.
- `dashboard` replays history on connect, then follows live.
