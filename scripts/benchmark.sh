#!/usr/bin/env bash
#
# M2 acceptance benchmark: runs `rutter open` (navigate + snapshot
# round-trip) against a 20-site corpus and reports the success rate
# (blueprint §10, M2 gate; §9 benchmark level). Manual and
# network-dependent by design; never part of CI gates.

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
  "https://www.gnu.org/software/wget/"
  "https://www.iana.org/domains/reserved"
  "https://www.rfc-editor.org/rfc/rfc2616"
  "https://http.cat/200"
  "https://httpstat.us/200"
  "https://api.github.com/zen"
  "https://www.w3.org/History/19921103-hypertext/hypertext/WWW/Link.html"
  "https://motherfuckingwebsite.com/"
  "https://books.toscrape.com/"
  "https://quotes.toscrape.com/"
)

passed=0
failed=0
failed_sites=()

for site in "${sites[@]}"; do
  if cargo run -q -p rutter -- open "$site" >/dev/null 2>&1; then
    printf 'PASS %s\n' "$site"
    passed=$((passed + 1))
  else
    printf 'FAIL %s\n' "$site"
    failed_sites+=("$site")
    failed=$((failed + 1))
  fi
done

printf '\nnavigate+snapshot: %d/%d succeeded\n' "$passed" "${#sites[@]}"
if [[ $failed -gt 0 ]]; then
  printf 'failed sites:\n'
  printf '  %s\n' "${failed_sites[@]}"
fi
# The M2 gate is a healthy navigation rate; network flakiness on any
# single site is reported but does not fail the run unless it drops
# below 90 percent.
threshold=$(( ${#sites[@]} * 9 / 10 ))
[[ $passed -ge $threshold ]]
