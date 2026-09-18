# server/ — rmcp host

| File | Role |
|---|---|
| `mod.rs` | `RutterMcp`: session lifecycle (OnceCell + fail-fast after close), the 17 tools, result/protocol error mapping |
| `params.rs` | Pure input types: parameter structs, `Direction`, `SameSiteInput`, `CookieInput` → `Cookie` |
| `tests.rs` | Lifecycle tests (stub engine), truncation marker, param mapping |

The `#[tool_router]` impl block must keep all `#[tool]` methods in one
place (rmcp macro constraint). Tool names and parameter names are
TOOL_SPEC contract — do not rename.
