# download/ — engine acquisition

English | [中文](README.zh.md)

Resolves a browser binary: explicit override → cache → Chrome for
Testing stable manifest (TLS), then download, extract, and install
atomically.

| File | Role |
|---|---|
| `mod.rs` | `ensure(product)`: the resolve-or-download pipeline, race-safe |
| `manifest.rs` | Parses the CfT manifest; endpoint overridable via `RUTTER_ENGINE_MANIFEST_URL`, artifact URLs pinned to the CfT storage host |
| `fetch.rs` | HTTP client: connect/total timeouts, retries 5xx not 4xx |
| `store.rs` | Cache layout, zip-slip-safe extraction, atomic install |

The full archive is extracted on purpose: chrome-headless-shell fails
without its sibling files (`icudtl.dat` et al).

## Trust model — accepted, no hash pin

Downloaded zips are **not** checksum-verified. Chrome for Testing
publishes no hash to verify against: the manifest entries carry only
`platform` and `url`, and the storage host serves no `.sha256` sidecar
next to the artifacts (verified against the live endpoints 2026-10-01).
Adding verification would mean inventing a new trust source, which is
weaker than the chain below. The accepted chain is instead:

1. the manifest arrives over TLS from the Google-published endpoint,
2. the artifact URL is pinned to https on
   `storage.googleapis.com` (`manifest.rs`), and
3. the archive itself is untrusted input: extraction is capped and
   zip-slip-safe (`store.rs`), so a hostile payload cannot exhaust the
   machine or escape the cache.
