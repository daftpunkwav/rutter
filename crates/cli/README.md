# rutter/ (cli) — binary entry

English | [中文](README.zh.md)

The `rutter` binary and the composition root: the only place that
depends on every workspace crate. Three entry modes — browse (headed
window, no MCP), serve (MCP over stdio or `--http`, optional
`--dashboard` and `--policy`), and open (one-shot snapshot) — all
resolve flags and the environment into a `Settings` and wire the
launcher, manager, broker, and servers.

## Boundary

Assembly and user-facing error presentation only. All behavior lives
in the crates it wires; every exit path (client disconnect, Ctrl-C)
shuts the manager down so the supervised browser never outlives the
server.

## Tests

`tests/` covers the library surface (settings, errors, entry modes).
Binary-level acceptance tests live in the workspace-root `tests/`
package.
