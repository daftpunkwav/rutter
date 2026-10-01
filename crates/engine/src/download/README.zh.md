# download/ — 引擎获取

[English](README.md) | 中文

解析浏览器二进制：显式覆盖 → 缓存 → Chrome for Testing stable
manifest（TLS），随后下载、解压并原子安装。

| 文件 | 职责 |
|---|---|
| `mod.rs` | `ensure(product)`：解析或下载流水线，竞态安全 |
| `manifest.rs` | 解析 CfT manifest；端点可由 `RUTTER_ENGINE_MANIFEST_URL` 覆盖，artifact URL 钉在 CfT 存储主机上 |
| `fetch.rs` | HTTP 客户端：连接/总超时，重试 5xx 不重试 4xx |
| `store.rs` | 缓存布局、防 zip-slip 的解压、原子安装 |

完整解压整个档案是有意为之：chrome-headless-shell 缺了同目录文件
（`icudtl.dat` 等）就无法运行。

## 信任模型——已接受的现状：无哈希校验

下载的 zip **不做**校验和比对。Chrome for Testing 没有可校验的哈希
发布物：manifest 条目只有 `platform` 和 `url` 两个字段，存储主机上
也不存在与 zip 并列的 `.sha256` 旁路文件（2026-10-01 对线上端点实测
确认）。引入校验就意味着发明一个新的信任源，反而弱于下面的链条。
现接受的信任链为：

1. manifest 经 TLS 取自 Google 发布端点；
2. artifact URL 被钉在 `storage.googleapis.com` 的 https 上
   （`manifest.rs`）；
3. 档案本身按不受信输入处理：解压有体积上限且防 zip-slip
   （`store.rs`），恶意载荷既耗不尽机器也逃不出缓存。
