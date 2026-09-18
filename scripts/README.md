# scripts/ — quality gates and benchmarks

| Script | Role |
|---|---|
| `check_headers.sh` | Every source file opens with its header comment (CI gate) |
| `check_encoding.sh` | Tracked text files stay English-only (CI gate) |
| `smoke_open.sh` | M0 acceptance corpus: 10 real sites through `rutter open` |
| `benchmark.sh` | M2 gate: 20-site navigate+snapshot benchmark, 90% success bar |

The two check scripts run locally and in CI; the site scripts need
network access and a downloaded engine.
