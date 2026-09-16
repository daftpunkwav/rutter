# rutter

A single-binary, headless browser orchestration service for AI agents,
written in Rust. rutter manages a real browser engine as a supervised
child process, exposes a structured view of pages (accessibility-tree
snapshots) instead of pixels, executes typed actions with deterministic
semantics, and offers a local supervision dashboard with human approval
for sensitive operations.

Status: pre-release skeleton under active development. The canonical
architecture and engineering reference is [`docs/BLUEPRINT.md`](docs/BLUEPRINT.md).

## Repository layout

```
rutter/
├── docs/         # blueprint and specifications
├── scripts/      # quality-gate helpers run by CI
├── crates/       # workspace members (see below)
└── frontend/     # dashboard sources (added with milestone M2)
```

Crates that exist today, each with a single responsibility:

| Crate          | Responsibility                                    |
|----------------|---------------------------------------------------|
| `rutter-core`  | Shared domain vocabulary: actions, snapshots, references, errors |
| `rutter-engine`| Engine and page traits, launch and health types   |
| `rutter-observe`| Pure DOM-JSON to Snapshot conversion             |
| `rutter` (cli) | Binary entry modes: browse, serve, open           |

Further crates (`rutter-engine-cdp`, `rutter-events`, `rutter-policy`,
`rutter-session`, `rutter-mcp`, `rutter-dashboard`) materialize with the
milestone that needs them; empty crates are forbidden by the blueprint
(§10).

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

## Policies

- English is the project's only working language; CI enforces this on
  all tracked text files.
- Commits follow Conventional Commits, English, imperative mood.
- License: Apache-2.0 (the documented default of open decision OD-3 in
  the blueprint; see `LICENSE`).
