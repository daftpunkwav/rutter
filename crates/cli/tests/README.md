# tests/ — end-to-end acceptance

Drives the built `rutter serve` binary as an rmcp client against the
real engine (`#[ignore]`d by default; run with `--ignored`):

| File | Covers |
|---|---|
| `mcp_e2e.rs` | TOOL_SPEC §6 task classes: read (navigate+snapshot), interact (click/type/select), form task, wait_for, close_session fail-fast |
| `approval_e2e.rs` | Policy gating: require_approval parks, dashboard decision path, deny/timeout |
| `http_e2e.rs` | Streamable HTTP transport: one session per connection over `/mcp` |

Each test spawns a child `rutter` process — keep them behavioral
(assert tool results), not implementation-coupled.
