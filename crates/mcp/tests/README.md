# tests/ — per-crate functional tests

English | [中文](README.zh.md)

Public-API tests for this crate only.

| File | Covers |
|---|---|
| `tool_params.rs` | Tool input types: schema-driven deserialization, `CookieInput` → `Cookie` mapping, direction mapping |
| `http_transport.rs` | Streamable HTTP acceptance in process: `serve_http` over loopback, one session per connection, Origin and Host guards |
| `common/` | Shared engine-double fixtures for the transport tests |
