# download/ — engine acquisition

English | [中文](README.zh.md)

Resolves a browser binary: explicit override → cache → Chrome for
Testing stable manifest (TLS), then download, extract, and install
atomically.

| File | Role |
|---|---|
| `mod.rs` | `ensure(product)`: the resolve-or-download pipeline, race-safe |
| `manifest.rs` | Parses the CfT manifest; artifact URLs pinned to the CfT storage host |
| `fetch.rs` | HTTP client: connect/total timeouts, retries 5xx not 4xx |
| `store.rs` | Cache layout, zip-slip-safe extraction, atomic install |

The full archive is extracted on purpose: chrome-headless-shell fails
without its sibling files (`icudtl.dat` et al).
