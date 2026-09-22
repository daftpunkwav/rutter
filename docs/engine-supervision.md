# Engine Supervision

English | [中文](engine-supervision.zh.md)

How rutter owns a browser process: the engine abstraction, where the
binary comes from, and how the supervisor keeps it alive — or fails
callers cleanly while it cannot.

## 1. The engine abstraction

[`rutter-engine`](../crates/engine/src/engine.rs) defines the only
engine abstraction in the codebase:

```rust
#[async_trait]
pub trait Engine: Send + Sync {
    fn descriptor(&self) -> EngineDescriptor;   // backend, version
    async fn create_context(&self, config: ContextConfig)
        -> Result<Arc<dyn ContextHandle>, EngineError>;
    async fn health(&self) -> Result<HealthReport, EngineError>;
    async fn shutdown(&self) -> Result<(), EngineError>;
}
```

Page-scoped operations — `navigate`, `evaluate`, `dispatch_input`,
`capture_screenshot`, `start_screencast`, … — live on
[`PageHandle`](../crates/engine/src/page.rs). Nothing above the trait
may know which backend is running; swapping or adding engines is
confined to implementing `EngineLauncher` (the registration trait)
plus a registration point in the CLI
([`crates/cli/src/launcher.rs`](../crates/cli/src/launcher.rs)).

Launch supports two modes: `Headless` (default in serve mode) and
`Headed` (a visible window, used by browse mode and interactive
debugging). The mode is engine-level configuration and changes no API
above the trait.

`rutter-engine-cdp` is the shipped backend — the only crate that
speaks CDP, over `chromiumoxide`. Public surface: one launcher
(`CdpLauncher`) that turns an executable path into a running
[`Engine`](../crates/engine-cdp/src/lib.rs).

## 2. Engine acquisition

[`ensure()`](../crates/engine/src/download/mod.rs) resolves a
launchable binary; nothing else in the codebase downloads or launches
engines directly:

1. An explicit `--engine-executable <PATH>` wins. The path must exist;
   its version reports as `external`; the cache is untouched.
2. The cache is consulted. `<cache-root>/engines/<product>/<version>/`
   holds the extracted binary; a hit needs no network.
3. On a miss, the Chrome for Testing manifest is fetched and the
   pinned stable artifact (`chrome-headless-shell` or `chrome`, the
   build for the host platform) is downloaded once and installed into
   the cache. The
   cached version is reused until the cache directory is cleared,
   including offline.

The cache root defaults to `<OS cache dir>/rutter` and is overridable
by `--cache-dir` / `RUTTER_CACHE_DIR`. Progress goes to stderr; stdout
stays reserved for data.

Headed runs (browse mode) prefer a system-installed browser when no
explicit path is set: `discover_system_browser()` checks standard
install locations for Chrome, Edge, or Chromium (no `PATH` search, no
registry probing) and falls back to a full Chrome for Testing
download.

`serve` defers all of this to the first launch
([`LazyLauncher`](../crates/cli/src/launcher.rs)), so the process
reaches MCP-ready before any download or disk probe.

## 3. Process supervision

The [`Supervisor`](../crates/engine/src/supervisor/mod.rs) owns the
engine's lifecycle:

- **Heartbeat.** Every 10 s the supervisor probes `Engine::health()`.
- **Restart under policy.** On a failed probe the engine is replaced:
  launch attempts wait out a capped exponential backoff (1 s base,
  30 s cap) while a sliding-window circuit breaker counts attempts —
  too many restarts inside a 60 s window opens the breaker and fails
  callers with `Terminated` until the window drains. The heartbeat
  keeps retrying after the drain, so a failed or dead engine recovers
  on its own.
- **A healthy probe closes the window.** Surviving a heartbeat wipes the
  attempt history. Without that reset, three crashes that each recovered
  would leave an engine one crash away from permanent abandonment even
  after running cleanly for an hour — the breaker would be counting the
  journey, not the trouble.
- **One state, one restart path.** Supervision state is a single
  `Phase` value (`Idle`, `Running`, `Replacing`, `BreakerOpen`) carrying
  its own restart history, rather than a slot, a flag, and a history that
  readers had to reconcile. The policy-driven launch exists once
  (`bring_up`), used by the initial start and by every later replacement.
- **Restart serialization.** A mutex guards restart cycles so
  concurrent failures relaunch once.
- **Restart notification.** Every successful replacement bumps a
  `watch` counter; the session manager's recovery task watches it and
  rebuilds each session ([sessions](sessions.md#4-recovery)).

While the engine is dead, restarting, or breaker-open,
`Supervisor::engine()` reports `EngineError::Terminated`, and affected
operations fail with that error — a crashed engine is never a crash of
rutter. This is the orchestration-layer invariant
([architecture](architecture.md#cross-cutting-invariants)).

## 4. CDP transport notes

Facts about the shipped backend that are visible above the trait:

- **Every CDP command runs under a deadline** — the fixed
  [`COMMAND_TIMEOUT`](../crates/engine-cdp/src/error.rs) (30 s) where
  a call has no dedicated budget — so a wedged browser degrades to
  errors instead of hanging a session or its supervision.
- **Screencast** uses `Page.startScreencast`: JPEG, width ≤ 1024, a
  `screencastFrameAck` loop per frame. CDP stops screencasts on
  navigation, so the transport restarts the capture when it sees the
  navigation event. Frames flow through a bounded channel (4): a slow
  viewer loses frames, not memory
  ([dashboard](dashboard.md#5-screencast)).
- **Screenshots** honor a per-page minimum interval (500 ms, from the
  context config) that caps the capture rate.
