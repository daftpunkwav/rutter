# rutter-engine-cdp/ — CDP backend

Implements `rutter-engine`'s traits over chromiumoxide. This is the
only crate in the workspace allowed to name CDP or chromiumoxide types
(blueprint §5); the public surface is `CdpLauncher`, an
`EngineLauncher` — every implementation module is private and no CDP
type appears in a public signature.

## Boundary

Depends on `core` and `engine` only. Consumers wire `CdpLauncher::new`
into a supervisor and never learn that CDP exists. All CDP calls run
under deadlines (`error::with_deadline`); browser-side surprises (a
target that vanished, an already-dead context) fold into the
protocol-neutral error taxonomy rather than leaking protocol text.

## Tests

`tests/` runs against a real downloaded engine and is `#[ignore]`d by
default (see its README).
