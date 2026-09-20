# Policy and Approvals

English | [中文](policy.zh.md)

How rutter decides which actions run, which are refused, and which
wait for a human. Everything here is pure computation plus parked
futures — [`rutter-policy`](../crates/policy/src/rules.rs) performs no
I/O. The judgment lives in rutter's rules, never in an agent's
self-declaration; that is the point of supervision.

## 1. Classes and verdicts

Actions are classified into six classes
([`class.rs`](../crates/policy/src/class.rs)): `navigation`, `pointer`,
`keyboard`, `selection`, `scroll`, `cookies`.

Every class maps to one of three verdicts:

| Verdict | Effect |
|---|---|
| `allow` | Run without supervision overhead |
| `deny` | Refuse outright; the caller sees `ApprovalDenied` |
| `require_approval` | Park the action until a human grants or denies it, or the window times out |

## 2. Rule evaluation

[`RuleSet::evaluate`](../crates/policy/src/rules.rs) matches an
ordered rule list; **the first match wins**, then the default verdict.
A rule sets any of: `action_class` (one class, or every class when
omitted), `url_pattern` (`*` wildcards), and its `verdict`.

The built-in default set is conservative for the sensitive class:
**cookies require approval**, everything else is allowed, with a
120 s approval window. Loading a policy file replaces the whole set —
an empty file is the explicit permissive configuration (default
`allow`, no rules), not the built-in one.

## 3. The judgment URL

The URL an action is judged at is where the action **leads**, not
where it comes from:

- `Navigate` is judged on its canonicalized target URL.
- Every other class is judged on the canonicalized current page URL.

Canonicalization runs the WHATWG parser
([`canonical_url`](../crates/policy/src/canonical.rs)): host case and
default ports normalize away, and patterns cannot be slipped past with
textual tricks.

**Fail-closed.** When no usable URL exists — an unreadable page, a
target that is not a URL at all, or a target embedding credentials
(`https://good.example@evil.example/`) — the session calls
`evaluate_without_url`: URL-scoped rules cannot match, class-only
rules still bind, and a bare `allow` upgrades to `require_approval`.
Missing information never passes an action unsupervised.

## 4. Approvals

The [`ApprovalBroker`](../crates/policy/src/broker.rs) parks actions
until a human decides:

1. The session opens an approval: the broker mints `apr-<n>` and hands
   back a decision receiver.
2. The session publishes `ApprovalRequested` and parks.
3. A human answers through the dashboard — the WebSocket `decision`
   message or `POST /api/decisions`
   ([dashboard](dashboard.md#4-decision-http-api)).
4. `ApprovalResolved` records the outcome; the parked action resumes
   on `granted: true`.

Mapping: denial → `ActionError::ApprovalDenied`; silence within the
window → `ApprovalTimedOut`. The window is 120 s by default and
configurable (`approval_timeout_ms`, capped at 24 h). A late decision
on an unknown approval is rejected; a cancelled waiter (a
disconnecting client) reclaims its slot. Approvals apply to
[agent-origin](glossary.md) actions only; human-origin actions bypass
approval and are recorded on the same timeline.

## 5. TOML configuration

```toml
default = "allow"            # verdict for actions no rule matched
approval_timeout_ms = 120000 # human answer window

[[rules]]
action_class = "navigation"
url_pattern  = "https://*.example.com/*"
verdict      = "allow"

[[rules]]
action_class = "cookies"
verdict      = "require_approval"
```

[`parse_policy`](../crates/policy/src/config.rs) rejects: unknown
verdict or class names, a rule with neither `action_class` nor
`url_pattern`, a blank `url_pattern` (it would silently widen the rule
to every URL), and windows above 24 h. Parse errors carry the TOML
position so operators can fix the file.
