# rutter/ (cli) — binary entry

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

`tests/` drives the built binary as an MCP client against the real
engine (`#[ignore]`d; see its README).
