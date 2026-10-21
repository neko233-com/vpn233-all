---
layout: page
title: 快速验证与发布打包
---

快速模式用于开发反馈，完整验证用于发布验收。缓存和产物复用减少重复编译，保留完整验收门槛。

验证工具统一使用 Rust workspace 中的 `vpn233-xtask`，无需 Python。隐私扫描、版本链、归档、校验和、缓存、真实 HTTP 夹具和官方解析验收均由同一个原生工具完成；回归测试随 `cargo test --locked --workspace --no-fail-fast` 自动运行。安装脚本仍通过真实 Bash / PowerShell 执行，浏览器验收使用前端已有的 Node.js / Playwright。

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

脚本在独立的 `.cache/xtask-bootstrap` 构建开发工具，并把其实际二进制内容纳入缓存指纹。该目录位于业务 `target/` 之外，避免 Rust 依赖缓存恢复或清理时删除仍需执行的工具。业务编译和测试不会在一次缓存事务中改写验证工具。workspace 测试统一使用 `--no-fail-fast`，单个测试目标失败后仍收齐其余目标，最终保留非零失败状态。正式包仍只有三个业务程序，开发工具不随安装包分发。

开发构建仅为压缩与哈希热路径依赖 `zlib-rs`、`sha2` 设置包级 `opt-level = 3`，减少纯 Rust 工具处理归档的耗时；其余开发和发布 profile 保持既有设置。CI 与发布的原生工具编译缓存同时包含根 `Cargo.toml`、锁文件、工具链和工具源码的内容指纹，修改这些优化配置会重新构建工具，完整验证仍执行全部真实检查。

原生工具缓存未精确命中时，仅回退到相同运行器系统与平台前缀：Linux CI 的 glibc 2.35 基线、Windows MSVC 作业、发布预检的最新 Linux 主机及两种 macOS 架构彼此隔离。回退只复用依赖编译产物，每次入口仍执行 `cargo build --locked`，由 Cargo 核对当前源码、锁文件、编译器与配置并重建需要更新的部分；隐私扫描和测试照常运行，不把编译缓存当作验证结果。

Quick 使用 Cargo 自身管理的锁定 crates 缓存，不逐次重扫用户全局 Cargo registry 或每个工具安装目录。手工修改这些全局安装内容后，应使用强制重跑选项；完整验证始终运行真实编译与测试。

首次安装或更换前端依赖的 Quick 需要下一次依赖稳定的真实检查成功后才保存报告。设置自定义脚本启动文件、Node 启动选项或 Rust 编译包装器时保守禁用结果缓存。

完整验证需仓库锁定版本的 Rust、Node.js 24、Git、Bash、ShellCheck、actionlint、gitleaks、cargo-audit、PowerShell/PSScriptAnalyzer 和 OpenSSL，Linux 还需提供 `systemd-analyze` 的 systemd 工具包，缺少工具真实失败。安装器测试把用户服务目录隔离在临时 `XDG_CONFIG_HOME`，以受控管理器验证首装、重装归属与失败回滚，并在 Linux 使用真实 systemd 解析器验收生成单元。官方 Mihomo 解析门禁首次联网下载固定版本的公开内核归档，之后复用本地缓存，每次重新核对固定 SHA-256；下载或校验失败不会静默跳过。

| 层次 | 覆盖 |
| --- | --- |
| Rust | 协议字段、URI/Base64/结构化导入、格式筛选、认证、令牌、配额、注册 |
| 官方核心解析 | 经 SHA-256 校验的 Mihomo，真实 Provider API 生成的 8 入站及合并配置、核心库 57 个合成出站配置，仅执行 `-t` |
| 安装器 | 镜像回退、逐源哈希、有界断点恢复、真实 HTTP 截断/Range 负例、哈希缺失/错误、恶意归档、参数、配置数据保留 |
| 前端 | 生产构建、资源、CSP、无外部资源/源码映射、体积预算 |
| E2E | 真实三服务、管理 API、订阅认证、导出格式、注册、HTTP/SOCKS5 本地转发 |
| 隐私 | 工作树候选、暂存区、历史、凭据与非例服务器地址、发布目录 |
| 发布 | 包内容允许列表、必要文件、公开模板、校验和与归档一致 |

E2E 使用回环地址和合成凭据，不依赖真实代理节点。失败不打印私密日志或订阅响应。

完整验证和 CI 的平台 E2E 还使用已经构建并校验的公开包执行离线安装生命周期：调用真实安装脚本，再由真实 `start` 启动三个业务程序，检查回环健康、前端、Provider 和订阅，核对进程身份后停止并确认监听关闭。Windows 使用实际 PowerShell 5.1，不以模拟 `Start-Process` 或休眠进程替代业务程序；该检查复用现成二进制和前端，不重复编译或重跑已有的完整 14 组 E2E，也不能替代发布后的真实联网安装升级。失败只打印固定阶段和受控错误类型，不输出路径、URL、配置或日志正文。

已生成并校验平台归档后，可用 `cargo xtask network installed-lifecycle --assets target/verify-package/assets --tag v版本号` 单独执行这项离线检查。`--assets` 指定包含平台归档、`SHA256SUMS` 和 `version.txt` 的目录，工具消费校验后的归档，不接受已解包的安装目录替代。其结果明确标记 `online_installation_verified: false`。

离线与公开客户测试仅在合成临时 Provider 配置中把心跳周期设为 1 秒，减少等待注册的耗时；业务程序的生产默认值仍是 30 秒。

Rust 回环 HTTP 夹具直接绑定内核分配的端口并通过私有就绪文件交接，不执行主机名反向解析。两端 E2E 使用同一实现，两种 macOS 原生 E2E 仍是发布必过项。

浏览器回归使用 Playwright 驱动真实后台和本地订阅服务，覆盖登录失败/成功、会话存储、结构化节点显示、不同节点独立选择与取消保存、订阅链接与二维码、刷新、移动导航和退出；不替代每种客户端或公网代理协议的互通验证。

官方解析验收可单独运行，下列导出目录需尚未存在同名配置文件，示例使用被 Git 忽略的 `target/`。首次 `--download-core` 只下载并校验公开官方二进制，后续复用缓存且重新校验；也可提供受信本地 `--mihomo-bin` 或官方归档 `--mihomo-archive`。

```bash
cargo run -p vpn233-core-lib-rust --example mihomo_fixtures --locked -- target/mihomo-fixtures
cargo xtask network mihomo --download-core --core-fixtures-dir target/mihomo-fixtures
```

该命令需先有本机 release Provider 二进制，或用 `--provider-bin` 指定。所有代理端点使用合成占位内容，执行配置检查不会启动代理监听，也不会验证公网连通性。

GitHub Pages 仅在正式发布整体成功后触发；手动入口也必须指定已通过的发布 run，使用同一套 Rust 门禁核准最新稳定标签、精确提交、全套 CI / E2E / 校验和、公开安装与在线升级。每次 main 提交仅检查文档源码，不启动 Jekyll 或部署。文档树与上次实际成功部署相同则跳过构建、上传和部署；发布前还需本地生成页面的隐私、链接、锚点和浏览器零外联检查。部署成功才写入真实发布提交和文档树证明，避免默认分支 SHA 混淆及迟到的旧版本回滚。

CI 的已验证原生工具仅保存 7 天，Linux 上传前移除调试信息，两平台采用低级压缩；Pages 作业内临时复用工具仅保存 1 天。手动重试 Pages 时，同一 Release run 的已验证工具必须仍可下载，过期或缺失即失败，不从未验证源码重建。永久部署证明保存在 GitHub deployment 元数据中，不依赖这些短期工具产物。

Pages 使用 `docs/_config.yml` 中的 `jekyll-relative-links`，在构建时把相对 `.md` 链接转换为页面地址，使仓库浏览和文档站共用同一套链接。该插件包含在 [GitHub Pages 支持依赖](https://pages.github.com/versions/) 中；用法见 [插件官方说明](https://github.com/benbalter/jekyll-relative-links)。

版本由根 `Cargo.toml`、`version.txt`、前端包及 lockfile 同步，通过 `scripts/release.sh` / `scripts/release.ps1` 的 prepare 模式修改，按脚本帮助使用。仅维护 `main`。

`vMAJOR.MINOR.PATCH` 标签触发可复用 CI 验证，再构建 Linux x86_64、Windows x86_64、macOS x86_64 / arm64。每包只含三个二进制、前端、公开模板、脚本与文档。发布前验证 SHA256SUMS、内容和隐私；发布后再次核对下载。不得在 CI、完整验证、E2E、校验和、前端未全绿时发布。

公开客户验收采用候选发布再晋级：先把全部 10 个已校验资产上传草稿，Rust 工具逐项核对 API 的大小和 SHA-256，再公开为 `prerelease=true`、`make_latest=false`。客户验收必须显式使用 `--candidate`，严格绑定标签、提交和完整资产；默认公共安装验收仍拒绝预发布版本。两平台必须从公开地址真实安装、在已安装目录在线升级并保留配置数据，再通过运行与完整 E2E。候选失败时不晋级为稳定版，也不改变现有 latest。

第 22 个发布作业仅在前 21 个必需作业及其关键步骤全部成功后晋级；Rust 工具重新核准同一仓库、Release run、标签提交、发布 ID 及候选证明中的全部资产指纹，防止漏测、资产替换或旧版本覆盖更高的 latest。晋级只修改 `prerelease` / `latest` 元数据，不改标签或资产，并独立读回稳定状态及 latest 身份。仓库级晋级并发锁避免不同标签竞争。Pages 仍要求整条 Release 包含晋级和读回在内全部成功，手动入口也不能绕过；仅公开候选或已有下载资产不算完成正式验收。

macOS 两种架构分别使用 [GitHub 官方原生运行器](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)，发布工作流要求二进制架构、三个程序版本检查和真实回环 E2E 全部通过。

Linux 发布程序使用 Ubuntu 22.04 / glibc 2.35 编译基线，保持当前 Rust 工具链，并在最新 Ubuntu 上执行 E2E，兼顾依赖更新与发行版兼容。
