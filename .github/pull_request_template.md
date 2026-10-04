<!--
Squash merge uses the PR title as the final commit message:
  <type>(<scope>): <subject>, imperative, lowercase, no period.
One PR does one thing; merge requirements live in the branch ruleset
(2 approvals, threads resolved, up to date with main).
-->

## What

<!-- What changed, at one screenful most. Lead with the behavior. -->

## Why

<!-- The bug, the requirement, or the design pressure. Link the issue
when there is one (`Closes #123`). -->

## How

<!-- What a reviewer should know: key decisions, alternatives considered
and rejected, boundaries touched (engine, CDP surface, browser shell).
Delete this section if the diff is self-explanatory. -->

## Verification

<!-- What ran and what it proved. CI runs the same gates; local runs catch
failures before the push. State skipped checks honestly. -->
- [ ] Tests added or updated — a bug fix ships a regression test that fails
      before the fix and passes after it
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo check --locked --workspace --all-targets`
- [ ] `cargo clippy --locked --all-targets -- -D warnings`
- [ ] `cargo doc --locked --no-deps`
- [ ] `cargo test --locked --all`
- [ ] Ignored suites, when the change touches their surface:
      `cargo test --locked -p rutter-engine-cdp --tests -- --ignored` ·
      `cargo test --locked -p rutter-integration-tests --tests -- --ignored`
- [ ] Browser shell: `npm --prefix browser audit` and
      `npm --prefix browser audit --omit=dev` (and build/test if touched)
- [ ] Anything the tests cannot reach was verified manually (describe below)

<!-- Manual steps, before/after output. Delete if empty. -->

## Compatibility impact

<!-- Breaking changes to APIs, wire format, docs, or workflows. "None" is
an answer — state it explicitly. If breaking: what breaks, who is affected,
and the migration path. -->

## Security and supply chain

<!-- Does the change touch authentication or trust boundaries, Cargo.toml /
Cargo.lock / package manifests, or GitHub workflows? cargo-deny gates the
Rust dependency graph and ci.yml audits the browser shell on their
respective triggers — use this section to give the reviewer context the
scanners cannot infer. Otherwise write "N/A". -->

## Reviewer notes

<!-- Non-obvious trade-offs, known follow-ups, areas that deserve extra
scrutiny. Delete if empty. -->
