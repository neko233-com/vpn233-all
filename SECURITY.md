# Security Policy

VPN233 is self-hosted software that handles proxy credentials, subscription
tokens and traffic metadata. We take security reports seriously and we do not
run any telemetry, analytics or third-party callbacks in this project.

## Supported versions

| Version | Supported |
| --- | --- |
| Current main / 2.0.x workspace | Maintained; use a verified formal release for deployment |
| < 2.0 | No (legacy Go implementation, migration reference only) |

## Reporting a vulnerability

Please **do not open a public issue** for security problems.

1. Use GitHub's private vulnerability reporting:
   <https://github.com/neko233-com/vpn233-all/security/advisories/new>
2. Include: affected version/commit, affected component, reproduction steps,
   impact assessment and any suggested fix.
3. If private reporting is unavailable, open a minimal public issue that asks
   the maintainers for a private channel and contains **no exploit details,
   credentials or server addresses**.

We aim to acknowledge reports within 7 days and to publish a fix and advisory
as soon as practical. Please give us a reasonable window before public
disclosure.

## Privacy statement

- The services and the admin web contain **no telemetry, no analytics, no crash
  reporting, no external fonts and no third-party CDN assets**.
- Outbound connections only happen for the features a deployment explicitly
  configures: upstream subscription sources, provider registry/heartbeat, the
  optional client statistics relay, GitHub mirror downloads during install and
  the client's own proxy traffic.
- Target domains are **not recorded by default**. `vpn233-client` keeps daily
  upload/download counters and only stores per-domain counters when
  `record_domains: true` is explicitly configured.
- Credentials, tokens and subscription URLs are never written to service logs.
  The automated E2E suites fail if a known secret appears in a log.
- Runtime configuration and data (`config.yaml`, `data/`, `logs/`, `backups/`)
  are untracked, generated with random credentials, and written with owner-only
  permissions on Unix.

## Deployment hardening checklist

- Change the default `root` / `root` administrator credentials before exposing
  any service to a network. The installers generate random credentials on first
  install; keep them out of shell history and chat logs.
- Put the subscribe server behind TLS (reverse proxy) when it is reachable from
  the internet. Subscription tokens are bearer credentials.
- Keep `provider_registry_token`, `provider_access_token` and subscription
  tokens in the untracked configuration or environment, never in Git.
- Verify release assets against `SHA256SUMS`; installers fail closed on missing
  or mismatched checksums. Prefer a pinned release over "latest".
- Mirror prefixes (`ghproxy.net`, `gh.llkk.cc`, `ghfast.top`, `gh-proxy.com`) are
  third-party proxies. They only accelerate transport; the SHA-256 check is the
  integrity boundary, so never disable it with `VPN233_ALLOW_UNSIGNED=1` in
  production.
- Run the services as an unprivileged user where the platform allows it.
- Back up `data/` before upgrades; `update` keeps configuration and data and
  rolls back automatically on a failed health check, but backups are still your
  responsibility.

## Known limitations (not vulnerabilities)

- The Rust services generate and manage Clash/Mihomo/sing-box configuration and
  proxies, but full TUN/kernel routing and protocol transport execution remain an
  external Mihomo/sing-box boundary. The client only enables that boundary when
  configured and does not claim to bundle a VPN core.
- Legacy code under `legacy/go-v1/` is not built, tested or supported. It is kept
  for migration reference only.
