# download/ — 引擎获取

[English](README.md) | 中文

解析浏览器二进制：显式覆盖 → 缓存 → Chrome for Testing stable
manifest（TLS），随后下载、校验、解压并原子安装。

| 文件 | 职责 |
|---|---|
| `mod.rs` | `ensure(product)`：解析或下载流水线，竞态安全 |
| `manifest.rs` | 解析 CfT manifest；artifact URL 钉在 CfT 存储主机上 |
| `fetch.rs` | HTTP 客户端：连接/总超时，重试 5xx 不重试 4xx |
| `store.rs` | 缓存布局、防 zip-slip 的解压、原子安装 |

完整解压整个档案是有意为之：chrome-headless-shell 缺了同目录文件
（`icudtl.dat` 等）就无法运行。
