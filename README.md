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
| [`docs/architecture.md`](docs/architecture.md) | Reading-order entry point: crates, runtime shape, invariants |
| [`docs/glossary.md`](docs/glossary.md) | The normative domain vocabulary |
| [`docs/tool-catalog.md`](docs/tool-catalog.md) | The MCP tool surface: transport, semantics, auto-wait, error mapping |
| [`docs/snapshot-format.md`](docs/snapshot-format.md) | The snapshot contract: serialization, references, token budget |
| [`docs/read-format.md`](docs/read-format.md) | The read contract: markdown extraction, guards |
| [`docs/engine-supervision.md`](docs/engine-supervision.md) | Engine trait, binary acquisition, supervisor, CDP notes |
| [`docs/sessions.md`](docs/sessions.md) | Session model, action path, storage state, recovery |
| [`docs/events.md`](docs/events.md) | Event vocabulary, backbone semantics, replay |
| [`docs/policy.md`](docs/policy.md) | Action classes, verdicts, fail-closed rules, approvals |
| [`docs/dashboard.md`](docs/dashboard.md) | Dashboard server, access control, WebSocket protocol |
| [`docs/testing.md`](docs/testing.md) | Test levels, contract pins, how to run |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | Working rules and local gates |

Every source directory carries a `README.md` stating its
responsibility, boundary, and file map (the five shared test-helper
directories — `tests/common/`, `crates/engine-cdp/tests/common/`,
`crates/session/tests/common/`, `crates/dashboard/tests/common/`, and
`crates/mcp/tests/common/` — are covered by their parent test READMEs)
— start at
[`crates/README.md`](crates/README.md). Each README and each
`docs/` document has a Chinese mirror: `README.zh.md` beside its
`README.md`, and `<name>.zh.md` beside `<name>.md` under `docs/`;
the two languages are updated together.

## Usage

```sh
# One-shot diagnostic: navigate and print a YAML snapshot to stdout.
rutter open https://example.com

# One-shot scrape: navigate and print the page as markdown.
rutter read https://example.com

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
(x64) are cut by cargo-dist (`cargo dist build`, configured in the
`[workspace.metadata.dist]` section of the root [`Cargo.toml`](Cargo.toml)). The
browser engine itself is not bundled — rutter downloads Chrome for
Testing into its cache on first use (or point `--engine-executable`
at an existing binary).

## Repository layout

```
rutter/
├── docs/         # code-facing documentation (English + zh mirrors)
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
| `rutter` (cli) | Binary entry modes: browse, serve, open, read |

## Development

Requires a stable Rust toolchain (1.88 or newer).

```sh
cargo build
cargo test --all
cargo clippy --all-targets -- -D warnings
cargo doc --no-deps
cargo fmt --all -- --check
```

Quality gates, identical to CI:

```sh
bash scripts/check_headers.sh   # every source file opens with a header
bash scripts/check_encoding.sh  # tracked text files: UTF-8, LF, no BOM
```

CI also gates workspace line coverage: `scripts/check_coverage.sh`
checks the cargo-llvm-cov report against a threshold (90 % in CI,
engine suites and e2e suites included).

Engine integration tests need a real binary and are `#[ignore]`d by
default; run them explicitly (they reuse the cache `rutter open`
fills):

```sh
cargo test -p rutter-engine-cdp --tests -- --ignored
cargo test -p rutter-integration-tests --test mcp_e2e -- --ignored
cargo test -p rutter-integration-tests --test approval_e2e -- --ignored
cargo test -p rutter-integration-tests --test http_e2e -- --ignored
cargo test -p rutter-integration-tests --test open_e2e -- --ignored
cargo test -p rutter-integration-tests --test read_e2e -- --ignored
```

`scripts/smoke_open.sh` drives 10 real sites through `rutter open` and
prints a pass/fail summary; `scripts/benchmark.sh` runs the 20-site
navigate+snapshot benchmark with a 90 % success bar.

## Policies

- Commits follow Conventional Commits, English, imperative mood.
- Code comments may be written in Chinese or English; documentation
  ships as English `README.md` with Chinese `README.zh.md` mirrors,
  updated together.
- Security issues are reported privately — see
  [`SECURITY.md`](SECURITY.md).
- License: Apache-2.0 (see [`LICENSE`](LICENSE)).
