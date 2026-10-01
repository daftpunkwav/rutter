# cli/ agent rules

The module map lives in [README.md](README.md) and
[src/README.md](src/README.md). Workspace crate rules live in
[../AGENTS.md](../AGENTS.md).

## Boundary

- This crate is the composition root. The package name is `rutter`.
  The library target is `rutter_cli`. It depends on every workspace
  crate except `rutter-events`.
- New behavior belongs in a lower crate. This crate parses arguments,
  resolves settings, wires processes, and presents errors.
- `main.rs` calls `rutter_cli::entry::run`. The library's public
  modules are `config`, `entry`, and `error`. `policy_file` is also
  public.
- `launcher.rs` is the only module that constructs a `CdpLauncher`.

## Entry modes

- No arguments: browse mode. A headed supervisor runs until Ctrl-C,
  then `shutdown`. This mode does not attach the dashboard.
- `serve`: MCP, stdio unless `--http`. The engine is headless unless
  `--headed`. `--dashboard PORT` and `--policy FILE` are optional.
  `LazyLauncher` defers engine I/O until the first launch.
- `serve` constructs one `ApprovalBroker`, passes it to
  `SessionManager`, and shuts that manager down on client disconnect,
  Ctrl-C, and serve failure.
- A non-loopback `--http` address is refused unless `--allow-remote`
  is set.
- `open`: one snapshot on stdout, then exit. No session and no policy.
- `read`: one markdown readout on stdout, then exit. No session and
  no policy.
- A failed `open` or `read` still shuts the engine down.
- Headed launch sets `--app` only when this crate resolved the
  executable itself. An explicit `--engine-executable` does not
  receive `--app`.

## Tests

Library tests live in this crate's `tests/`. Binary acceptance lives
in the workspace `tests/` package.
