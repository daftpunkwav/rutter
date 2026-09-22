# tests/ — per-crate functional tests

English | [中文](README.zh.md)

Public-API tests for this crate only.

| File | Covers |
|---|---|
| `token.rs` | Per-launch token properties: 16 hex characters, unique per server instance |
| `http_flow.rs` | The real server over loopback: the token gate on every route, the query-token-to-cookie exchange, decision posts (`400`/`404`/`200`), the pending count, and the 404 fallback |
| `ws_flow.rs` | The real WebSocket loop: the engine-off note, replay-then-live ordering, decision acks, screencast/subscribe control frames, and the refused upgrade without a token |
| `common/mod.rs` | Shared scriptable engine double (launcher → engine → context → page) so the flow tests run without a browser |
