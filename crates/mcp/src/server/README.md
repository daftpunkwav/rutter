# server/ — rmcp host

English | [中文](README.zh.md)

| File | Role |
|---|---|
| `mod.rs` | `RutterMcp`: session lifecycle (OnceCell + fail-fast after close), the 19 tools, result/protocol error mapping |
| `params.rs` | Pure input types: parameter structs, `Direction`, `SameSiteInput`, `CookieInput` → `Cookie` |
| `tests.rs` | Lifecycle tests over a stub engine, truncation-marker mapping (private paths) |

Parameter-mapping tests live in the crate's `tests/tool_params.rs`.

The `#[tool_router]` impl block must keep all `#[tool]` methods in one
place (rmcp macro constraint). Tool names and parameter names are
docs/tool-catalog.md contract — do not rename.
