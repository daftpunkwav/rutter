#!/usr/bin/env bash
#
# Coverage gate: fails when the workspace line coverage in a
# cargo-llvm-cov lcov report drops below the threshold (default 90 %).
# Only source files count — test targets themselves are excluded, so
# the number measures the code, not the suites.

set -euo pipefail

report=${1:-target/coverage.lcov}
threshold=${2:-90}

if [ ! -f "$report" ]; then
  echo "coverage report not found: $report" >&2
  echo "run: RUTTER_BIN=\"$PWD/target/llvm-cov-target/debug/rutter\" cargo llvm-cov --locked --workspace --bins --tests --lcov --output-path $report -- --include-ignored --test-threads=1" >&2
  exit 2
fi

awk -v threshold="$threshold" '
  /^SF:/ {
    path = substr($0, 4)
    gsub(/\\/, "/", path)
    # Test sources are the measurement, not the measured: skip them.
    if (path ~ /\/tests\//) in_test = 1; else in_test = 0
    next
  }
  /^end_of_record/ { in_test = 0; next }
  # This skip must stay below the record terminator above, or the
  # terminator of a skipped record is never reached.
  in_test { next }
  /^DA:/ {
    total++
    split(substr($0, 4), fields, ",")
    if (fields[2] + 0 > 0) covered++
    next
  }
  END {
    if (total == 0) {
      print "coverage gate: no source lines found in " ARGV[1] > "/dev/stderr"
      exit 2
    }
    percent = 100 * covered / total
    printf "line coverage: %.2f%% (%d/%d, threshold %.0f%%)\n",
           percent, covered, total, threshold
    if (percent + 0 < threshold + 0) exit 1
  }
' "$report"
