#!/usr/bin/env bash
#
# JavaScript behaviour gate: the shipped page scripts and the dashboard
# client are executed, not just parsed. The scripts carry
# security-relevant and stateful logic (the ref store's sweep, the
# markdown table rules, the reconnect reset) that `node --check` cannot
# see; these tests run them against a minimal DOM and assert what they
# actually do.
#
# `node --test` is a Node builtin, so this gate adds no dependency. It
# needs Node 18 or newer; a runner without Node reports "skipped"
# rather than failing, so a contributor without Node is not blocked --
# CI runs this in the fast gates job.

set -euo pipefail

root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)

if ! command -v node >/dev/null 2>&1; then
  printf 'javascript gate: node is not installed, skipped\n'
  exit 0
fi

major=$(node --print 'process.versions.node.split(".")[0]')
if [ "$major" -lt 18 ]; then
  printf 'javascript gate: node %s is older than 18, skipped\n' "$major"
  exit 0
fi

cd "$root"
node --test tests/js/*.test.mjs
printf 'javascript gate: clean\n'
