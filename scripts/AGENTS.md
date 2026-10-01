# scripts/ agent rules

The script table lives in [README.md](README.md). A `README.md` change
includes `README.zh.md`.

## Gates

These four scripts are CI gates. Do not relax a check to land a
change:

- `check_headers.sh` — tracked `*.rs` open with `//!`; `*.sh` and
  `*.py` open with `#` after an optional shebang; `*.js`, `*.mjs`,
  and `*.cjs` open with `/`.
- `check_encoding.sh` — tracked text is UTF-8, LF, no BOM. The gate
  does not check language.
- `check_js.sh` — `node --test tests/js/*.test.mjs`. Without Node, or
  with Node older than 18, the script exits 0 and prints `skipped`.
- `check_coverage.sh` — the default line-coverage threshold is 90.
  CI fails the workspace below that threshold. Test targets are
  excluded from the count.

## Corpus

- `smoke_open.sh` and `benchmark.sh` need network and a downloaded
  engine. They are not CI gates.
- `benchmark.sh` keeps a 90 percent navigate-and-snapshot success
  bar.
