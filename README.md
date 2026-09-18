# rutter

A single-binary, headless browser orchestration service for AI agents,
written in Rust. rutter manages a real browser engine as a supervised
child process, exposes a structured view of pages (accessibility-tree
snapshots) instead of pixels, executes typed actions with deterministic
semantics, and offers a local supervision dashboard with human approval
for sensitive operations.

Status: milestones M0-M2 work end to end — `rutter open` prints
snapshots, `rutter serve` speaks MCP over stdio with the full tool
surface (navigate, snapshot, click, type, tabs, cookies, ...) against a
real, supervised Chrome for Testing engine, sessions carry a policy
with human approvals, storage state survives engine restarts, and the
supervision dashboard (live screencast, events, approvals) serves on
localhost. Packaging and the public 0.1.0 release remain (milestone
M3). The canonical architecture reference is
[`docs/BLUEPRINT.md`](docs/BLUEPRINT.md); contracts:
[`docs/SNAPSHOT_SPEC.md`](docs/SNAPSHOT_SPEC.md) and
[`docs/TOOL_SPEC.md`](docs/TOOL_SPEC.md).

## Usage

```sh
# One-shot diagnostic: navigate and print a YAML snapshot to stdout.
rutter open https://example.com

# MCP server over stdio: connect any MCP client (engine headless;
# --headed runs a visible window instead).
rutter serve

# The same MCP server over streamable HTTP, with the supervision
# dashboard and a policy file attached.
rutter serve --http 127.0.0.1:8080 --dashboard 7700 --policy policy.toml

# Browse mode (no subcommand): a headed engine window you drive by hand.
rutter
```

Global flags (valid on every mode):

| Flag | Environment | Meaning |
|------|-------------|---------|
| `--engine-executable <PATH>` | — | Use this browser binary; skips download and cache |
| `--cache-dir <DIR>` | `RUTTER_CACHE_DIR` | Engine cache root (default: OS cache dir + `rutter`) |
| `--engine-arg <ARG>` | — | Extra argument passed to the engine process (repeatable) |

Serve flags (on `rutter serve` only):

| Flag | Meaning |
|------|---------|
| `--http <ADDR>` | Serve MCP over streamable HTTP on that address instead of stdio |
| `--dashboard <PORT>` | Attach the supervision dashboard on 127.0.0.1:`<PORT>` |
| `--policy <FILE>` | Load the supervision rule set from a TOML file |

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

## Installation

From source (requires Rust 1.88+):

```sh
cargo install --path crates/cli
```

Release archives for Windows (msvc), macOS (x64/arm64), and Linux
(x64) are attached to GitHub releases by cargo-dist; see
[`.github/workflows/release.yml`](.github/workflows/release.yml). The
browser engine itself is not bundled — rutter downloads Chrome for
Testing into its cache on first use (or point `--engine-executable`
at an existing binary).

Contributions follow [`CONTRIBUTING.md`](CONTRIBUTING.md); the
architecture map is [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Repository layout

```
rutter/
├── docs/         # blueprint, snapshot spec (the contracts)
├── scripts/      # quality-gate helpers run by CI
├── crates/       # workspace members (see below)
└── frontend/     # dashboard sources (vanilla JS, no build step)
```

Every source directory carries a README stating its responsibility,
boundary, and file map — start at [`crates/README.md`](crates/README.md).

| Crate | Responsibility |
|-------|----------------|
| `rutter-core` | Shared domain vocabulary: actions, snapshots, references, errors |
| `rutter-engine` | Engine and page traits, binary downloader, supervisor |
| `rutter-engine-cdp` | The only crate that speaks CDP (chromiumoxide) |
| `rutter-observe` | In-page scripts plus the snapshot builder |
| `rutter-events` | Typed event backbone: bus, ring buffers, replay |
| `rutter-session` | Orchestration: contexts, pages, actions, auto-wait |
| `rutter-mcp` | MCP tool surface (rmcp) |
| `rutter-policy` | Rule set, verdicts, approval broker |
| `rutter-dashboard` | Local supervision dashboard (events, approvals) |
| `rutter` (cli) | Binary entry modes: browse, serve, open |

## Development

Requires a stable Rust toolchain (1.88 or newer).

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
cargo test -p rutter --test mcp_e2e -- --ignored
cargo test -p rutter --test approval_e2e -- --ignored
cargo test -p rutter-engine-cdp --test screencast -- --ignored
```

`scripts/benchmark.sh` runs the 20-site navigate+snapshot benchmark
(M2 gate: 90 percent success).

`scripts/smoke_open.sh` runs the M0 acceptance corpus (10 real sites)
and prints a pass/fail summary.

## Policies

- Commits follow Conventional Commits, English, imperative mood.
- License: Apache-2.0 (the documented default of open decision OD-3 in
  the blueprint; see `LICENSE`).
