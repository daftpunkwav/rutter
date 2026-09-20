# rutter

English | [中文](README.zh.md)

A single-binary, headless browser orchestration service for AI agents,
written in Rust. rutter manages a real browser engine as a supervised
child process, exposes a structured view of pages (accessibility-tree
snapshots) instead of pixels, executes typed actions with deterministic
semantics, and offers a local supervision dashboard with human approval
for sensitive operations.

## Documentation

| Document | Role |
|----------|------|
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Reading-order entry point: crates, runtime shape, contracts |
| [`docs/BLUEPRINT.md`](docs/BLUEPRINT.md) | Architecture blueprint: design rules and normative decisions (§-referenced from code) |
| [`docs/TOOL_SPEC.md`](docs/TOOL_SPEC.md) | The MCP tool surface: parameters, semantics, error mapping |
| [`docs/SNAPSHOT_SPEC.md`](docs/SNAPSHOT_SPEC.md) | The snapshot format: serialization, references, token budget |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | Working rules and local gates |

Every source directory carries a `README.md` stating its
responsibility, boundary, and file map — start at
[`crates/README.md`](crates/README.md). Each README has a Chinese
mirror named `README.zh.md`; the two are updated together.

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

Serve flags (accepted on every mode, effective on `rutter serve` only):

| Flag | Meaning |
|------|---------|
| `--http <ADDR>` | Serve MCP over streamable HTTP on that address instead of stdio |
| `--dashboard <PORT>` | Attach the supervision dashboard on 127.0.0.1:`<PORT>` |
| `--policy <FILE>` | Load the supervision rule set from a TOML file |
| `--allow-remote` | Confirm a non-loopback `--http` bind (the transport has no authentication) |

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

### Policy and approvals

Actions are classified (navigation, pointer, keyboard, selection,
scroll, cookies) and judged against a rule set of `class × URL pattern
→ verdict` loaded from a TOML file. Verdicts are `allow`, `deny`, and
`require_approval`; navigations are judged on their canonicalized
target URL, and an unreadable page fails closed to a human decision.
Cookies require approval by default.

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

## Repository layout

```
rutter/
├── docs/         # blueprint, architecture, tool and snapshot specs
├── scripts/      # quality-gate helpers run by CI
├── crates/       # workspace members (see below)
├── tests/        # cross-crate acceptance tests driving the binary
└── frontend/     # dashboard sources (vanilla JS, no build step)
```

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
bash scripts/check_encoding.sh  # code and comments stay English-only
```

Engine integration tests need a real binary and are `#[ignore]`d by
default; run them explicitly (they reuse the cache `rutter open`
fills):

```sh
cargo test -p rutter-engine-cdp --test integration -- --ignored
cargo test -p rutter-integration-tests --test mcp_e2e -- --ignored
cargo test -p rutter-integration-tests --test approval_e2e -- --ignored
cargo test -p rutter-integration-tests --test http_e2e -- --ignored
cargo test -p rutter-engine-cdp --test screencast -- --ignored
```

`scripts/smoke_open.sh` drives 10 real sites through `rutter open` and
prints a pass/fail summary; `scripts/benchmark.sh` runs the 20-site
navigate+snapshot benchmark with a 90 % success bar.

## Policies

- Commits follow Conventional Commits, English, imperative mood.
- Code, comments, and commit messages are English-only (enforced by
  the encoding gate); documentation ships as English `README.md` with
  Chinese `README.zh.md` mirrors.
- License: Apache-2.0 (see [`LICENSE`](LICENSE)).
