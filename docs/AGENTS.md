# docs/ agent rules

Reading order starts at [architecture.md](architecture.md).

## Pairs

- Every English document has a Chinese `*.zh.md` mirror. Update both
  in the same change.
- Glossary terms are exact. See [glossary.md](glossary.md). Docs and
  the code they name use those terms.

## Contracts

`tool-catalog.md` and `snapshot-format.md` change in their own docs
commit, before the implementation commit. Update the `.zh.md` mirror
in that same docs commit.

These documents stay in lockstep with the code they specify. A
behavior change updates the English document and its `.zh.md` mirror:

- [read-format.md](read-format.md)
- [policy.md](policy.md)
- [events.md](events.md)
- [dashboard.md](dashboard.md)
- [sessions.md](sessions.md)
- [engine-supervision.md](engine-supervision.md)

## Scope

Architecture non-goals stay non-goals: no rendering engine of
rutter's own, no stealth or anti-fingerprinting, no consumer browser
UI, no cloud service.
