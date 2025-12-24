# Neko233 VPN (Go Rewrite)

这是一个用 Go 语言重写的 VPN 项目，包含服务端和客户端。

## 特性
- **多协议支持**: 客户端支持 **SOCKS5**、**HTTP** 代理以及 **TUN (Tunnel)** 模式。
- **DNS 劫持**: 内置 DNS 服务器，支持分流解析，防止 DNS 泄漏。
- **Clash 兼容路由**: 路由规则独立为 `rules.yaml`，兼容 Clash 的规则格式（DOMAIN, IP-CIDR 等），可直接复用 Clash 生态。
- **CLI 友好**: 运行不带参数时自动显示帮助信息，支持 `-c` 指定配置文件。
- **REALITY 伪装**: 服务端支持流量伪装，未授权的连接将转发至目标网站。
- **安全加密**: 握手阶段采用 **AES-GCM** 加密，确保目标地址等敏感信息不被探测。
- **一键打包**: 根目录提供 `.cmd` 脚本，打包后的名字为 `vpn233-client` 和 `vpn233-server`。

## 目录结构
- `server-vpn-neko233/`: 服务端代码
- `client-vpn-neko233/`: 客户端代码
- `client-vpn-neko233/rules.yaml`: 路由规则文件（Clash 格式）
- `server-打包-linux.cmd`: 打包生成 `vpn233-server`
- `client-打包-windows-macos-linux.cmd`: 打包生成 `vpn233-client`

## 使用方法
1. **服务端**:
   - 修改 `server-vpn-neko233/config.json`。
   - 运行 `server-打包-linux.cmd`。
   - 部署 `vpn233-server` 到 Linux。
   - 运行：`./vpn233-server -c config.json`

2. **客户端**:
   - 修改 `client-vpn-neko233/config.json` 和 `rules.yaml`。
   - 运行 `client-打包-windows-macos-linux.cmd`。
   - 运行：`vpn233-client -c config.json`
   - **帮助**: 运行 `vpn233-client -h` 查看更多选项。




## 注意事项
- 本项目为简化版实现，主要展示架构和核心逻辑。
- 生产环境下建议使用更成熟的库（如 `utls`）来增强 TLS 指纹伪装。
