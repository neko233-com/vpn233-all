---
layout: page
title: 部署与升级
---

安装器支持各平台正式发布包，升级保留已有配置与运行数据。新配置使用随机凭据并绑定本机，密码只保存在本地配置中。

升级先验证全部输入，再停止旧进程并替换应用。启动或健康检查失败会恢复原有应用文件并返回失败，配置与数据不回写覆盖；按失败提示在本地重新启动原版本。

## 常用命令

```bash
bash install.sh --release --mirror auto
bash install.sh --release --prefix "$HOME/.vpn233" --no-start
bash install.sh --release --dry-run
bash update.sh --mirror auto
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File install.ps1 -Release -Mirror auto
powershell -NoProfile -ExecutionPolicy Bypass -File install.ps1 -Release -Prefix (Join-Path $env:USERPROFILE '.vpn233') -NoStart
powershell -NoProfile -ExecutionPolicy Bypass -File install.ps1 -Release -DryRun
powershell -NoProfile -ExecutionPolicy Bypass -File update.ps1 -Mirror auto
```

`--version` / `-Version` 固定版本；`--mirror off` / `-Mirror off` 强制直连；`--no-start` / `-NoStart` 仅安装。更新入口复用同一安装和校验路径。缺少库时单文件安装器下载公开的镜像帮助库，dry-run 不下载或修改文件。

## 离线安装

从同一正式发布下载平台归档及 `SHA256SUMS`，一起传到目标机器，保持文件名不变。也可用 `--sha256` / `-Sha256` 传入独立核实的 64 位哈希。

```bash
bash install.sh --bundle ./vpn233-rust-linux-x86_64.tar.gz --no-start
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File install.ps1 -Bundle ./vpn233-rust-windows-x86_64.zip -NoStart
```

缺少校验和、哈希错误、归档越界或缺少必要文件时拒绝安装。解压目录不能替代已校验归档。显式 `VPN233_ALLOW_UNSIGNED=1` 仅允许缺失哈希，不能忽略哈希不匹配。镜像及校验和来自同一分发链时，SHA-256检查完整性；高要求环境从独立可信渠道核实哈希。

## 目录和服务

| 内容 | 用户安装默认位置 |
| --- | --- |
| 二进制 | `~/.vpn233/bin` |
| 前端 | `~/.vpn233/admin-web/dist` |
| 私密配置 | `~/.vpn233/config` |
| 数据 | `~/.vpn233/data` |
| 日志和备份 | `~/.vpn233/logs`、`~/.vpn233/backups` |

Windows 的用户根目录为 `%USERPROFILE%`。`VPN233_BIN_DIR`、`VPN233_WEB_DIR`、`VPN233_CONFIG_DIR`、`VPN233_DATA_DIR` 覆盖目录，`VPN233_WEB_DIR` 直接指向 `dist`。进程模式在安装目录运行 `start.sh` / `start.ps1` 和 `stop.sh` / `stop.ps1`；服务模式使用对应系统服务管理器。

Windows 发布程序静态链接 Visual C++ 运行库，无需另装 VC++ Redistributable；打包时检查三个 EXE 的实际导入表，发现额外 VC 运行库依赖会拒绝发布。

Linux `--system` 需要管理员权限，使用 `/opt/vpn233`、`/etc/vpn233`、`/var/lib/vpn233`，可用时注册 systemd；用户模式可用时使用用户 systemd。macOS 使用 launchd。Windows `-System` 使用 `%ProgramData%/vpn233` 并要求管理员权限；当前运行后台进程，不注册 Windows 服务或开机自启动。

安装器把目录和运行范围保存到仅本机可读的 `.vpn233-layout`，不包含密码。更新、启动、停止和卸载读取该记录，因此系统模式和外置配置目录不会在更新时切换到新默认目录；显式环境变量仍可覆盖。文件损坏或旧目录归属无法确定时中止，不能猜测并重建生产配置。此文件不能发布。

卸载自动停用并移除属于当前安装的 systemd/launchd 单元与二进制，保留私密配置、数据、日志和备份。源码模式运行不带参数的 `install.sh` / `install.ps1`，需要固定 Rust、Node.js 24 和平台构建工具。公网监听前设置强管理员密码及 HTTPS，不能使用首次启动的默认口令。

## Provider 生成的代理节点

Provider 的 `.sh` / `.ps1` 输出用于安装代理节点，与上述三个 VPN233 服务的安装入口不同。它部署外部 Mihomo 并生成 8 种入站配置。默认固定 Mihomo `v1.19.32`，内嵌官方 GitHub Release 的 Linux、macOS、Windows x64 / arm64 共六个平台资产 SHA-256；未知版本或自定义资产需要独立配置 `mihomo_sha256`，默认拒绝缺失或错误的校验和。ARM64 使用其实际发布的资源名，不添加 x64 的 `compatible` 后缀。

HTTPS 入站需要证书和私钥。可使用本地 `VPN233_TLS_CERT`、`VPN233_TLS_KEY` 指定文件；缺少文件时尝试使用 OpenSSL 生成自签证书。Windows 也会查找 Git for Windows 附带的 OpenSSL。没有可用证书或生成失败时退出，启动前先运行 Mihomo 配置检查。Hysteria2、TUIC 需要目标端口的 UDP 放行。

自签证书不自动获得客户端信任。VLESS / Trojan 等客户端必须信任该证书，或使用由目标客户端信任的证书与正确服务名；不能把配置检查通过视为跨机 TLS、QUIC 或所有协议已连通。生成的脚本与链接含节点凭据，仅在本地私密保存。

这两个生成脚本面向新节点：若目标 `mihomo/config.yaml` 已存在，脚本在下载、更换应用、生成证书或写入配置之前明确退出，保留原文件，也不输出本次新凭据的节点链接。已有节点应保留原配置手动维护，或指定新的 `VPN233_NODE_ROOT` 创建独立节点；不提供自动覆盖旧配置的开关。

Unix 上，自定义节点目录或其最近的现存父目录可写时，脚本以当前用户创建节点文件，即使系统已安装 `sudo` 也不会更改归属。系统目录、系统服务和防火墙操作按需提权；生成的配置目录权限为 `0700`，配置与证书为 `0600`。
