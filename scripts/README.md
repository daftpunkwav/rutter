# scripts/ — quality gates and benchmarks

English | [中文](README.zh.md)

| Script | Role |
|---|---|
| `check_headers.sh` | Every source file opens with its header comment (CI gate) |
| `check_encoding.sh` | Tracked text files are valid UTF-8, LF-ended, BOM-free (CI gate) |
| `check_js.sh` | The shipped page scripts and dashboard client are executed against a fake DOM (`node --test`; skipped without Node) |
| `check_coverage.sh` | Workspace line coverage from a cargo-llvm-cov lcov report stays at or above the threshold (CI gate, default 90 %) |
| `smoke_open.sh` | Smoke corpus: 10 real sites through `rutter open` |
| `benchmark.sh` | 20-site navigate+snapshot benchmark with a 90% success bar |

The four check scripts run locally and in CI; the site scripts need
network access and a downloaded engine.
