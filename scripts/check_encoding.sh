#!/usr/bin/env bash
#
# Encoding gate: every tracked text file must be valid UTF-8, carry no
# byte-order mark, and use LF line endings — the repository normalizes
# to LF via .gitattributes (*.sh is forced LF on every platform).
# Content language is free (Chinese and English are both accepted);
# this gate checks byte hygiene only, never language. Binary files
# are skipped with the same heuristic git grep uses.

set -u

root=$(git rev-parse --show-toplevel)

failures=()

while IFS= read -r -d '' file; do
  path=$root/$file
  # Skip binary files (the -I heuristic: a file is binary if it
  # contains a NUL in its first 32 KiB).
  if grep -Iq . "$path"; then
    # UTF-8 validity: iconv fails on malformed byte sequences.
    if ! iconv -f UTF-8 -t UTF-8 "$path" >/dev/null 2>&1; then
      failures+=("$file: not valid UTF-8")
      continue
    fi
    # Byte-order mark.
    if [[ $(head -c 3 "$path") == $'\xEF\xBB\xBF' ]]; then
      failures+=("$file: UTF-8 byte-order mark")
      continue
    fi
    # CRLF line endings.
    if grep -qU $'\r' "$path"; then
      failures+=("$file: CRLF line endings")
    fi
  fi
done < <(git -C "$root" ls-files -z)

if [[ ${#failures[@]} -gt 0 ]]; then
  printf 'encoding gate: byte-hygiene failures:\n' >&2
  printf '  %s\n' "${failures[@]}" >&2
  exit 1
fi

printf 'encoding gate: clean\n'
