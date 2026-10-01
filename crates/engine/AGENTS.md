# engine/ agent rules

The module map lives in [README.md](README.md) and
[src/README.md](src/README.md). Workspace crate rules live in
[../AGENTS.md](../AGENTS.md). Supervision lives in
[../../docs/engine-supervision.md](../../docs/engine-supervision.md).

## Boundary

- Workspace dependency: `rutter-core` only.
- This crate defines `Engine`, `ContextHandle`, and `PageHandle`. It
  does not name CDP or `chromiumoxide` types.
- `session` drives engines through these traits. A backend implements
  them in its own crate.
- `EngineLauncher` is injected from above.
- `backoff` stays private.

## Supervisor

- Take the current engine from `Supervisor::engine` on every use. Do
  not store the `Arc` across calls.
- Restart behavior is tested with an injected clock and a
  deterministic `RestartPolicy` in `tests/supervisor_flow.rs`.

## Download

- Callers resolve a binary through `ensure`. `fetch`, `manifest`, and
  `store` stay private.
- Resolution order: an explicit file path, then the cache, then the
  Chrome for Testing stable manifest. An explicit path does not read
  or write the cache. A missing explicit path is an error.
- `discover_system_browser` checks standard install paths only. No
  `PATH` search and no registry.
- A blank `RUTTER_CACHE_DIR` is unset. The default root is the OS
  cache directory plus `rutter`.
- `RUTTER_ENGINE_MANIFEST_URL` overrides the manifest endpoint. A
  blank override is unset.
- Artifact URLs stay `https` on `storage.googleapis.com`.
- Fetch retries HTTP 5xx and transport failures. It does not retry
  HTTP 4xx or an over-cap body. Connect and total timeouts stay set.
- Extraction is capped, zip-slip-safe, and unpacks the full archive.
- Downloaded zips are not checksum-verified.

## Errors

`EngineError` variants carry hints. Tool results report the engine
error unchanged. The event backbone maps it to `ActionError`.
