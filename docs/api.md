---
layout: page
title: API 与访问策略
---

管理认证与订阅访问独立。不要将完整订阅 URL、密码或生成的节点脚本放到公共日志中。

| 服务 | 路由 | 用途 |
| --- | --- | --- |
| Provider | `GET /api/v1/health` | 健康状态 |
| Provider | `/api/v1/protocols`、`/api/v1/formats` | 能力 |
| Provider | `POST /api/v1/login`、`POST /api/v1/logout` | 管理会话 |
| Provider | `/api/v1/generate`、`/api/v1/generate.sh`、`/api/v1/generate.ps1` | 节点清单和私密脚本 |
| Provider | `/api/v1/nodes` | 使用独立 feed token 的节点源 |
| Subscribe | `/api/v1/formats` | 动态协议和格式矩阵 |
| Subscribe | `/api/status`、`POST /api/auth/login` | 状态、登录 |
| Subscribe | `/api/admin/tokens` | 创建和列出令牌 |
| Subscribe | `POST /api/admin/tokens/{token}/renew`、`/reset` | 续期、重置配额 |
| Subscribe | `/api/admin/accounts`、`/api/admin/groups` | 账户、分组 |
| Subscribe | `/api/admin/providers`、`/api/admin/traffic` | 提供者、统计 |
| Client | `/api/v1/health`、`/api/v1/stats` | 健康状态和认证统计 |
| Client | `GET /api/v1/config` | Basic 认证后的配置视图，不返回密码、订阅 URL 或上游地址 |
| Client | `GET /proxy.pac` | 本地代理自动配置脚本 |

`/sub/c` 为 Clash，`/sub/singbox`、`/sub/v2ray`、`/sub/links` 等选择格式；`/sub?format=<id>` 显式选择。保留 `/sub/base64` 和 `/sub/c/{name}`。订阅需要有效令牌及独立密码或支持的访问凭据。

| 订阅响应头 | 含义 |
| --- | --- |
| `X-VPN233-Format` | 实际输出的格式 ID |
| `X-VPN233-Nodes` | 去重后成功解析的节点总数，包含目标格式不支持的节点 |
| `X-VPN233-Rendered` | 通过目标格式协议与传输筛选的节点数 |
| `X-VPN233-Skipped` | 解析失败的条目数与格式不支持的节点数之和 |

`Rendered` 是格式筛选结果，不代表远端节点健康或连接验证通过。解析错误计入 `Skipped` 而不计入 `Nodes`，因此不能简单用 `Nodes - Skipped` 计算输出数。

`quota_period` 支持 `lifetime`、`calendar_month`、`monthly_from_creation`、`weekly`、`daily`。`duration` 支持 `seven_days`、`one_month`、`one_year`，显式 `expires_at` 优先；`other_uid` 可与自己的计费系统对接。过期记录保留以支持续期。订阅响应流量与真实代理流量是不同指标。

提供者注册和心跳仅访问配置的 `registry_url`。客户端统计中继需配置 `client_stats_url`、用户名和私密密码；未配置时不伪造代理流量。生产配置使用未跟踪 YAML 或环境变量，管理员覆盖项为 `VPN233_ADMIN_USER`、`VPN233_ADMIN_PASSWORD`，`VPN233_CONFIG` 指定配置路径。源码模板是各组件 `config.example.yaml`，发布包模板位于 `examples/`。

本地 Client 使用独立的 `VPN233_CLIENT_*` 环境变量，例如 `VPN233_CLIENT_CONFIG`、`VPN233_CLIENT_ADMIN_PASSWORD`、`VPN233_CLIENT_UPSTREAM_MODE`、`VPN233_CLIENT_UPSTREAM_SOCKS5`。`upstream_mode` 为 `direct`、`socks5` 或 `auto`：只有 `socks5` 或配置了上游的 `auto` 才转发至上游；它不自动执行 `subscription_url` 中的加密协议。SOCKS5/HTTP 入口可分别配置 `socks_user` / `socks_password`、`http_user` / `http_password`，每组必须同时提供。

`record_domains` 默认 `false`，可通过 `VPN233_CLIENT_RECORD_DOMAINS=true` 明确启用；未启用时统计仅包含聚合字节数和连接数。`/api/v1/stats` 与 `/api/v1/config` 均要求 Basic 认证，并返回 `Cache-Control: no-store, private`。
