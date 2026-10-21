# VPN233

自托管代理节点部署、订阅转换和本地管理后台。后端使用 Rust，界面随包分发，不加载第三方统计、字体或 CDN。

[English](README.md) · [GitHub Docs](https://neko233-com.github.io/vpn233-all/) · [下载](https://github.com/neko233-com/vpn233-all/releases) · [安全说明](SECURITY.md)

## 傻瓜式部署

Linux / macOS：

```bash
(
  set -eu
  setup="$(mktemp "${TMPDIR:-/tmp}/vpn233-install.XXXXXXXX")"
  trap 'rm -f -- "$setup"' EXIT
  curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fL --connect-timeout 5 --max-time 30 'https://ghproxy.net/https://raw.githubusercontent.com/neko233-com/vpn233-all/main/install.sh' -o "$setup" ||
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fL --connect-timeout 5 --max-time 30 'https://raw.githubusercontent.com/neko233-com/vpn233-all/main/install.sh' -o "$setup"
  bash "$setup" --release --mirror 'auto'
)
```

Windows PowerShell：

```powershell
$setup = Join-Path ([IO.Path]::GetTempPath()) ('vpn233-install-' + [guid]::NewGuid().ToString('N') + '.ps1')
$previousTls = [Net.ServicePointManager]::SecurityProtocol
try {
  [Net.ServicePointManager]::SecurityProtocol = $previousTls -bor [Net.SecurityProtocolType]::Tls12
  try { Invoke-WebRequest -UseBasicParsing 'https://ghproxy.net/https://raw.githubusercontent.com/neko233-com/vpn233-all/main/install.ps1' -OutFile $setup -TimeoutSec 30 -ErrorAction Stop }
  catch { Invoke-WebRequest -UseBasicParsing 'https://raw.githubusercontent.com/neko233-com/vpn233-all/main/install.ps1' -OutFile $setup -TimeoutSec 30 -ErrorAction Stop }
  powershell -NoProfile -ExecutionPolicy Bypass -File $setup -Release -Mirror 'auto'
  if ($LASTEXITCODE -ne 0) { throw 'VPN233 installation failed.' }
}
finally {
  try {
    if (Test-Path -LiteralPath $setup) { Remove-Item -LiteralPath $setup -Force }
  }
  finally { [Net.ServicePointManager]::SecurityProtocol = $previousTls }
}
```

命令下载公开安装器，需要时可先检查再执行；对应平台必须已有正式发布。安装器自动回退 GitHub 镜像、校验 SHA-256，默认安装到用户 `.vpn233`。新配置仅监听本机，随机密码只保存在本地私密配置，不打印。升级保留配置和数据。[离线部署、系统服务、更新](docs/deployment.md)。

## 协议和客户端

支持 Clash/Mihomo、Stash、sing-box、v2rayN/v2rayNG、Shadowrocket、Hiddify、NekoBox、Surge、Quantumult X、Loon、Surfboard 格式。URI 导入包含 SS、SSR、VMess、VLESS、Trojan、Hysteria2、TUIC、AnyTLS、SOCKS5、HTTP(S)；Clash YAML 还保留 Hysteria、WireGuard、Snell、SSH、Mieru。动态矩阵由 `/api/v1/formats` 提供。[完整兼容范围](docs/protocols.md)。

本地客户端提供 HTTP/SOCKS5 入口和直连或 SOCKS5 上游转发。加密协议及 TUN 使用用户配置的兼容内核，订阅转换不代表已在 Rust 中实现全部传输协议。

Mihomo 渲染器接收 15 种结构化协议；Stash 与 sing-box 各提供 9 种协议子集。Provider 生成的节点脚本创建 8 种入站。协议矩阵描述已实现的格式转换，具体客户端版本、证书信任和远端连通性仍需验证。

## 超快验证与完整验收

```bash
bash scripts/verify-all.sh --quick
bash scripts/verify-all.sh
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-all.ps1 -Quick
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-all.ps1
```

快速模式跳过发布构建、真实服务 E2E 和联网依赖审计；完整模式与 CI 保留这些发布门槛。缓存及产物复用减少重复编译。[测试与打包](docs/testing.md)。源码构建使用仓库固定 Rust、Node.js 24；Windows 需要 MSVC C++ Build Tools 和 Windows SDK。

验证、打包、隐私与测试夹具统一使用 Rust `vpn233-xtask`，无需安装 Python。GitHub Pages 仅在正式稳定版完整验收成功后更新，文档没有变化时跳过构建和部署。

## 隐私

真实配置、订阅链接、凭据、生成安装器、运行数据和日志不提交。无第三方遥测，生成订阅不默认访问公共测速地址；目的地址统计需要明确启用并仅保存在本地，可选统计中继由用户配置。管理令牌仅保存在当前浏览器会话。[隐私](docs/privacy.md)。手动启动服务前替换首次启动的 `root` / `root`，公网使用 HTTPS。[API](docs/api.md)。
