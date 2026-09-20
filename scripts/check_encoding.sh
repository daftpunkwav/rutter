#!/usr/bin/env bash
#
# Encoding gate: code, comments, and commit messages are
# English-only. Fails on any CJK codepoint: Han
# (including extensions), Kana, Hangul, CJK punctuation, and fullwidth
# forms. Binary files are skipped by git grep. The allowlist below
# holds explicit exceptions; the Chinese documentation mirrors
# (**/*.zh.md) and the language switcher line in the English
# READMEs are the only ones.

set -u
cd "$(git rev-parse --show-toplevel)"
export LC_ALL=C.UTF-8

pattern='[\x{1100}-\x{11FF}\x{3000}-\x{303F}\x{3040}-\x{30FF}\x{3100}-\x{312F}\x{3130}-\x{318F}\x{3400}-\x{4DBF}\x{4E00}-\x{9FFF}\x{AC00}-\x{D7AF}\x{F900}-\x{FAFF}\x{FF00}-\x{FFEF}]'

# Allowlist entries are git pathspecs, for example:
#   allowlist=('tests/fixtures/**')
# `*.zh.md` and `**/*.zh.md` together cover the Chinese mirrors at any
# depth on every git build (some treat `**/` as requiring one
# directory).
allowlist=('*.zh.md' '**/*.zh.md' 'scripts/check_encoding.sh')

exclusions=()
for entry in "${allowlist[@]}"; do
  exclusions+=(":!$entry")
done

raw=$(git grep -nIP "$pattern" -- . "${exclusions[@]}")
status=$?
# git grep: 0 = matches found, 1 = no matches, >= 2 = git grep itself failed.
if [[ $status -ge 2 ]]; then
  printf 'encoding gate: git grep failed (exit %s); PCRE support missing?\n' "$status" >&2
  exit 1
fi

if [[ $status -eq 0 ]]; then
  # The language switcher line ("English | [中文](<name>.zh.md)") in
  # the English READMEs and docs is the one sanctioned CJK use
  # outside the zh mirrors.
  filter='^[^:]+:[0-9]+:English \| \[中文\]\([A-Za-z0-9.-]+\.zh\.md\)$'
  violations=$(printf '%s\n' "$raw" | grep -vE "$filter")
  if [[ -n $violations ]]; then
    printf 'encoding gate: CJK codepoints found in tracked files:\n' >&2
    printf '%s\n' "$violations" >&2
    exit 1
  fi
fi

printf 'encoding gate: clean\n'
