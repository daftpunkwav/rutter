# src/ — file map

English | [中文](README.zh.md)

| Item | Role |
|---|---|
| `lib.rs` | Public entry points; implementation modules are private |
| `assets/` | `serializer.js`, `reader.js` — the in-page scripts (see their README) |
| `assets.rs` | `include_str!` embedding of the scripts |
| `builder/` | DOM tree → `Snapshot` pipeline (see its README) |
| `read.rs` | Reader envelope → `Readout` (version/truncation/clamping) |
| `response.rs` | Serializer envelope → builder input (version/truncation flags) |
| `resolver.rs` | Reference/focus/select/wait/storage helper scripts |

Pure transformation only: if you are about to add a dependency on
tokio, a file system, or a network client here, the change belongs in
`session` instead.
