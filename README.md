# VPN233

Self-hosted proxy deployment, subscription conversion and a local admin console. Rust services and a bundled React UI run without third-party analytics, web fonts or CDNs.

[中文](README.zh-CN.md) · [Documentation](https://neko233-com.github.io/vpn233-all/) · [Releases](https://github.com/neko233-com/vpn233-all/releases) · [Security](SECURITY.md)

## Install

Linux / macOS:

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

Windows PowerShell:

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
  if (Test-Path -LiteralPath $setup) { Remove-Item -LiteralPath $setup -Force }
  [Net.ServicePointManager]::SecurityProtocol = $previousTls
}
```

These commands download the public installer; inspect it before execution when required by your environment. A release asset must exist for your platform. The installer uses GitHub mirror fallback, checks SHA-256 before installation, creates private random credentials, binds new services to loopback and preserves existing configuration/data. Credentials stay in local configuration and are never printed. The default root is the current user's `.vpn233` directory. [Offline installation, upgrades and services](docs/deployment.md).

## Components and compatibility

| Component | Purpose | Default local endpoint |
| --- | --- | --- |
| `provider-server` | Node manifests and generated `.sh` / `.ps1` installers | `http://127.0.0.1:8080/api/v1/health` |
| `subscribe-server` | Subscription aggregation, tokens, quotas and admin UI | `http://127.0.0.1:23444/` |
| `vpn233-client` | Local HTTP/SOCKS5 proxy and local traffic statistics | `http://127.0.0.1:8082/api/v1/health` |
| `vpn233-core-lib-rust` | Shared parsers and subscription formats | Library |

Exports include Clash/Mihomo, Stash, sing-box, raw/Base64 links for v2rayN/v2rayNG, Shadowrocket, Hiddify and NekoBox, plus Surge, Quantumult X, Loon and Surfboard. URI imports cover SS, SSR, VMess, VLESS, Trojan, Hysteria2, TUIC, AnyTLS, SOCKS5 and HTTP(S); structured Clash imports also preserve Hysteria, WireGuard, Snell, SSH and Mieru. Each output has its own limits: query `/api/v1/formats` for the exact matrix. [Protocol compatibility](docs/protocols.md).

The local client executes direct or configured SOCKS5 upstream connections. Encrypted transports and TUN require a separately configured compatible core such as Mihomo; subscription conversion does not implement those transports in Rust.

The Mihomo renderer accepts 15 structured protocols; Stash and sing-box each expose a 9-protocol subset. Provider-generated node scripts create 8 inbound types. Protocol listings describe implemented conversions, while actual client versions, TLS trust and remote connectivity still require validation.

## Fast and full verification

Use the pinned Rust toolchain, Node.js 24 and platform build tools.

```bash
bash scripts/verify-all.sh --quick
bash scripts/verify-all.sh
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-all.ps1 -Quick
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-all.ps1
```

Quick verification skips release compilation, real service E2E and network dependency audits. Full verification and CI cover those gates plus installer tests, UI output, privacy, documentation and package checksums. [Testing and packaging](docs/testing.md).

Verification, release tools and fixtures use the Rust `vpn233-xtask` workspace crate. No Python installation is required. GitHub Pages updates only after a fully successful stable release and skips unchanged documentation.

## Privacy

Live configuration, subscription credentials, runtime data, logs and generated installers must stay out of Git and public bundles. Services contact only configured upstream subscriptions, provider registry and optional statistics relay; generated subscriptions have no default third-party latency probe. The admin UI uses same-origin assets and session-scoped authentication. Local traffic statistics may contain destination hosts when explicitly enabled; protect them as private data. [Privacy](docs/privacy.md).

The source-level default `root` / `root` is for first boot only. Set `VPN233_ADMIN_PASSWORD` before exposing manually started services; use HTTPS and access controls. [API and configuration](docs/api.md).

Legacy Go code is migration reference and excluded from builds/releases. [Contributing](CONTRIBUTING.md) · [License](LICENSE).
