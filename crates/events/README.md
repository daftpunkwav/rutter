# rutter-events/ — event backbone

English | [中文](README.zh.md)

Typed semantic events (serde) with a global sequence number and RFC
3339 timestamp, fanned out on a tokio broadcast bus and kept in a
bounded per-session ring for replay.

## Boundary

Depends on `core` only. Publishing is fire-and-forget and never blocks
action execution (docs/events.md). Backpressure: the bus may drop
envelopes from slow subscribers (they resync from the rings), but
semantic events are never dropped from the rings; screencast frames
are not events at all — they flow as binary dashboard frames.

## Consumers

- `session` publishes everything a session does.
- `dashboard` replays history on connect, then follows live.
