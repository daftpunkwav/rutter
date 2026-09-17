# Architecture

> rutter 0.1.0 · This document mirrors the canonical reference in
> [`BLUEPRINT.md`](BLUEPRINT.md) at release time: what exists, how the
> pieces talk, and which boundaries hold. The blueprint stays
> normative; this file is the reading-order entry point.

## What rutter is

A single-binary, headless browser orchestration service for AI agents.
rutter manages a real browser engine (Chrome for Testing) as a
supervised child process, exposes pages as token-budgeted
accessibility-tree snapshots instead of pixels, executes typed actions
with deterministic three-phase auto-wait semantics, and supervises
agents through a localhost dashboard with human approval gates.

## The layer cake

Crates depend downward only (blueprint §5 is normative):

```
rutter (cli)          entry modes: browse, serve (stdio/HTTP), open
rutter-mcp            MCP tool surface (rmcp): stdio + streamable HTTP
rutter-dashboard      localhost web UI: live view, events, approvals
rutter-session        orchestration: sessions, pages, auto-wait,
                      policy enforcement, storage state, recovery
rutter-policy         verdict evaluation + approval broker (pure)
rutter-engine         traits: Engine/Context/Page; downloader; supervisor
rutter-events         typed event backbone: bus, ring buffers, replay
rutter-observe        in-page scripts (serializer, resolver) + snapshot
                      builder (pure)
rutter-engine-cdp     the only crate that speaks CDP (chromiumoxide)
rutter-core           shared vocabulary: Action, Snapshot, Reference,
                      errors (knows nothing above it)
```

Key seams and why they exist:

- **`rutter-core` knows nothing above it.** Changing an engine or a
  transport never touches the vocabulary.
- **`rutter-engine-cdp` is the only CDP speaker.** Swapping engines is
  confined there plus the CLI registration point.
- **`rutter-observe` owns the in-page scripts** (serializer, reference
  resolver, storage dump/restore). It is pure, has no async code, and
  is unit-testable without an engine.
- **`rutter-policy` is pure computation** (`class + URL -> verdict`)
  plus the approval broker; the dashboard submits decisions through it,
  and it never sees I/O.
- **`rutter-session` is where everything meets**: actions, snapshots,
  events, policy, storage, recovery. One reason to change per crate.
- **The dashboard never executes actions** — observation plus verdict
  submission only.

## Runtime shape

```
MCP client ──stdio / streamable HTTP──► rutter serve
                                          │  sessions (policy gate,
                                          │  storage, auto-wait)
                                          ▼
                                    rutter-engine (supervisor:
                                    heartbeat, backoff, breaker)
                                          │ CDP over WebSocket
                                          ▼
                                 chrome-headless-shell (child)

human ──browser──► dashboard (127.0.0.1) ──decisions──► broker
```

- One engine child process per rutter process; one browser context per
  MCP session; pages capped per context.
- The engine starts lazily on the first session (startup < 100 ms) and
  is supervised: heartbeat probes, capped-backoff restarts, a
  sliding-window circuit breaker. A restart triggers storage-state
  replay and page restoration, and every session learns about it via an
  `EngineRestarted` event (§7.4).
- Storage state (cookies + localStorage) persists per session on every
  change and survives restarts and process restarts (§7.4).

## Contracts

- `SNAPSHOT_SPEC.md` — the serializer envelope, reference minting, the
  YAML text form, and the token budget. Pinned by golden and property
  tests.
- `TOOL_SPEC.md` — the MCP tool surface, auto-wait semantics, and the
  error mapping (action failures are results with hints, not protocol
  errors).

## Quality gates

CI runs on every push/PR: fmt, clippy `-D warnings`, the full test
suite, the header gate (every source file opens with a truthful
header), and the encoding gate (tracked text files stay English-only;
CJK is reserved for future fixtures under an explicit allowlist,
open decision OD-1). Integration and acceptance suites (engine, MCP
e2e, approval flows, screencast, HTTP transport) drive the real engine
and are `#[ignore]`d locally, run by the CI integration job with a
cached engine. Release archives are cut by cargo-dist on tags
(`release.yml`).
