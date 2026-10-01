# mcp/ agent rules

The module map lives in [README.md](README.md) and
[src/README.md](src/README.md). Workspace crate rules live in
[../AGENTS.md](../AGENTS.md). The tool contract is
[../../docs/tool-catalog.md](../../docs/tool-catalog.md).

## Boundary

- Normal workspace dependencies: `rutter-core`, `rutter-session`.
- `rutter-engine` and `rutter-policy` stay dev-dependencies.
  Production code reaches engine types through `rutter-session`
  re-exports.
- This crate declares `schemars` directly. The `JsonSchema` derive
  resolves the `schemars::` path.
- This crate maps the protocol. Parameter checks return
  `invalid_params`. Business rules belong in `session`.
- The engine starts on the first tool call that opens this
  connection's session. `console_messages` and `network_requests` do
  that and do not open a page. `close_session` on a connection that
  never opened a session does not launch an engine.

## Tools

- All `#[tool]` methods stay in the one `#[tool_router]` impl block
  in `src/server/mod.rs`.
- Tool names and parameter names match `docs/tool-catalog.md`. A
  rename updates that document and `docs/tool-catalog.zh.md` in their
  own docs commit first.
- Snapshots return text, including the `… truncated` marker.
  Screenshots return image blocks. Failures are `isError` results
  with a message and a hint.
- One MCP connection is one session. A closed session is not
  resurrected for a later tool call.

## HTTP

- `src/http.rs` serves streamable HTTP at `/mcp`.
- Origin validation is enforced with an empty allowlist. A browser
  `Origin` is rejected.
- The bind path prints that the transport has no authentication, on
  every address. A non-loopback address also prints a warning.
- `crates/cli` refuses a non-loopback `--http` address unless
  `--allow-remote` is set.
- Stdio serving lives in `crates/cli`. Each HTTP connection mints
  `http-<pid>-<n>`. Stdio uses `stdio-<pid>`.
