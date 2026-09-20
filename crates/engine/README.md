# rutter-engine/ — engine traits, download, supervision

English | [中文](README.zh.md)

Defines what a browser engine is for the rest of rutter (`Engine`,
`ContextHandle`, `PageHandle` traits) and owns everything that keeps
one alive: the Chrome-for-Testing downloader and the supervisor with
heartbeat, capped-backoff restarts, and a circuit breaker.

## Boundary

Speaks only `rutter-core` types and its own traits — it cannot name a
CDP type. Which binary to download and how to launch it are decided by
an `EngineLauncher` implementation injected from above (`engine-cdp`
provides the real one; tests provide scripted ones).

## Consumers

- `session` drives engines only through the traits.
- `engine-cdp` implements the traits and exposes a single launcher.
- `cli` picks launch modes and resolves `--engine-executable`.
