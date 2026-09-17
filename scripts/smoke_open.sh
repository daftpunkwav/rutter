#!/usr/bin/env bash
#
# M0 acceptance helper: runs `rutter open` against a fixed corpus of
# real sites and reports a pass/fail summary (blueprint §10, M0 gate).
# Manual and network-dependent by design; never part of CI gates.

set -uo pipefail
cd "$(git rev-parse --show-toplevel)"

sites=(
  "https://example.com"
  "https://www.wikipedia.org"
  "https://news.ycombinator.com"
  "https://github.com"
  "https://developer.mozilla.org"
  "https://www.rust-lang.org"
  "https://docs.rs"
  "https://crates.io"
  "https://httpbin.org/html"
  "https://text.npr.org"
)

passed=0
failed=0

for site in "${sites[@]}"; do
  if cargo run -q -p rutter -- open "$site" >/dev/null 2>&1; then
    printf 'PASS %s\n' "$site"
    passed=$((passed + 1))
  else
    printf 'FAIL %s\n' "$site"
    failed=$((failed + 1))
  fi
done

printf '\n%d passed, %d failed\n' "$passed" "$failed"
[[ $failed -eq 0 ]]
