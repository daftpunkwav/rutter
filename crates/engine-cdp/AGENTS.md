# engine-cdp/ agent rules

The module map lives in [README.md](README.md) and
[src/README.md](src/README.md). Workspace crate rules live in
[../AGENTS.md](../AGENTS.md).

## Boundary

- This is the only crate that depends on `chromiumoxide` or names CDP
  types.
- Workspace dependencies: `rutter-core`, `rutter-engine`,
  `rutter-observe`.
- The public type is `CdpLauncher`. Every other module is private.
- `chromiumoxide` types stay inside private modules.
- Reference lookup and file upload run `rutter-observe` scripts. Do
  not re-implement the page-side reference store.

## Calls

- Protocol commands go through `error::with_deadline` or
  `error::with_deadline_by`. The connect loop uses its own
  per-attempt timeout and the launch deadline, and retries until that
  deadline.
- Browser failures fold into `EngineError`. The crate re-exports
  `CdpLauncher` only. `EngineError` details may carry the CDP message
  text.
- Headed mode adds `--app=about:blank` only when `app_window` is
  set. `CdpLauncher::new` leaves it false. `crates/cli` sets it only
  for a headed executable that crate resolved itself.
- Spawn sets `RUTTER_CDP_PORT` and `RUTTER_PROFILE`, nulls stdin and
  stdout, and sends stderr to a file. On Windows the child is created
  with `CREATE_NO_WINDOW`.
- The profile directory is created exclusively under the temp dir,
  owner-only on Unix (`0700`): it holds the browser's cookies and login
  state, and a directory made with the process umask would be readable
  by every other local user. Creation failure, including a collision,
  is `LaunchFailed` and is not retried.
- `launch_secret` and the dashboard `generate_token` stay the same
  construction. A change to one updates the other.

## Tests

`tests/` drives a real engine and is `#[ignore]`d. One concern per
file. The binary resolver lives in `tests/common/`.
