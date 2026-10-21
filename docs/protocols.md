---
layout: page
title: 协议与客户端兼容
---

导入、导出和代理执行是不同能力。本地客户端只负责 HTTP/SOCKS5 入口、直连或 SOCKS5 上游；加密传输、UDP/TUN 与路由使用用户配置的兼容核心。

## 导入

| 输入 | 支持 |
| --- | --- |
| URI / 标准 Base64 / Base64URL | SS、SSR、VMess、VLESS、Trojan、Hysteria2、TUIC、AnyTLS、SOCKS5、HTTP(S) |
| Clash / Mihomo YAML | 上述协议及 Hysteria、WireGuard、Snell、SSH、Mieru，保留节点字段 |
| sing-box JSON | 下表 sing-box 的 9 种出站协议；映射常见 TLS、REALITY、WS、HTTPUpgrade、gRPC、HTTP2 字段 |

`detour`、`dialer-proxy` 链式出站、不支持的 sing-box 传输和没有具体端口的范围设置会拒绝导入。Clash 节点保留字段；sing-box 导入只映射已实现字段，高级 TLS、路由和传输选项不保证完整保留。全局控制器、外部规则集和测速 URL 不随节点导入。跨格式转换后应检查目标客户端配置，不能只凭协议名称判断所有选项兼容。

## 导出

| 格式 ID | 客户端 | 范围 |
| --- | --- | --- |
| `clash`、`mihomo` | Clash / Mihomo | 15 种结构化节点协议 |
| `stash` | Stash | 9 种：SS、VMess、VLESS、Trojan、Hysteria2、TUIC、AnyTLS、SOCKS5、HTTP；输出 Clash YAML，仍需目标版本支持相应字段 |
| `singbox`、`sing-box` | sing-box | SS、VMess、VLESS、Trojan、Hysteria2、TUIC、AnyTLS、SOCKS5、HTTP |
| `v2ray`、`base64` | v2rayN/v2rayNG、Shadowrocket、Hiddify、NekoBox | 10 种 URI 协议的 Base64 订阅；目标客户端可用的协议随版本不同 |
| `links` | URI 客户端 | 10 种 URI 协议，每行一个 |
| `surge` | Surge | SS、VMess、Trojan、SOCKS5、HTTP |
| `quantumultx`、`qx` | Quantumult X | SS、VMess、VLESS、Trojan、SOCKS5、HTTP |
| `loon` | Loon | SS、VMess、VLESS、Trojan、Hysteria2、SOCKS5、HTTP |
| `surfboard` | Surfboard | SS、SSR、VMess、Trojan、SOCKS5、HTTP |

`GET /api/v1/formats` 提供协议列表、扩展名与 Content-Type。协议列表表示格式的实现范围，不代表目标客户端已实测。未知协议不伪装输出，不能表达的节点被筛除；例如文本格式不能表达的 REALITY、gRPC 或 HTTPUpgrade 不降级为普通 TCP。Hysteria v1、WireGuard、SSH、Snell、Mieru 仅支持 Mihomo 结构化节点，不生成私有 URI。响应中的解析、输出和跳过数量见 [API](api.md)。

默认不指定公共 DNS 或第三方测速地址，解析使用系统或目标客户端的默认 DNS；历史 Auto/Fallback 组名保留为手动选择组。用户可在客户端自行设置探测。Provider 的节点部署清单实际生成 **8 种**入站：VLESS、VMess、Trojan、SS、Hysteria2、TUIC、AnyTLS、SOCKS5；与核心库的 15 种订阅兼容能力不同，运行需外部 Mihomo。安装与自签证书边界见 [部署](deployment.md)。

字段参考 [Mihomo 官方配置](https://wiki.metacubex.one/en/config/proxies/) 和 [sing-box 出站配置](https://sing-box.sagernet.org/configuration/outbound/)。使用目标客户端的配置检查确认版本兼容。
