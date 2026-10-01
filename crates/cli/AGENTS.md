# cli/ agent rules

The module map lives in [README.md](README.md) and
[src/README.md](src/README.md). Workspace crate rules live in
[../AGENTS.md](../AGENTS.md).

## Boundary

- This crate is the composition root. The package name is `rutter`.
  The library target is `rutter_cli`. Its workspace dependencies are
  the `rutter` row in [../AGENTS.md](../AGENTS.md). It does not depend
  on `rutter-events`.
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
- `serve` constructs one `ApprovalBroker` and passes it to
  `SessionManager`. Stdio shuts that manager down on client
  disconnect, Ctrl-C, and serve failure. HTTP shuts it down on Ctrl-C
  and on serve failure. One HTTP client disconnect does not.
- A non-loopback `--http` address is refused unless `--allow-remote`
  is set.
- `open` prints one snapshot on stdout, then exits. `read` prints one
  markdown readout on stdout, then exits. Neither mode creates a
  session or applies a rule set. A global `--policy` flag is still
  parsed before dispatch.
- A failed `open` or `read` still shuts the engine down.
- Headed launch sets `--app` only when this crate resolved the
  executable itself. An explicit `--engine-executable` does not
  receive `--app`.

## Tests

Public flows live in this crate's `tests/`. Private behavior stays
in `src/` under `#[cfg(test)]`. Binary acceptance lives in the
workspace `tests/` package.
