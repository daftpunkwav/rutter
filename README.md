# rutter

A single-binary, headless browser orchestration service for AI agents,
written in Rust. rutter manages a real browser engine as a supervised
child process, exposes a structured view of pages (accessibility-tree
snapshots) instead of pixels, executes typed actions with deterministic
semantics, and offers a local supervision dashboard with human approval
for sensitive operations.

Status: milestone M0 works end to end — `rutter open` navigates and
prints snapshots through a real Chrome for Testing engine. The MCP
server surface is milestone M1. The canonical architecture reference is
[`docs/BLUEPRINT.md`](docs/BLUEPRINT.md); the snapshot contract is
[`docs/SNAPSHOT_SPEC.md`](docs/SNAPSHOT_SPEC.md).

## Usage

```sh
# One-shot diagnostic: navigate and print a YAML snapshot to stdout.
rutter open https://example.com

# Browse mode (no subcommand): a headed engine window you drive by hand.
rutter

# MCP server: milestone M1; reports itself as unavailable for now.
rutter serve --headed
```

Global flags (valid on every mode):

| Flag | Environment | Meaning |
|------|-------------|---------|
| `--engine-executable <PATH>` | — | Use this browser binary; skips download and cache |
| `--cache-dir <DIR>` | `RUTTER_CACHE_DIR` | Engine cache root (default: OS cache dir + `rutter`) |
| `--engine-arg <ARG>` | — | Extra argument passed to the engine process (repeatable) |

### Engine acquisition

On first use rutter resolves a browser binary in this order: an
explicit `--engine-executable`, the engine cache, then the Chrome for
Testing stable channel (headless shell for `open`, full Chrome for
browse mode when no system browser is found). The download happens
once; the cached version is reused until the cache directory is
cleared, including offline. Browse mode prefers a system-installed
Chrome or Edge when present.

The engine is supervised: heartbeats detect a dead process, restarts
use capped exponential backoff, and a sliding-window circuit breaker
stops restart storms — a crashed engine surfaces as an error on
affected operations, never as a crash of rutter.

## Repository layout

```
rutter/
├── docs/         # blueprint, snapshot spec
├── scripts/      # quality-gate helpers run by CI
├── crates/       # workspace members (see below)
└── frontend/     # dashboard sources (added with milestone M2)
```

| Crate | Responsibility |
|-------|----------------|
| `rutter-core` | Shared domain vocabulary: actions, snapshots, references, errors |
| `rutter-engine` | Engine and page traits, binary downloader, supervisor |
| `rutter-engine-cdp` | The only crate that speaks CDP (chromiumoxide) |
| `rutter-observe` | In-page serializer plus the snapshot builder |
| `rutter` (cli) | Binary entry modes: browse, serve, open |

Further crates (`rutter-events`, `rutter-policy`, `rutter-session`,
`rutter-mcp`, `rutter-dashboard`) materialize with the milestone that
needs them; empty crates are forbidden by the blueprint (§10).

## Development

Requires a stable Rust toolchain (1.85 or newer).

```sh
cargo build
cargo test --all
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Quality gates, identical to CI:

```sh
bash scripts/check_headers.sh   # every source file opens with a header
bash scripts/check_encoding.sh  # tracked text files stay English-only
```

Engine integration tests need a real binary and are `#[ignore]`d by
default; run them explicitly (they reuse the cache `rutter open`
fills):

```sh
cargo test -p rutter-engine-cdp --test integration -- --ignored
```

`scripts/smoke_open.sh` runs the M0 acceptance corpus (10 real sites)
and prints a pass/fail summary.

## Policies

- English is the project's only working language; CI enforces this on
  all tracked text files.
- Commits follow Conventional Commits, English, imperative mood.
- License: Apache-2.0 (the documented default of open decision OD-3 in
  the blueprint; see `LICENSE`).
