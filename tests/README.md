# tests/ — cross-crate integration

English | [中文](README.zh.md)

Acceptance tests spanning crate boundaries: each one spawns the built
`rutter` binary — the MCP suites drive it as an MCP client, `open_e2e`
exercises the one-shot diagnostic — so the whole stack
(client → transport → mcp → session → engine-cdp) runs together.
The engine-touching tests are `#[ignore]`d by default; run them with
`--ignored` (needs the engine binary in the cache).

| Target | Covers |
|---|---|
| `mcp_e2e.rs` | Full task flows over stdio: navigate + snapshot/markdown reading, a page-mutating click, form type/select with screenshot, wait_for, close_session fail-fast |
| `approval_e2e.rs` | Policy gating over the real binary: park, grant/deny, timeout |
| `http_e2e.rs` | Streamable HTTP transport end to end |
| `open_e2e.rs` | The `rutter open` one-shot diagnostic: exactly one snapshot on stdout, and a missing engine exits with a failure and a hint (that case needs no engine) |
| `read_e2e.rs` | The `rutter read` one-shot scrape: the page's markdown document on stdout |

The binary is located via `CARGO_BIN_EXE_rutter` when set, else the
`RUTTER_BIN` override, else the workspace `target/debug` layout.
