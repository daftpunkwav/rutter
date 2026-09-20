# rutter-policy/ — supervision rules

English | [中文](README.zh.md)

Pure verdict computation plus the approval state machine. A TOML rule
set maps action class × URL pattern to `Allow | Deny |
RequireApproval`; the `ApprovalBroker` parks require-approval actions
while the session publishes the request, and resolves on a human
decision or timeout.

## Boundary

Depends on `core` only inside the workspace; no I/O beyond parsing the
TOML config. It
never executes anything and never asks who is asking — dangerousness
is decided by the rules, never by the agent's self-declaration
(docs/policy.md). The approval window (default 120 s, rejected above
24 h) lives on the `RuleSet`, set via `approval_timeout_ms` in the
config.

## Consumers

- `session` evaluates verdicts before agent actions and parks on the
  broker.
- `dashboard` submits decisions to the same broker (the one the
  session manager owns — there is exactly one per process).
