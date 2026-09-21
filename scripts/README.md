# scripts/ — quality gates and benchmarks

English | [中文](README.zh.md)

| Script | Role |
|---|---|
| `check_headers.sh` | Every source file opens with its header comment (CI gate) |
| `check_encoding.sh` | Tracked text files are valid UTF-8, LF-ended, BOM-free (CI gate) |
| `smoke_open.sh` | Smoke corpus: 10 real sites through `rutter open` |
| `benchmark.sh` | 20-site navigate+snapshot benchmark with a 90% success bar |

The two check scripts run locally and in CI; the site scripts need
network access and a downloaded engine.
