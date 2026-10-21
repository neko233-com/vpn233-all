---
layout: page
title: 快速验证与发布打包
---

快速模式用于开发反馈，完整验证用于发布验收。缓存和产物复用减少重复编译，保留完整验收门槛。

```bash
bash scripts/verify-all.sh --quick
bash scripts/verify-all.sh --quick --no-quick-cache
bash scripts/verify-all.sh
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-all.ps1 -Quick
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-all.ps1 -Quick -NoQuickCache
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-all.ps1
```

快速模式首次运行格式、Clippy、Rust 测试、已有依赖下的前端构建、脚本、文档和隐私检查，跳过发布编译、服务 E2E、官方内核解析及联网依赖审计。成功后在本地忽略目录 `.cache/quick-verify/` 保存仅含摘要、时间和检查数的报告。再次运行仍实时检查版本、候选工作树、暂存区、完整 Git 历史隐私及 diff，再复用匹配的成功结果，明确显示 `QUICK CACHED`。

缓存指纹覆盖候选文件全部内容、权限、暂存区和历史引用，锁文件，已安装 `node_modules` 和前端产物的每个文件内容，工具版本及入口文件、PowerShell 分析器模块，以及相关构建环境、Cargo/npm 配置和 Vite 环境文件的摘要。修改源码、依赖、产物或这些输入会触发真实检查；被忽略的服务运行数据不参与开发缓存。失败、中断、损坏的记录或验证期间输入变化都不能生成可复用结果。无需信任本地缓存时用 `--no-quick-cache` / `-NoQuickCache` 强制重跑。

缓存仅加速完整的 Quick 检查集合，显式跳过项标记部分验证。完整验证和 CI 始终执行真实检查，完全不使用 Quick 报告；Quick 报告不能作为发布通过或安全签名。

Quick 使用 Cargo 自身管理的锁定 crates 缓存，不逐次重扫用户全局 Cargo registry 或每个工具安装目录。手工修改这些全局安装内容后，应使用强制重跑选项；完整验证始终运行真实编译与测试。

首次安装或更换前端依赖的 Quick 需要下一次依赖稳定的真实检查成功后才保存报告。设置自定义脚本启动文件、Node 启动选项或 Rust 编译包装器时保守禁用结果缓存。

完整验证需仓库锁定版本的 Rust、Node.js 24、Python 3、Git、Bash、ShellCheck、actionlint、gitleaks、cargo-audit、PowerShell/PSScriptAnalyzer 和 OpenSSL，Linux 还需提供 `systemd-analyze` 的 systemd 工具包，缺少工具真实失败。安装器测试把用户服务目录隔离在临时 `XDG_CONFIG_HOME`，以受控管理器验证首装、重装归属与失败回滚，并在 Linux 使用真实 systemd 解析器验收生成单元。官方 Mihomo 解析门禁首次联网下载固定版本的公开内核归档，之后复用本地缓存，每次重新核对固定 SHA-256；下载或校验失败不会静默跳过。

| 层次 | 覆盖 |
| --- | --- |
| Rust | 协议字段、URI/Base64/结构化导入、格式筛选、认证、令牌、配额、注册 |
| 官方核心解析 | 经 SHA-256 校验的 Mihomo，真实 Provider API 生成的 8 入站及合并配置、核心库 57 个合成出站配置，仅执行 `-t` |
| 安装器 | 镜像回退、哈希缺失/错误、恶意归档、参数、配置数据保留 |
| 前端 | 生产构建、资源、CSP、无外部资源/源码映射、体积预算 |
| E2E | 真实三服务、管理 API、订阅认证、导出格式、注册、HTTP/SOCKS5 本地转发 |
| 隐私 | 工作树候选、暂存区、历史、凭据与非例服务器地址、发布目录 |
| 发布 | 包内容允许列表、必要文件、公开模板、校验和与归档一致 |

E2E 使用回环地址和合成凭据，不依赖真实代理节点。失败不打印私密日志或订阅响应。

浏览器回归使用 Playwright 驱动真实后台和本地订阅服务，覆盖登录失败/成功、会话存储、结构化节点显示、不同节点独立选择与取消保存、订阅链接与二维码、刷新、移动导航和退出；不替代每种客户端或公网代理协议的互通验证。

官方解析验收可单独运行，下列导出目录需尚未存在同名配置文件，示例使用被 Git 忽略的 `target/`。首次 `--download-core` 只下载并校验公开官方二进制，后续复用缓存且重新校验；也可提供受信本地 `--mihomo-bin` 或官方归档 `--mihomo-archive`。

```bash
cargo run -p vpn233-core-lib-rust --example mihomo_fixtures --locked -- target/mihomo-fixtures
python scripts/e2e/check_mihomo_parser.py --download-core --core-fixtures-dir target/mihomo-fixtures
```

该命令需先有本机 release Provider 二进制，或用 `--provider-bin` 指定。所有代理端点使用合成占位内容，执行配置检查不会启动代理监听，也不会验证公网连通性。

GitHub Pages 使用 `docs/_config.yml` 中的 `jekyll-relative-links`，在构建时把文档相对 `.md` 链接转换为页面地址，使 GitHub 仓库浏览和文档站共用同一套链接。该插件包含在 [GitHub Pages 支持依赖](https://pages.github.com/versions/) 中；用法见 [插件官方说明](https://github.com/benbalter/jekyll-relative-links)。文档源码链接检查通过与 Pages 实际构建通过是两个验收项。

版本由根 `Cargo.toml`、`version.txt`、前端包及 lockfile 同步，通过 `scripts/release.sh` / `scripts/release.ps1` 的 prepare 模式修改，按脚本帮助使用。仅维护 `main`。

`vMAJOR.MINOR.PATCH` 标签触发可复用 CI 验证，再构建 Linux x86_64、Windows x86_64、macOS x86_64 / arm64。每包只含三个二进制、前端、公开模板、脚本与文档。发布前验证 SHA256SUMS、内容和隐私；发布后再次核对下载。不得在 CI、完整验证、E2E、校验和、前端未全绿时发布。

macOS 两种架构分别使用 [GitHub 官方原生运行器](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)，发布工作流要求二进制架构、三个程序版本检查和真实回环 E2E 全部通过。

Linux 发布程序使用 Ubuntu 22.04 / glibc 2.35 编译基线，保持当前 Rust 工具链，并在最新 Ubuntu 上执行 E2E，兼顾依赖更新与发行版兼容。
