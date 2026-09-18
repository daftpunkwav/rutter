# src/ — file map

| File | Role |
|---|---|
| `event.rs` | The event vocabulary (serde `tag = "type"`, snake_case) |
| `envelope.rs` | `Envelope`: seq + timestamp + session + event, as consumers see it |
| `bus.rs` | Tokio broadcast fan-out; capacity bounds slow-subscriber loss |
| `ring.rs` | Bounded per-session ring, oldest evicted, `history()` for replay |
| `backbone.rs` | `Backbone`: publish + subscribe + replay, the only type others touch |

Publishing never blocks; a `Lagged` subscriber resyncs through
`replay` — both dashboard code paths rely on that pairing.
