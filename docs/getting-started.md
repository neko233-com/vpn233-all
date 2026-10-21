---
layout: page
title: 快速开始
---

从 [正式发布](https://github.com/neko233-com/vpn233-all/releases) 选择操作系统和 CPU 对应的包。预编译包无需安装 Rust 或 Node.js。

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

Windows：

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

默认安装到用户 `.vpn233`。从该目录的 `config/subscribe-server.yaml` 读取管理员设置，打开 `http://127.0.0.1:23444/`。若端口冲突或使用了自定义端口，以安装输出和配置为准。勿将配置复制到公共工单。

在后台添加自己的节点或上游订阅，创建账户或订阅令牌，设置独立密码和配额，选择目标客户端格式。生成的订阅 URL 自带访问凭据，必须按密码保管。VPN233 不附带可直接使用的公共节点。

默认只允许本机访问。远程管理使用自己的 HTTPS 反向代理或 SSH 隧道，确认强密码和访问控制后再开放监听。[部署](deployment.md) · [兼容范围](protocols.md)。
