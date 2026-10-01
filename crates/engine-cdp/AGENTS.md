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
- No `chromiumoxide` type appears in a `pub` signature.
- Reference lookup and file upload run `rutter-observe` scripts. Do
  not re-implement the page-side reference store.

## Calls

- Every CDP call goes through `error::with_deadline`.
- Browser failures fold into `EngineError`. CDP error text does not
  leave this crate.
- Headed mode adds `--app=about:blank` only when `app_window` is
  set. `CdpLauncher::new` leaves it false. `crates/cli` sets it only
  for a headed executable that crate resolved itself.
- Spawn sets `RUTTER_CDP_PORT` and `RUTTER_PROFILE`, nulls stdin and
  stdout, and sends stderr to a file. On Windows the child is created
  with `CREATE_NO_WINDOW`.
- The profile directory is created exclusively under the temp dir.
  `create_dir` failure, including a collision, is `LaunchFailed` and
  is not retried.
- `launch_secret` and the dashboard `generate_token` stay the same
  construction. A change to one updates the other.

## Tests

`tests/` drives a real engine and is `#[ignore]`d. One concern per
file. The binary resolver lives in `tests/common/`.
