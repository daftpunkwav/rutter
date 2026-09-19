# rutter — Project Blueprint

> Status: draft v1.1 · 2026-09-17 · Canonical architecture and engineering reference.
>
> rutter is a single-binary, headless browser orchestration service for AI
> agents, written in Rust. English is the project's only working language;
> every git-tracked file is English-only by policy (see §8.6 and §8.7).

---

## 1. Mission

Give AI agents a fast, reliable, observable, and safe way to browse and act
on the web. rutter manages a real browser engine as a supervised child
process, exposes a structured view of pages (snapshots) instead of pixels,
executes typed actions with deterministic semantics, and lets a human
supervise and approve sensitive operations through a local dashboard.

The value of rutter is everything **above** the engine: observation quality,
action reliability, session resilience, and protocol ergonomics. The engine
itself is a replaceable dependency.

## 2. Non-Goals

- **No rendering engine of our own.** We never implement HTML/CSS/JS. The
  engine is an external process behind a trait.
- **No stealth / anti-fingerprinting** in the first releases. Documented
  limitation; proxy pass-through is supported.
- **No consumer browser UI** (no address bar, tabs chrome, bookmarks).
- **No cloud service.** rutter is self-hosted, local-first software.

Non-goals are enforced at review time: a pull request that expands scope is
rejected by default.

## 3. Product Direction

1. **Primary interface: MCP** (Model Context Protocol). rutter ships as an
   MCP server (stdio first, streamable HTTP later). Agents such as coding
   assistants consume it directly.
2. **Supervision dashboard.** A local web page (served by rutter, embedded
   in the binary) showing live agent activity: screencast of the page, the
   snapshot tree, an action timeline, and human-approval prompts for
   sensitive actions.
3. **Engine pluggability.** Chromium headless shell today. Any
   CDP-compatible engine (e.g. Lightpanda) can be added later behind the
   same trait, selected per context.
4. **CLI-first debugging.** Every core capability is reachable without MCP:
   `rutter open <url>` prints a snapshot. The CLI is the first consumer of
   the core crates and proves they stand alone.
5. **Standalone operation.** The binary runs without any MCP client: a
   no-argument launch starts browse mode — a headed engine window operated
   by a human, with the dashboard available (§7.9). Agent attachment is an
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

- One engine child process per rutter process (initially). One browser
  **Context** per MCP session; Pages (tabs) live inside a Context with a
  configurable cap.
- The engine is **supervised**: heartbeat + `Browser.getVersion` health
  probes; on death it is restarted and session state (cookies, storage,
  page list) is restored from persisted snapshots of that state.
- The engine binary is downloaded on first run from the Chrome for Testing
  manifest into a cache directory and pinned by version. Offline reuse of
  the cached engine is always allowed.
- The orchestration layer must survive every engine failure. A crashed
  engine is an `EngineTerminated` error on affected sessions, never a
  crash of rutter.

## 5. Workspace Layout and Dependency Rules

```
rutter/
├── Cargo.toml                # workspace manifest
├── deny.toml                 # cargo-deny: advisories, licenses, bans
├── rustfmt.toml
├── .github/workflows/ci.yml
├── scripts/                  # CI helpers (encoding gate, header gate)
├── docs/                     # this blueprint, tool spec, snapshot spec
├── crates/
│   ├── core/                 # rutter-core
│   ├── events/               # rutter-events
│   ├── engine/               # rutter-engine
│   ├── engine-cdp/           # rutter-engine-cdp
│   ├── observe/              # rutter-observe
│   ├── policy/               # rutter-policy
│   ├── session/              # rutter-session
│   ├── mcp/                  # rutter-mcp
│   ├── dashboard/            # rutter-dashboard
│   └── cli/                  # rutter (binary)
└── frontend/                 # no-build vanilla JS dashboard sources
    ├── src/
    └── i18n/en.json          # string catalog (default locale)
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
| rutter-mcp       | core, session, events                   | MCP host (rmcp) and the tool surface                       |
| rutter-dashboard | core, events, session (read-only), policy | HTTP/WS server, embedded frontend, decision submission    |
| rutter (cli)     | mcp, dashboard, session                 | Binary entry, config loading, CLI subcommands               |

Key seams and why they exist:

- **core knows nothing above it.** It is pure vocabulary; changing an
  engine or a protocol never touches it.
- **engine-cdp is the only crate permitted to speak CDP.** Swapping or
  adding engines (cdpkit, a custom client, Lightpanda) is confined to one
  crate plus a registration point in the CLI.
- **observe is pure data transformation** (`serde_json::Value` in,
  `Snapshot` out) plus a JS asset it owns. It is unit-testable without any
  engine and has no async code.
- **policy is pure computation** (`RuleSet + action class × URL → Verdict`)
  plus the approval broker state machine. No I/O, fully testable.
- **session composes the above**; it is the only place where engine,
  observation, policy, and events meet. High cohesion by design: one
  reason to change per crate.
- **dashboard never executes actions.** It reads events and submits
  policy decisions. Supervision is observation plus verdicts, nothing more.

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

## 7. Core Design Decisions

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
debugging). The mode is engine-level configuration and does not change any
API above the trait.

The supervisor owns process lifecycle, restart with capped exponential
backoff, and a circuit breaker (max restarts per time window → fail the
affected sessions with a clear error instead of thrashing).

### 7.2 Observation layer

- **Format:** Playwright-compatible YAML-style accessibility tree,
  `- button "Sign in" [ref=e17]`. Agents already read this format; refs
  are minted by the page serializer and stay stable across snapshots of
  the same page (`docs/SNAPSHOT_SPEC.md` §4).
- **Mechanics:** a serializer script owned by `rutter-observe` (embedded
  JS asset) is evaluated in the page; it walks the composed DOM including
  shadow roots and reports a plain JSON tree. `observe::build` converts
  that JSON into a `Snapshot` (pure function).
- **Token budget:** viewport-first culling, similar-item folding
  (`list item × 20`), depth and size caps, opt-in full-page mode.
- **Honesty rule:** hidden elements are omitted, never guessed at.

### 7.3 Action model and error taxonomy

Every mutating action runs a three-phase auto-wait (element visible,
stable, enabled → act → settle) and returns a fresh snapshot by default.

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

All origins share a single execution path and event timeline. Origin
determines attribution and policy applicability (§7.6): approval rules
apply to Agent-origin actions; Human-origin actions (CLI diagnostics,
dashboard manual control per OD-2) bypass approval and are recorded
identically.

Every error carries an actionable, English, agent-readable hint. Errors
are data, not logs.

### 7.4 Session and recovery

- Storage state (cookies, localStorage) is persisted per session on change
  and can be saved/loaded explicitly — this is how login state survives
  restarts.
- Recovery = supervisor restart + storage-state replay + page-list
  restoration, then a `EngineRestarted` event so the agent knows time
  passed.
- Per-context resource caps: max pages, navigation timeout, screenshot
  rate.

### 7.5 Event backbone

Typed events (serde) on a tokio broadcast bus with a per-session ring
buffer for replay. The dashboard backfills history on connect from the
ring buffer. Publish is fire-and-forget and must never block action
execution. Backpressure policy: screencast frames are droppable
(latest-wins); semantic events are never dropped.

### 7.6 Policy and approval

```rust
// rutter-policy — pure verdict evaluation.
pub enum Verdict { Allow, Deny, RequireApproval }
impl RuleSet {
    pub fn evaluate(&self, class: ActionClass, url: &str) -> Verdict;
    pub fn evaluate_action(&self, action: &Action, url: &str) -> Verdict;
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
- The approval broker parks the action (configurable timeout, default
  120 s), publishes `ApprovalRequested`, and awaits a decision submitted
  through the policy API (used by the dashboard). Denial and timeout map
  to `ApprovalDenied` / `ApprovalTimedOut`.
- Dangerousness is decided by rutter's rules, never by the agent's
  self-declaration — that is the point of supervision.
- Approval applies to Agent-origin actions only (§7.3). Human-origin
  actions bypass approval and are recorded on the same timeline.

### 7.7 Dashboard

- Three panes: live view (screencast), snapshot tree, action timeline;
  approval modal linked to the referenced element; multi-session switcher.
- Screencast via `Page.startScreencast`: JPEG, ~1–5 fps, width ≤ 1024,
  correct `screencastFrameAck` loop, restarted after navigations (CDP
  stops screencasts on navigation — known behavior, handled).
  **On-demand only:** streaming starts when a viewer subscribes and stops
  when the last viewer leaves.
- WS protocol: text frames carry JSON events (same envelope as the bus),
  binary frames carry JPEG images. Client messages: subscribe, decision,
  screencast on/off, and — if OD-2 is accepted — input events. Approval
  decisions also travel as `POST /api/decisions` with the same token
  gate (the dashboard's HTTP channel, kept for simple automation
  clients).
- Manual control (OD-2, §11; not committed): the live view becomes an
  input surface. Viewer coordinates are mapped to page coordinates using
  screencast frame metadata (scroll offset, page scale, device
  dimensions) and dispatched through the engine input API, entering the
  session action path as Human-origin actions. Streamed rendering imposes
  frame-latency and image-quality constraints; the capability targets
  supervision and occasional intervention, not primary human browsing.
- Security: binds 127.0.0.1 only; per-launch random token printed to the
  terminal (exchanged for a cookie on first connect); `Host` header
  validation against DNS-rebinding.
- Frontend: vanilla JS, no build step, embedded into the binary at
  compile time (`include_str!`). All UI
  strings come from `frontend/i18n/en.json` catalog keys.

### 7.8 MCP tool surface

Aligned with the de-facto naming agents already know. Core set (~15):

`navigate, back, forward, reload, snapshot, screenshot, click, hover,
type, press_key, select_option, scroll, wait_for, tabs_list,
tabs_select, tabs_close, set_cookies, close_session`

Tool schema, semantics, and error mappings are specified in
`docs/TOOL_SPEC.md` (written before implementation; the spec is the
contract).

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
- One responsibility per crate, one responsibility per file, one purpose
  per symbol.
- No feature flags that cut across crate boundaries incoherently; engine
  selection is a registration, not a compile-time fork of the codebase.

### 8.2 Naming rules

- Name after responsibility, boundary, or behavior — never after
  implementation details, brands, or jokes. `SnapshotBuilder`, not
  `CdpTreeThing`.
- The brand `rutter` appears only in: repository name, binary name,
  workspace/crate names (`rutter-*`), and documentation titles. Never in
  identifiers.
- The glossary (§6) is normative. No synonyms for glossary terms in code
  or docs.
- No ad-hoc abbreviations; established ones only (URL, CDP, MCP, DOM).

### 8.3 File headers and single responsibility

Every source file (Rust, JS, shell, script) begins with a header comment
stating purpose and boundary. Rust uses inner doc comments:

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

- `thiserror` enums per crate; error types are public contracts.
- No `unwrap`/`expect`/`panic` outside tests and process init (clippy
  denies). A `catch_unwind` guard at the action boundary contains bugs as
  `ActionError::Internal`; a panic is always a reported bug, never a
  silent pass.
- Every I/O operation has a timeout. No unbounded waits exist anywhere.
- Retries with capped exponential backoff for engine launch/connect;
  circuit breaker against restart storms.
- Hostile-input discipline: the snapshot pipeline treats arbitrary page
  data as untrusted input and must degrade (truncate, fold, mark unknown)
  rather than fail or panic.

### 8.5 Performance doctrine

Targets (measured, not vibes; criterion benches for pure crates, e2e
harness for the rest):

| Metric                                              | Target          |
|-----------------------------------------------------|-----------------|
| rutter startup → MCP ready (engine lazy)            | < 100 ms        |
| Snapshot round-trip, warm engine, p50               | < 150 ms        |
| Orchestration-layer RSS (engine excluded)           | < 50 MB         |
| Concurrent contexts per engine process              | ≥ 50 (capped)   |
| Dashboard frame latency (page change → pixel)       | < 500 ms        |

The engine is spawned lazily and only while at least one session exists.
Hot paths (ref resolution, snapshot parse, culling) avoid gratuitous
allocation; correctness and clarity outrank micro-optimization
everywhere else.

### 8.6 Internationalization and content policy

- English is the only language of code, comments, docs, commit messages,
  issues, and PRs. CI enforces this with an encoding gate: tracked text
  files must contain no CJK codepoints (see `scripts/`).
- **Open decision OD-1:** real-world web fixtures for tests may need
  non-Latin content to test Unicode handling honestly. Proposed policy:
  CJK allowed **only** under `tests/fixtures/**` via an explicit
  allowlist entry, with ASCII-safe alternatives where coverage permits.
  This exception requires owner approval before the first such fixture
  lands (§11).
- Dates/times are RFC 3339 UTC; no locale-dependent formatting anywhere.
- All strings UTF-8; dashboard layout must not assume left-to-right.

### 8.7 Repository hygiene

- Conventional Commits, English only; imperative mood.
- `CHANGELOG.md` maintained per release; `CONTRIBUTING.md` and
  `docs/ARCHITECTURE.md` (this document) are release blockers.
- `.gitattributes`: LF in repo, UTF-8, binary assets marked.
- LICENSE: Apache-2.0 or MIT, decided at repo init (OD-3, §11);
  Apache-2.0 is the default for the patent grant.

## 9. Testing and Quality Gates

| Level        | Scope                                                        | Tooling            |
|--------------|--------------------------------------------------------------|--------------------|
| Unit         | Pure crates (core, observe, policy, events)                  | plain `#[test]`    |
| Golden        | Snapshot builder output on fixture DOM trees                 | `insta`            |
| Property      | Culling/folding invariants; serializer never panics          | `proptest`, fuzz   |
| Integration   | Real headless shell: launch, navigate, act, recover          | per-crate `tests/` |
| E2E           | Full MCP client → rutter → engine round-trip                 | `rutter-mcp` tests |
| Benchmark     | 20-site fixed corpus: click hit rate, snapshot tokens, task completion | harness in `scripts/` |

Release gates: fmt clean, clippy `-D warnings` clean, all tests green,
encoding gate clean, header gate clean (every source file has a header),
`cargo-deny` clean, benchmark thresholds met (≥ 90 % click hit rate on
the corpus).

## 10. Milestones and Acceptance

| Phase | Window  | Delivers                                                       | Acceptance gate                                        |
|-------|---------|----------------------------------------------------------------|--------------------------------------------------------|
| M0    | wk 1–2  | core, engine, engine-cdp, observe, cli; headless + headed launch, browse entry mode | `rutter open` on 10 real sites; click success ≥ 80 %    |
| M1    | wk 3–6  | events, mcp, auto-wait, contexts, screenshots                  | Dogfooded from a real agent; 3 e2e task classes pass    |
| M2    | mo 2–3  | session recovery, storage state, policy + approval, dashboard  | 20-site benchmark green; approval flows (grant/deny/timeout) |
| M3    | mo 4+   | packaging (cargo-dist), streamable HTTP transport, hardening, docs | Public 0.1.0 release                               |

Crates materialize with the milestone that needs them; empty crates are
forbidden. Post-M3 candidates (for example OD-2) require an accepted
open decision before scheduling.

## 11. Open Decisions

Pending product-level choices are registered here; each is resolved with
a recorded rationale. Unresolved items carry a default that applies if no
decision is made.

| ID   | Question                                                | Default if unresolved                        |
|------|---------------------------------------------------------|----------------------------------------------|
| OD-1 | Allow CJK content under `tests/fixtures/**`? (§8.6)    | Denied; ASCII-safe fixtures only             |
| OD-2 | Ship dashboard manual control? (§7.7)                   | Not shipped; dashboard remains read-only plus approval submission |
| OD-3 | License: Apache-2.0 or MIT? (§8.7)                      | Apache-2.0                                   |

## 12. Risk Register

| Risk                                        | Mitigation                                                  |
|---------------------------------------------|-------------------------------------------------------------|
| chromiumoxide maintenance gaps              | Confined to `engine-cdp` behind the trait; cdpkit/custom swap path |
| Anti-bot walls block headless traffic       | Documented non-goal; proxy pass-through; revisit post-1.0   |
| Screencast CDP edge cases (nav restart, ack)| Dedicated M2 acceptance test for continuous navigation      |
| MCP spec drift (2026-07-28 stateless wave)  | Official `rmcp` SDK tracks specs; tool surface kept stable  |
| Approval semantics block agent tool calls   | Configurable timeouts; semantics documented in TOOL_SPEC    |
| Solo-maintainer scope creep                 | Non-goals enforced at review; milestone gates are hard stops|

## 13. Definition of Done (per release)

1. All quality gates green (§9), failures disclosed, nothing skipped silently.
2. Every source file carries a truthful header; glossary terms respected.
3. Encoding gate clean (English-only policy upheld, exceptions explicit).
4. `CHANGELOG.md`, `docs/`, and tool spec updated in the same release.
5. No TODO markers, commented-out code, or debug leftovers in merged code.
6. User-visible behavior changes are covered by a test or a documented
   manual verification note.
