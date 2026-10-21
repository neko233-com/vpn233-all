---
layout: page
title: GitHub 镜像加速
---

安装器和节点部署脚本按候选顺序回退，镜像可用性取决于当前网络。

| 顺序 | 前缀 |
| --- | --- |
| 1 | `https://ghproxy.net/` |
| 2 | `https://gh.llkk.cc/` |
| 3 | `https://ghfast.top/` |
| 4 | `https://gh-proxy.com/` |
| 5 | 空前缀，直连 GitHub |

公开 raw 脚本还可使用 `https://cdn.jsdelivr.net/` 的 GitHub 文件映射作为备用；它不用于发布二进制，也不用于前端运行时资源。

镜像只用于公开 GitHub Release 和公开脚本，不能用于真实订阅 URL、管理 API 或含凭据地址。镜像会看到公开下载请求，服务日常运行无需镜像。

```bash
bash install.sh --release --mirror auto
bash install.sh --release --mirror off
bash install.sh --release --mirror https://ghfast.top/
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File install.ps1 -Release -Mirror auto
powershell -NoProfile -ExecutionPolicy Bypass -File install.ps1 -Release -Mirror off
```

`VPN233_GH_MIRROR` 设置首选前缀，`VPN233_GH_MIRRORS` 提供 `|` 分隔候选，`VPN233_NO_MIRROR=1` 关闭镜像。内置来源均使用 HTTPS；自定义镜像可使用 HTTP(S)，前缀拒绝账号、密码、查询参数和片段。

最新发布先解析版本，再固定 tag 下载 SHA256SUMS 和平台归档，避免下载期间 latest 改变；校验成功才替换本机文件。契约源位于 `scripts/lib/mirrors.sh` 与 `scripts/lib/mirrors.ps1`，离线测试检查顺序、回退和哈希。[排查](troubleshooting.md)。

VPN233 应用归档使用清单中的预期 SHA-256 逐源校验。连接中断时，同一来源可做有界断点续传；服务器必须正确支持字节范围。未完成或哈希错误的内容不会被安装，也不会被缓存为成功来源，而是继续尝试后续候选。不同来源的片段分别处理，最终只接受完整哈希匹配的归档。公开 raw 帮助库和清单沿用普通下载路径；显式允许缺失哈希不会绕过已有哈希的不匹配。
