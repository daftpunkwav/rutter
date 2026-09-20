#!/usr/bin/env bash
#
# Header gate: every tracked source file must open with a header comment
# stating purpose and boundary. Rust files open with
# `//!` inner doc comments; shell and Python files may open with a
# shebang followed by a comment header; JS files open with a comment.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

failures=()

# check <file> <prefix> <skip_shebang>: the first (non-shebang) line of
# the file must start with <prefix>.
check() {
  local file=$1 prefix=$2 skip_shebang=$3 first
  if [[ $skip_shebang == yes ]]; then
    first=$(awk 'NR == 1 && /^#!/ { if ((getline line) <= 0) exit; print line; exit } NR == 1 { print; exit }' "$file")
  else
    first=$(head -n 1 "$file")
  fi
  if [[ $first != "$prefix"* ]]; then
    failures+=("$file")
  fi
}

while IFS= read -r file; do
  case $file in
    *.rs) check "$file" '//!' no ;;
    *.sh) check "$file" '#' yes ;;
    *.py) check "$file" '#' yes ;;
    *.js | *.mjs | *.cjs) check "$file" '/' no ;;
  esac
done < <(git ls-files -- '*.rs' '*.sh' '*.py' '*.js' '*.mjs' '*.cjs')

if [[ ${#failures[@]} -gt 0 ]]; then
  printf 'header gate: missing header comment in:\n' >&2
  printf '  %s\n' "${failures[@]}" >&2
  exit 1
fi

printf 'header gate: clean\n'
