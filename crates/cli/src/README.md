# src/ — file map

| File | Role |
|---|---|
| `lib.rs` | Library root: every module below is wired and exported here |
| `main.rs` | clap definitions and process entry (a thin shell over `rutter_cli`) |
| `entry.rs` | `EntryMode` enum: browse / serve / open |
| `config.rs` | `Settings::resolve`: flags + environment → runtime settings |
| `launcher.rs` | Engine launcher selection: cached CdpLauncher or explicit binary |
| `serve.rs` | MCP serving (stdio/HTTP) + dashboard/policy wiring; every exit path shuts the engine down |
| `browse.rs` | Headed, human-driven mode; Ctrl-C stops the supervisor |
| `open.rs` | One-shot: navigate once, print snapshot, exit |
| `error.rs` | `CliError` presentation: hints for engine, signal, transport failures |

This crate wires; it does not implement. New behavior belongs in a
lower crate unless it is genuinely about argument parsing or process
lifecycle.
