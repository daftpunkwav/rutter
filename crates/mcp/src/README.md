# src/ — file map

| Item | Role |
|---|---|
| `lib.rs` | Exports `RutterMcp`; crate-level boundary statement |
| `server/` | The rmcp server: tools, params, result mapping (see its README) |
| `http.rs` | Streamable HTTP transport (`/mcp`); warns on non-loopback binds |

Transports are thin: stdio lives in `cli` (rmcp's `serve`), HTTP here.
Anything that starts validating business rules in this crate is a
sign the rule belongs in `session`.
