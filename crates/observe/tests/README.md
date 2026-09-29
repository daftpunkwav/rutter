# tests/ — per-crate functional tests

English | [中文](README.zh.md)

Public-API tests for this crate only.

| File | Covers |
|---|---|
| `snapshot_pipeline.rs` | Envelope → snapshot conversion (truncation flags, rendered refs), embedded script contract markers |
| `page_scripts.rs` | Runs the embedded serializer and reader under `node --test` (skipped without Node): the ref sweep, the markdown table rules and the character budget are proved by execution, not by reading the source |
