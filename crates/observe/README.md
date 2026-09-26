# rutter-observe/ — snapshot building and markdown readouts

English | [中文](README.zh.md)

Turns a serialized DOM tree into a token-budgeted accessibility
snapshot, and a page's readable content into a markdown readout. Owns
the in-page JavaScript (serializer, reader, resolver, wait, storage
helpers) that session injects through an engine's `evaluate`.

## Boundary

Pure data transformation: no async, no I/O, no tokio dependency — the
only workspace crate beside `core` with that guarantee, which is why
its golden and property tests run without any browser. Callers inject
the scripts through an engine and hand the JSON back here.

## Contract

The rendered text is specified by `docs/snapshot-format.md` (line
format, ref minting, budgets); the markdown readout by
`docs/read-format.md` (extraction rules, guards). The specs are the
contract; this crate implements them.
