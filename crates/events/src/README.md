# src/ — file map

English | [中文](README.zh.md)

| File | Role |
|---|---|
| `event.rs` | The event vocabulary (serde `tag = "type"`, snake_case) |
| `envelope.rs` | `Envelope`: seq + timestamp + session + event, as consumers see it |
| `bus.rs` | Tokio broadcast fan-out; capacity bounds slow-subscriber loss |
| `ring.rs` | Bounded per-session ring, oldest evicted, `history()` for replay |
| `backbone.rs` | `Backbone`: publish + subscribe + replay, the only type others touch |

Publishing never waits on consumers; a `Lagged` subscriber resyncs
through
`replay` — both dashboard code paths rely on that pairing. The counter
and the rings share one guard, so numbering, recording, and fan-out form
a single critical section: ring order and live order both equal
allocation order.
