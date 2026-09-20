# rutter — Architecture Blueprint

> As-built reference: this document describes the implemented system and
> the contracts it commits to. Wire-level contracts live in
> [`TOOL_SPEC.md`](TOOL_SPEC.md) (MCP surface) and
> [`SNAPSHOT_SPEC.md`](SNAPSHOT_SPEC.md) (snapshot format); the
> reading-order entry point is [`ARCHITECTURE.md`](ARCHITECTURE.md).

rutter is a single-binary, headless browser orchestration service for AI
agents, written in Rust. English is the working language of all code,
comments, and commit messages; documentation is written in English and
mirrored in Chinese (`*.zh.md`, see §8.6).

---

## 1. Purpose

rutter gives AI agents a fast, reliable, observable, and safe way to
browse and act on the web. It manages a real browser engine as a
supervised child process, exposes a structured view of pages
(accessibility-tree snapshots) instead of pixels, executes typed actions
with deterministic semantics, and lets a human supervise and approve
sensitive operations through a local dashboard.

The value of rutter is everything **above** the engine: observation
quality, action reliability, session resilience, and protocol
ergonomics. The engine itself is a replaceable dependency.

## 2. Non-Goals

- **No rendering engine of our own.** HTML/CSS/JS is never implemented
  here; the engine is an external process behind a trait.
- **No stealth / anti-fingerprinting.** Documented limitation; engine
  flags (including a proxy) can be passed through with `--engine-arg`.
- **No consumer browser UI** (no address bar, tab chrome, bookmarks).
- **No cloud service.** rutter is self-hosted, local-first software.

Non-goals are enforced at review time: a pull request that expands scope
is rejected by default.

## 3. Interfaces and Entry Modes

1. **Primary interface: MCP** (Model Context Protocol). `rutter serve`
   speaks MCP over stdio and over streamable HTTP; MCP clients consume
   it directly.
2. **Supervision dashboard.** A local web page served by the same
   binary shows live agent activity: page screencast, the snapshot
   tree, an action timeline, and human-approval prompts for sensitive
   actions.
3. **Engine pluggability.** Chromium headless shell (via CDP) is the
   shipped backend. Any engine that implements the launcher trait can
   be added behind the same boundary.
4. **CLI-first debugging.** Every core capability is reachable without
   MCP: `rutter open <url>` prints a snapshot. The CLI is the first
   consumer of the core crates and proves they stand alone.
5. **Standalone operation.** A no-argument launch starts browse mode —
   a headed engine window operated by a human. Agent attachment is an
   addition, never a prerequisite.

## 4. Architecture Overview

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

### Process model

- One engine child process per rutter process. One browser **Context**
  per MCP session; Pages (tabs) live inside a Context with a
  configurable cap.
- The engine is **supervised**: a 10 s heartbeat health probe; on death
  it restarts with capped exponential backoff behind a sliding-window
  circuit breaker, and session state (cookies, storage, page list) is
  restored from the persisted snapshot of that state.
- The engine binary is resolved on first use — explicit override,
  cache, then a Chrome for Testing download into the cache directory,
  pinned by version. Offline reuse of the cached engine always works.
- The orchestration layer survives every engine failure: a crashed
  engine is a `Terminated` error on affected operations, never a crash
  of rutter.

## 5. Workspace Layout and Dependency Rules

```
rutter/
├── Cargo.toml                # workspace manifest
├── deny.toml                 # cargo-deny: advisories, licenses, bans
├── rustfmt.toml
├── .github/workflows/        # ci.yml, release.yml (cargo-dist)
├── scripts/                  # quality-gate helpers run by CI
├── docs/                     # this blueprint, tool spec, snapshot spec
├── crates/                   # the ten workspace members
└── frontend/                 # no-build vanilla JS dashboard sources
```

### Dependency DAG (normative)

Arrows point downward only. Rust forbids cyclic crate dependencies by
construction; the deeper rule is **layering**: a crate may never import
anything from a crate listed below it in this table.

| Crate            | Depends on                              | Sole responsibility                                        |
|------------------|-----------------------------------------|------------------------------------------------------------|
| rutter-core      | serde                                   | Domain vocabulary: Action, Snapshot, Reference, errors      |
| rutter-events    | core                                    | Event types, broadcast bus, ring buffer, replay             |
| rutter-engine    | core                                    | Engine/Page traits, supervisor, engine downloader           |
| rutter-engine-cdp| engine, core                            | CDP implementation details (chromiumoxide) — nothing else   |
| rutter-observe   | core                                    | Injected serializer script + pure DOM→Snapshot builder      |
| rutter-policy    | core                                    | Rule set, verdict evaluation, approval broker               |
| rutter-session   | core, events, engine, observe, policy   | Orchestration: execution, contexts, storage state, recovery |
| rutter-mcp       | core, session                           | MCP host (rmcp) and the tool surface                       |
| rutter-dashboard | core, events, session (read-only), policy | HTTP/WS server, embedded frontend, decision submission    |
| rutter (cli)     | core, engine, engine-cdp, mcp, dashboard, observe, policy, session | Binary entry, composition root, config loading |

Key seams and why they exist:

- **core knows nothing above it.** It is pure vocabulary; changing an
  engine or a protocol never touches it.
- **engine-cdp is the only crate permitted to speak CDP.** Swapping or
  adding engines is confined to one crate plus a registration point in
  the CLI.
- **observe is pure data transformation** (`serde_json::Value` in,
  `Snapshot` out) plus the JS assets it owns. It has no async code and
  is unit-testable without any engine.
- **policy is pure computation** (`RuleSet + action class × URL →
  Verdict`) plus the approval broker state machine. No I/O, fully
  testable.
- **session composes the above**; it is the only place where engine,
  observation, policy, and events meet. High cohesion by design: one
  reason to change per crate.
- **dashboard never executes actions.** It reads events and submits
  policy decisions. Supervision is observation plus verdicts, nothing
  more.

## 6. Domain Vocabulary (normative glossary)

One concept, one term, everywhere. Code, docs, and UI must use these
exact terms.

| Term        | Meaning                                                        |
|-------------|----------------------------------------------------------------|
| Engine      | A browser process managed by rutter (Chromium headless shell)  |
| Context     | Isolated cookie/storage unit inside an engine                  |
| Page        | One tab/target inside a context                                |
| Session     | One MCP client's workspace: its context, pages, and state      |
| Snapshot    | Token-budgeted accessibility-tree view of a page, with refs    |
| Reference   | Stable handle to an element, usable across snapshots           |
| Action      | Typed operation requested by an agent (click, type, …)         |
| Verdict     | Policy decision: Allow / Deny / RequireApproval                |
| Event       | Structured fact published on the event backbone                |
| Origin      | Attribution of an action's initiator: Agent or Human           |
| Browse mode | Entry mode: headed engine session operated directly by a human |

## 7. Core Design

### 7.1 Engine abstraction

```rust
// rutter-engine — the only engine abstraction in the codebase.
#[async_trait::async_trait]
pub trait Engine: Send + Sync {
    fn descriptor(&self) -> EngineDescriptor;          // id, version, capabilities
    async fn create_context(&self, cfg: ContextConfig) -> Result<ContextHandle>;
    async fn health(&self) -> Result<HealthReport>;
    async fn shutdown(&self) -> Result<()>;
}

// Page-scoped operations live on PageHandle: navigate, evaluate,
// dispatch_input, capture_screenshot, start_screencast, …
```

Engine launch supports two modes: `Headless` (default in serve mode) and
`Headed` (a visible engine window, used by browse mode and interactive
debugging). The mode is engine-level configuration and does not change
any API above the trait.

The supervisor owns process lifecycle: launch with capped exponential
backoff, a 10 s heartbeat probe, restarts behind a sliding-window
circuit breaker (too many restarts inside the window opens the breaker
and fails callers with a clear error until it drains), and clean
shutdown. Every CDP command runs under a deadline, so a wedged browser
degrades to errors instead of hanging the engine or its supervision.

### 7.2 Observation layer

- **Format:** Playwright-compatible YAML-style accessibility tree,
  `- button "Sign in" [ref=e17]`. Agents already read this format; refs
  are minted by the page serializer and stay stable across snapshots of
  the same page (`docs/SNAPSHOT_SPEC.md` §4).
- **Mechanics:** a serializer script owned by `rutter-observe` (embedded
  JS asset) is evaluated in the page; it walks the composed DOM
  including shadow roots and reports a plain JSON tree. `observe::build`
  converts that JSON into a `Snapshot` (pure function).
- **Token budget:** viewport-first culling, similar-item folding
  (`list item × 20`), depth and size caps, opt-in full-page mode.
- **Honesty rule:** hidden elements are omitted, never guessed at.

### 7.3 Action model and error taxonomy

Every mutating action runs a three-phase auto-wait (element visible,
stable, enabled → act → settle) and returns a fresh snapshot by
default. Each phase has its own budget that restarts only when the
phase advances, so a flickering page cannot stretch the wait
indefinitely.

```rust
// rutter-core — the error surface agents must be able to reason about.
pub enum ActionError {
    NavigationFailed  { url: String, cause: TransportCause },
    ReferenceExpired  { reference: Reference },
    NotInteractable   { reference: Reference, reason: String },
    TimedOut          { phase: WaitPhase, elapsed: Duration },
    ApprovalDenied    { reference: Reference },
    ApprovalTimedOut  { waited: Duration },
    EngineTerminated  { session: SessionId },
    Internal          { detail: String },   // bug containment, never silent
}
```

Every action carries an origin:

```rust
// rutter-core — who initiated an action.
pub enum Origin { Agent, Human }
```

### 7.4 Session and recovery

- Storage state (cookies, localStorage) is persisted per session on
  change — atomically (temp file + rename, owner-only on Unix) — and
  can be saved/loaded explicitly; this is how login state survives
  restarts.
- Recovery = supervisor restart + storage-state replay + page-list
  restoration, followed by an `EngineRestarted` event so the agent
  knows time passed.
- Session event history lives in a bounded per-session ring; closing a
  session drops its ring.

### 7.5 Events

- Publish is fire-and-forget and must never block action execution.
- Backpressure policy: screencast frames are droppable; semantic events
  are never dropped. Frames are handed off to the dashboard without
  awaiting, so a stalled viewer loses frames, not memory.
- The event bus bounds slow-subscriber loss (capacity + `Lagged`
  resync through replay); per-session rings are bounded and replay in
  global sequence order.

### 7.6 Policy and approval

```rust
// rutter-policy — pure verdict evaluation.
pub enum Verdict { Allow, Deny, RequireApproval }
impl RuleSet {
    pub fn evaluate(&self, class: ActionClass, url: &str) -> Verdict;
    pub fn evaluate_without_url(&self, class: ActionClass) -> Verdict;
}
```

- Rules come from a TOML config: action class × URL pattern → verdict.
  Defaults are conservative for destructive classes.
- The judgment URL is where the action leads, not where it comes from:
  `Navigate` is judged on its canonicalized target URL, every other
  class on the canonicalized current page URL. A URL the session
  cannot establish — an unreadable page, or a target that is not a URL
  (including `user@host` credential tricks) — fails closed: class
  rules still apply and a bare allow upgrades to `RequireApproval`.
- The session publishes `ApprovalRequested` and parks the action on the
  approval broker (configurable timeout, default 120 s), which delivers
  the decision submitted through the policy API (used by the dashboard).
  Denial and timeout map to `ApprovalDenied` / `ApprovalTimedOut`.
- Dangerousness is decided by rutter's rules, never by the agent's
  self-declaration — that is the point of supervision.
- Approval applies to Agent-origin actions only (§7.3). Human-origin
  actions bypass approval and are recorded on the same timeline.

### 7.7 Dashboard

- Three panes: live view (screencast), snapshot tree, action timeline;
  approval modal linked to the referenced element; multi-session
  switcher.
- Screencast via `Page.startScreencast`: JPEG, ~1–5 fps, width ≤ 1024,
  a `screencastFrameAck` loop per frame, restarted after navigations
  (CDP stops screencasts on navigation — handled). **On-demand only:**
  streaming starts when a viewer subscribes and stops when the last
  viewer leaves; frames are droppable under load (§7.5).
- WS protocol: text frames carry JSON events (same envelope as the
  bus), binary frames carry JPEG images. Client messages: subscribe,
  decision, screencast on/off. Approval decisions also travel as
  `POST /api/decisions` with the same token gate (the dashboard's HTTP
  channel, kept for simple automation clients).
- Reconnect: the client replays history on connect and resyncs from
  the event rings after a broadcast lag, deduplicating by sequence
  watermark.
- Security: binds `127.0.0.1` only; per-launch random token printed to
  the terminal and exchanged for an HttpOnly, SameSite=Strict cookie on
  first connect; `Host` header validation against DNS rebinding.
- Frontend: vanilla JS, no build step, embedded into the binary at
  compile time (`include_str!`). All UI strings come from the
  `frontend/i18n/en.json` catalog keys.

### 7.8 MCP tool surface

Eighteen tools, named after the verbs agents already know:

`navigate, back, forward, reload, snapshot, screenshot, click, hover,
type, press_key, select_option, scroll, wait_for, tabs_list,
tabs_select, tabs_close, set_cookies, close_session`

Tool schema, semantics, and error mappings are specified in
`docs/TOOL_SPEC.md`; the spec is the contract. Transport hardening: the
streamable HTTP transport validates `Origin` (browser-originated
requests are rejected) and refuses a non-loopback bind unless
confirmed with `--allow-remote`; the `Host` header is validated against
a loopback allowlist (DNS rebinding).

### 7.9 Binary entry modes

| Invocation     | Behavior                                                     |
|----------------|--------------------------------------------------------------|
| `rutter`       | Browse mode: headed engine window for direct human operation |
| `rutter serve` | MCP server; engine headless by default, `--headed` overrides; `--dashboard PORT` and `--policy FILE` attach the supervision dashboard (§7.7) and rule set |
| `rutter open`  | One-shot diagnostic: navigate, print snapshot, exit          |

No-argument launch maps to browse mode so that launching the binary
yields a usable, human-operated session without any MCP client; the
dashboard is not part of browse mode — it attaches to `serve` only.
Engine mode and dashboard are orthogonal flags; entry modes only set
defaults.

## 8. Engineering Standards

### 8.1 Decoupling rules (the iron law)

- Crates communicate through their public APIs and `rutter-core` types
  only. No reaching into another crate's internals.
- One responsibility per crate, one responsibility per file, one
  purpose per symbol.
- No feature flags that cut across crate boundaries incoherently;
  engine selection is a registration, not a compile-time fork of the
  codebase.

### 8.2 Naming rules

- Name after responsibility, boundary, or behavior — never after
  implementation details, brands, or jokes. `SnapshotBuilder`, not
  `CdpTreeThing`.
- The brand `rutter` appears only in: repository name, binary name,
  workspace/crate names (`rutter-*`), and documentation titles. Never
  in identifiers.
- The glossary (§6) is normative. No synonyms for glossary terms in
  code or docs.
- No ad-hoc abbreviations; established ones only (URL, CDP, MCP, DOM).

### 8.3 File headers and single responsibility

Every source file (Rust, JS, shell, script) begins with a header
comment stating purpose and boundary. Rust uses inner doc comments:

```rust
//! Builds token-budgeted accessibility snapshots from serialized DOM trees.
//!
//! Boundary: pure data transformation only. DOM serialization (the injected
//! script), engine access, and action execution live in other crates.
```

Guidance: a Rust file beyond ~400 lines, or holding a second reason to
change, is split. Headers must stay truthful — an outdated header is a
defect.

### 8.4 Error handling and robustness

- `thiserror` enums per crate; error types are public contracts, and
  every error carries an actionable, English `hint`.
- No `unwrap`/`expect`/`panic` outside tests and process init (clippy
  denies). A panic is always a reported bug, never a silent pass.
- Every I/O operation and every CDP command has a timeout; no
  unbounded waits exist anywhere.
- Retries with capped exponential backoff for engine launch/connect;
  circuit breaker against restart storms. Deterministic failures (4xx,
  over-cap payloads) fail without retrying.
- Hostile-input discipline: the snapshot pipeline treats arbitrary page
  data as untrusted input and degrades (truncate, fold, mark unknown)
  rather than fail or panic.

### 8.5 Performance doctrine

Targets (the corpus harness in `scripts/benchmark.sh` measures the
click/snapshot side):

| Metric                                              | Target          |
|-----------------------------------------------------|-----------------|
| rutter startup → MCP ready (engine lazy)            | < 100 ms        |
| Snapshot round-trip, warm engine, p50               | < 150 ms        |
| Orchestration-layer RSS (engine excluded)           | < 50 MB         |
| Concurrent contexts per engine process              | ≥ 50 (capped)   |
| Dashboard frame latency (page change → pixel)       | < 500 ms        |

The engine is spawned lazily and only while at least one session
exists. Hot paths (ref resolution, snapshot parse, culling) avoid
gratuitous allocation; correctness and clarity outrank
micro-optimization everywhere else.

### 8.6 Language and content policy

- Code, comments, commit messages, issues, and PRs are English-only.
  CI enforces this with an encoding gate (`scripts/check_encoding.sh`):
  tracked text files must contain no CJK codepoints, except
  `**/*.zh.md` — the Chinese documentation mirrors (§8.7). Test
  fixtures that need non-Latin content require an explicit allowlist
  entry in the gate script.
- Dates/times are RFC 3339 UTC; no locale-dependent formatting
  anywhere.
- All strings UTF-8; dashboard layout must not assume left-to-right.

### 8.7 Documentation and repository hygiene

- Conventional Commits, English only; imperative mood.
- Every `README.md` is mirrored as `README.zh.md` in the same
  directory (same structure, same facts, Chinese prose; technical
  terms, identifiers, and flags stay in English). The two languages
  are updated together.
- `CHANGELOG.md` follows Keep a Changelog.
- `.gitattributes`: LF in repo, UTF-8, binary assets marked.
- License: Apache-2.0 (see `LICENSE`).

## 9. Testing and Quality Gates

| Level        | Scope                                                        | Tooling            |
|--------------|--------------------------------------------------------------|--------------------|
| Unit         | Pure crates (core, observe, policy, events)                  | plain `#[test]`    |
| Golden       | Snapshot builder output on fixture DOM trees                 | `insta`            |
| Property     | Culling/folding invariants; serializer never panics          | `proptest`         |
| Integration  | Real headless shell: launch, navigate, act, recover          | per-crate `tests/` |
| E2E          | Full MCP client → rutter → engine round-trip                 | workspace `tests/` package |
| Benchmark    | 20-site fixed corpus: click hit rate, snapshot tokens        | harness in `scripts/` |

Release gates: fmt clean, clippy `-D warnings` clean, all tests green,
encoding gate clean, header gate clean (every source file has a
header), `cargo-deny` clean, benchmark thresholds met (≥ 90 % click
hit rate on the corpus).
