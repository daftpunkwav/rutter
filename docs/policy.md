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

Supervision enters through
[`RuleSet::review`](../crates/policy/src/rules.rs): it takes the
operation and its raw URL, canonicalizes that URL itself, and answers
with a `Review` — allowed, denied, or parked together with the brief a
human decides from. `evaluate` remains for class-only questions, so no
caller reaches a verdict while skipping canonicalization.

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
(`https://good.example@evil.example/`) — `review` judges as if there
were no URL: URL-scoped rules cannot match unless their pattern is the
catch-all `*`, class-only rules still
bind, and a bare `allow` upgrades to `require_approval`. The brief then
carries `judged_url: null` and basis `missing_url`, so a human can tell
an unverifiable target from a known one. Missing information never
passes an action unsupervised.

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

### What the human is shown

The parked request carries an
[`ApprovalBrief`](../crates/policy/src/brief.rs) that policy built, not
the action it parked:

| Field | Meaning |
|---|---|
| `class` | the class the verdict was reached under |
| `judged_url` | the canonical URL judged; `null` on the fail-closed path |
| `basis` | which rule spoke (`rule`, with its one-based index and pattern), the set's `set_default`, or the `missing_url` upgrade |
| `effect` | what a grant authorizes: the action itself, or `cookies` with a count |

Policy owns this description because only policy knows those four
things. A cookie write has no `Action` variant, and substituting a
nearby action would let a human approve "reload" while authorizing a
cookie write — so `effect` names its own operation. The wire shape is
pinned by `crates/policy/tests/brief_wire.rs`, which is the only thing
holding it in step with the schema-less frontend.

### The audit trail

Every parked decision appends one JSON line to
`<cache-dir>/sessions/approvals.jsonl`: time, session, page, `request_id`,
the class, the judged URL, the basis, a summary of the effect, the outcome,
and how long the wait ran.

The session layer writes it, not the dashboard, so the record exists even
when nobody was watching — the case a timeout most needs explained. The
outcome distinguishes `granted`, `denied`, `timed_out`, and `cancelled`, the
last covering a client that disconnected mid-park. The entry records the
decision, not a verified human identity: on one machine and one account
rutter cannot tell a person at the dashboard from a process holding the
dashboard token ([dashboard §2](dashboard.md#2-access-control)).

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
to every URL), and windows above 24 h. TOML syntax errors carry the
TOML position; the rejections above are reported as messages.
