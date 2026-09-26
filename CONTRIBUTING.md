# Contributing

Thanks for looking at rutter. This document covers the working rules;
the code-facing reference lives under [`docs/`](docs/architecture.md),
starting from the architecture map.

## Ground rules

- **Chinese and English are both acceptable; neither is ever
  forbidden.** The existing code base's comments are English, and new
  comments may just as well be written in Chinese — clarity is the
  only bar. Commit messages follow Conventional Commits in English
  (see `AGENTS.md`), and documentation ships as English `README.md`
  files with Chinese `README.zh.md` mirrors — update the two
  together. The encoding gate (`scripts/check_encoding.sh`) checks
  byte hygiene only: valid UTF-8, LF endings, no BOM.
- **Scope is fixed.** The non-goals in
  [`docs/architecture.md`](docs/architecture.md) are enforced at
  review: no rendering engine, no stealth/anti-fingerprinting, no
  consumer browser UI, no cloud service. Scope creep is rejected by
  default.
- **Glossary terms are exact.** Engine, Context, Page, Session,
  Snapshot, Reference, Action, Verdict, Event, Origin — one concept,
  one term ([`docs/glossary.md`](docs/glossary.md)). No synonyms in
  code or docs.
- **Every source file opens with a truthful header** stating purpose
  and boundary; the header gate
  (`scripts/check_headers.sh`) checks the shape, reviews check the
  truth.

## Workflow

1. Fork / branch using `<type>/<kebab-case-description>`
   (for example `feat/context-compaction`).
2. Keep diffs minimal and traceable to the request; every line
   justifies itself.
3. Before pushing, run the local gates:

   ```sh
   cargo fmt --all -- --check
   cargo clippy --all-targets -- -D warnings
   cargo test --all
   bash scripts/check_headers.sh
   bash scripts/check_encoding.sh
   ```

4. Integration and acceptance suites need the real engine; run them
   once locally (the engine caches after the first download):

   ```sh
   cargo test -p rutter-engine-cdp --tests -- --ignored
   cargo test -p rutter-integration-tests --test mcp_e2e -- --ignored
   cargo test -p rutter-integration-tests --test approval_e2e -- --ignored
   cargo test -p rutter-integration-tests --test http_e2e -- --ignored
   cargo test -p rutter-integration-tests --test open_e2e -- --ignored
   cargo test -p rutter-engine-cdp --test screencast -- --ignored
   ```

## Commits

Conventional Commits, English, imperative mood, subject ≤ 50
characters, one concern per commit:

```
feat: add session storage replay
fix: keep ack loop alive across navigations
```

## Specs before surfaces

`docs/tool-catalog.md` and `docs/snapshot-format.md` are contracts: a
change to tool semantics or snapshot output lands as a spec change
first (a separate docs commit), then the implementation. Contract
changes hidden inside implementation commits will be asked to split.

## Reporting issues

Include: rutter version (`rutter --version`), OS, how the engine was
acquired (downloaded, cached, or `--engine-executable`), the tool
sequence (or command line) that failed, and the full error text
including the `hint:` line. Redact any URLs you cannot share and note
that in the report.
