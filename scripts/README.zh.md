# scripts/ — 质量门禁与基准

[English](README.md) | 中文

| 脚本 | 职责 |
|---|---|
| `check_headers.sh` | 每个源文件以头注开头（CI 门禁） |
| `check_encoding.sh` | git 跟踪的文本文件为合法 UTF-8、LF 换行、无 BOM（CI 门禁） |
| `smoke_open.sh` | 冒烟语料：经 `rutter open` 驱动 10 个真实站点 |
| `benchmark.sh` | 20 站点 navigate+snapshot 基准，成功率门槛 90% |

两个 check 脚本在本地与 CI 运行；站点脚本需要网络访问与已下载的引
擎。
