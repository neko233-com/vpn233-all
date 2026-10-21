---
layout: page
title: 快速验证与发布打包
---

快速模式用于开发反馈，完整验证用于发布验收。缓存和产物复用减少重复编译，保留完整验收门槛。

```bash
bash scripts/verify-all.sh --quick
bash scripts/verify-all.sh
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-all.ps1 -Quick
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-all.ps1
```

快速模式包含格式、Clippy、Rust 测试、已有依赖下的前端构建、脚本、文档和隐私检查，跳过发布编译、服务 E2E 及联网依赖审计。显式跳过项标记部分验证，不能作为发布通过。

完整验证需 Rust、Node.js 24、Python 3、Git、Bash、ShellCheck、actionlint、gitleaks、cargo-audit、PowerShell/PSScriptAnalyzer，缺少工具真实失败。

| 层次 | 覆盖 |
| --- | --- |
| Rust | 协议字段、URI/Base64/结构化导入、格式筛选、认证、令牌、配额、注册 |
| 安装器 | 镜像回退、哈希缺失/错误、恶意归档、参数、配置数据保留 |
| 前端 | 生产构建、资源、CSP、无外部资源/源码映射、体积预算 |
| E2E | 真实三服务、管理 API、订阅认证、导出格式、注册、HTTP/SOCKS5 本地转发 |
| 隐私 | 工作树候选、暂存区、历史、凭据与非例服务器地址、发布目录 |
| 发布 | 包内容允许列表、必要文件、公开模板、校验和与归档一致 |

E2E 使用回环地址和合成凭据，不依赖真实代理节点。失败不打印私密日志或订阅响应。

浏览器回归使用 Playwright 驱动真实后台和本地订阅服务，覆盖登录失败/成功、会话存储、订阅链接与二维码、刷新、移动导航和退出；不替代每种客户端或公网代理协议的互通验证。

GitHub Pages 使用 `docs/_config.yml` 中的 `jekyll-relative-links`，在构建时把文档相对 `.md` 链接转换为页面地址，使 GitHub 仓库浏览和文档站共用同一套链接。该插件包含在 [GitHub Pages 支持依赖](https://pages.github.com/versions/) 中；用法见 [插件官方说明](https://github.com/benbalter/jekyll-relative-links)。文档源码链接检查通过与 Pages 实际构建通过是两个验收项。

版本由根 `Cargo.toml`、`version.txt`、前端包及 lockfile 同步，通过 `scripts/release.sh` / `scripts/release.ps1` 的 prepare 模式修改，按脚本帮助使用。仅维护 `main`。

`vMAJOR.MINOR.PATCH` 标签触发可复用 CI 验证，再构建 Linux x86_64、Windows x86_64、macOS x86_64 / arm64。每包只含三个二进制、前端、公开模板、脚本与文档。发布前验证 SHA256SUMS、内容和隐私；发布后再次核对下载。不得在 CI、完整验证、E2E、校验和、前端未全绿时发布。
