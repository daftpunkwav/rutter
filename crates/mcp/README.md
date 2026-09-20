# rutter-mcp/ — MCP tool surface

English | [中文](README.zh.md)

Exposes `docs/tool-catalog.md` as an rmcp server: stdio and streamable
HTTP transports, one MCP connection = one session. Snapshots return as
text (with the `… truncated` marker), screenshots as image blocks,
failures as `isError` results carrying message plus hint.

## Boundary

Protocol mapping only. Parameters are validated against the spec
(`invalid_params`); all semantics live in `rutter-session`, and this
crate never touches the engine itself. The engine starts lazily on the
first tool call that needs a page.

## Consumers

- `cli` runs the server over stdio (child-process clients) or
  `--http ADDR` (streamable HTTP, one session per connection).
