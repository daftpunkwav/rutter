# src/ — file map

| File | Role |
|---|---|
| `lib.rs` | Public exports: `RuleSet`, `Verdict`, `ApprovalBroker` |
| `rules.rs` | `RuleSet`: verdict evaluation (`evaluate`, `evaluate_action`), approval window |
| `class.rs` | `ActionClass`: the rule axes and their parsing |
| `pattern.rs` | `Pattern`: URL glob matching (blank patterns are rejected, not widened) |
| `config.rs` | TOML parsing; rejects blank patterns and >24 h approval windows |
| `broker.rs` | `ApprovalBroker`: park/decide/wait, one decision per request id |

Verdict computation stays pure; only the broker holds state, and the
session manager owns the single broker instance per process.
