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

`VPN233_GH_MIRROR` 设置首选前缀，`VPN233_GH_MIRRORS` 提供 `|` 分隔候选，`VPN233_NO_MIRROR=1` 关闭镜像。只向合法 HTTPS 公共下载地址发送请求。

最新发布先解析版本，再固定 tag 下载 SHA256SUMS 和平台归档，避免下载期间 latest 改变；校验成功才替换本机文件。契约源位于 `scripts/lib/mirrors.sh` 与 `scripts/lib/mirrors.ps1`，离线测试检查顺序、回退和哈希。[排查](troubleshooting.md)。
