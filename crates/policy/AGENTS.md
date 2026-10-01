# policy/ agent rules

The module map lives in [README.md](README.md) and
[src/README.md](src/README.md). Workspace crate rules live in
[../AGENTS.md](../AGENTS.md). The contract is
[../../docs/policy.md](../../docs/policy.md).

## Boundary

- Workspace dependency: `rutter-core` only.
- No I/O except `parse_policy` on TOML text.
- `canonical` stays private. Callers use `RuleSet::review`.
- Verdicts come from the rule set. Do not branch on `Origin` or on a
  caller-supplied danger flag.
- `serve` constructs one `ApprovalBroker` and passes it to
  `SessionManager`. The dashboard submits decisions to that instance.
  Do not construct a second broker for the running server.

## Evaluation

- `RuleSet::review` is the supervision entry. It canonicalizes the
  URL. `evaluate` answers class-only questions.
- The first matching rule wins, then the default verdict.
- `Navigate` is judged on its target. Every other class is judged on
  the current page URL.
- With no usable URL, URL-scoped rules match only the pattern `*`,
  and a bare `allow` becomes `require_approval`. The brief carries
  `judged_url: null` and basis `missing_url`.
- A rule sets `action_class`, `url_pattern`, or both. A blank pattern
  is rejected.
- `approval_timeout_ms` of `0` is rejected. Values above 24 h are
  rejected. The built-in window is 120 s.
- An empty policy file is default `allow` and no rules. It is not
  `RuleSet::default_set`.
- `RuleSet::default_set`: `file://` navigation, `file_upload`, and
  `cookies` are `require_approval`. Every other class defaults to
  `allow`.

## Classes

`navigation`, `pointer`, `keyboard`, `selection`, `file_upload`,
`scroll`, `cookies`. A new class updates `docs/policy.md` and
`docs/policy.zh.md`.
