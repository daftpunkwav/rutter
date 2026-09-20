# rutter Architecture

English | [中文](architecture.zh.md)

The reading-order entry point: what exists, how the pieces talk, and
which boundaries hold. Start here; every section links to the document
that owns the details.

rutter is a single-binary, headless browser orchestration service for
AI agents, written in Rust. It manages a real browser engine as a
supervised child process, exposes pages as token-budgeted
accessibility-tree snapshots instead of pixels, executes typed actions
with deterministic three-phase auto-wait semantics, and supervises
agents through a localhost dashboard with human approval gates.

Out of scope by design: rutter implements no rendering engine of its
own (the engine is an external process behind a trait), no
stealth or anti-fingerprinting behavior, no consumer browser UI, and
no cloud service.

## The workspace

Ten crates, one dependency direction: arrows point downward only. The
deeper rule is layering — a crate never imports anything from a crate
below another crate it appears under. Rust forbids cycles by
construction; the table is the layering contract.

| Crate | Depends on | Sole responsibility |
|---|---|---|
| `rutter-core` | serde | Domain vocabulary: `Action`, `Snapshot`, `Reference`, errors |
| `rutter-engine` | core | `Engine`/`Page` traits, supervisor, engine downloader |
| `rutter-engine-cdp` | engine, core | CDP implementation details (chromiumoxide) — nothing else |
| `rutter-observe` | core | Injected serializer script + pure DOM→`Snapshot` builder |
| `rutter-policy` | core | Rule set, verdict evaluation, approval broker |
| `rutter-events` | core, policy | Event types, broadcast bus, ring buffers, replay |
| `rutter-session` | core, events, engine, observe, policy | Orchestration: execution, contexts, storage state, recovery |
| `rutter-mcp` | core, session | MCP host (rmcp) and the tool surface |
| `rutter-dashboard` | core, events, policy, session | HTTP/WS server, embedded frontend, decision submission |
| `rutter` (cli) | all of the above except `events` (transitive) | Binary entry modes, composition root, config loading |

The seams and why they exist:

- **`core` knows nothing above it.** It is pure vocabulary; changing an
  engine or a transport never touches it.
- **`events` touches `policy` for one field.** An approval event carries
  the brief policy built, because only policy knows the class, the
  canonical URL it judged, and the rule that spoke. Nothing else in the
  backbone speaks policy.
- **`engine-cdp` is the only crate permitted to speak CDP.** Swapping
  or adding engines is confined to one crate plus a registration point
  in the CLI ([`EngineLauncher`](../crates/engine/src/supervisor/mod.rs)).
- **`observe` is pure data transformation** (`serde_json::Value` in,
  `Snapshot` out) plus the JS assets it owns. No async code; unit-testable
  without an engine.
- **`policy` is pure computation** (`RuleSet` + class × URL →
  `Verdict`) plus the approval broker's parked-future bookkeeping.
  No I/O.
- **`session` composes the above** — the only place where engine,
  observation, policy, and events meet.
- **`dashboard` never executes actions.** It reads events and submits
  policy decisions; supervision is observation plus verdicts.

Per-directory `README.md` files state each module's responsibility,
boundary, and file map, starting at
[`crates/README.md`](../crates/README.md).

## Runtime shape

```
Agent (any MCP client)
  │  MCP (stdio / streamable HTTP)
  ▼
┌─ rutter (single Rust process) ─────────────────────────────┐
│  cli          argument parsing, config, process lifecycle  │
│  mcp          tool surface (rmcp)                          │
│  dashboard    axum server + embedded frontend (localhost)  │
│  session      orchestration: actions, contexts, recovery   │
│  policy       verdicts + approval broker (pure, no I/O)    │
│  observe      snapshot builder + injected serializer (pure)│
│  events       typed event backbone + ring buffer + replay  │
│  engine       Engine/Page traits, supervisor, downloader   │
│  engine-cdp   the only crate that knows CDP (chromiumoxide)│
│  core         shared domain vocabulary (types, errors)     │
└─────────────────────────────────────────────────────────────┘
  │ CDP over WebSocket (localhost)          ▲ events + frames
  ▼                                        │
chrome-headless-shell (child process)   Human browser (dashboard viewer)
```

- One engine child process per rutter process; one browser context per
  MCP session; pages capped per context.
- The engine starts lazily on the first session request — `rutter
  serve` is MCP-ready before any engine I/O — and is supervised:
  heartbeat probes, capped-backoff restarts, a sliding-window circuit
  breaker ([engine supervision](engine-supervision.md)).
- A restart triggers storage-state replay and page restoration, and
  every session learns about it through an `EngineRestarted` event
  ([sessions](sessions.md)).

### Entry modes

| Invocation | Behavior |
|---|---|
| `rutter` | Browse mode: headed engine window for direct human operation |
| `rutter serve` | MCP server; engine headless by default, `--headed` overrides; `--dashboard PORT` and `--policy FILE` attach the dashboard and rule set |
| `rutter open` | One-shot diagnostic: navigate, print a snapshot, exit |

No-argument launch maps to browse mode, so launching the binary yields
a usable, human-operated session without any MCP client. Engine mode
and dashboard are orthogonal flags; entry modes only set defaults.
Flags and environment variables are documented in the
[README](../README.md#usage).

## Cross-cutting invariants

These hold everywhere in the codebase; subsystem documents rely on
them without restating them.

- **Every wait is bounded.** Every I/O operation carries a deadline:
  CDP commands 30 s, navigation 30 s, each auto-wait phase 5 s,
  polling budgets clamped to 600 s. No unbounded waits exist.
- **A crashed engine is an error, never a crash.** While the engine is
  dead, restarting, or breaker-open, affected operations fail with
  `Terminated`; the heartbeat keeps retrying and the supervisor
  recovers on its own.
- **Errors are data.** Every action failure is one of the
  [error taxonomy](tool-catalog.md#2-result-conventions) variants,
  serializable, carrying an actionable English hint. `Internal`
  contains a bug, never a silent pass.
- **Hostile input degrades, never panics.** The snapshot pipeline
  treats arbitrary page data as untrusted: it truncates, folds, and
  marks unknown instead of failing. No `unwrap`/`expect`/`panic`
  outside tests and process init (clippy denies them).
- **Events never wait on consumers.** Publishing is fire-and-forget;
  semantic
  events survive for late joiners through bounded per-session rings,
  while screencast frames are droppable under load
  ([events](events.md)).
- **Deterministic failures fail without retrying.** Retries with
  capped exponential backoff exist for engine launch/connect; a
  circuit breaker stops restart storms.

Performance targets (design goals; no automated harness measures
them — `scripts/benchmark.sh` reports the navigate+snapshot success
rate only):

| Metric | Target |
|---|---|
| rutter startup → MCP ready (engine lazy) | < 100 ms |
| Snapshot round-trip, warm engine, p50 | < 150 ms |
| Orchestration-layer RSS (engine excluded) | < 50 MB |
| Concurrent contexts per engine process | capped by `max_sessions` (default 8) |
| Dashboard frame latency (page change → pixel) | < 500 ms |

## Document map

| Document | Role |
|---|---|
| [Glossary](glossary.md) | The normative domain vocabulary; one concept, one term |
| [Tool catalog](tool-catalog.md) | The MCP tool surface: transport, semantics, auto-wait, error mapping |
| [Snapshot format](snapshot-format.md) | The observation pipeline: serializer envelope, references, token budget |
| [Engine supervision](engine-supervision.md) | Engine trait, binary acquisition, supervisor, CDP notes |
| [Sessions](sessions.md) | Session model, action execution path, storage state, recovery |
| [Events](events.md) | Event vocabulary, backbone semantics, replay |
| [Policy](policy.md) | Action classes, verdict evaluation, fail-closed rules, approvals |
| [Dashboard](dashboard.md) | Server, access control, WebSocket protocol, screencast |
| [Testing](testing.md) | Test levels, what pins which contract, how to run the suites |
