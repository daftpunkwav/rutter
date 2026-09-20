# src/ — file map

English | [中文](README.zh.md)

| File | Role |
|---|---|
| `lib.rs` | Key exports: `RuleSet`, `Review`, `Verdict`, `ApprovalBroker` |
| `rules.rs` | `RuleSet`: `review` (canonicalize, match, answer with a brief) and `evaluate`; approval window |
| `brief.rs` | `ApprovalBrief`, `ApprovalEffect`, `VerdictBasis`: what a human decides from |
| `class.rs` | `ActionClass`: the rule axes and their parsing |
| `pattern.rs` | `Pattern`: URL glob matching (blank patterns are rejected, not widened) |
| `canonical.rs` | `canonical_url`: WHATWG canonicalization of the judgment URL (credential decoys fail closed) |
| `config.rs` | TOML parsing; rejects blank patterns and >24 h approval windows |
| `broker.rs` | `ApprovalBroker`: park/decide/wait, one decision per request id |

Verdict computation stays pure; only the broker holds state, and the
session manager owns the single broker instance per process.
