# VPN233 Rust

VPN233 Rust is a memory-safe rewrite of the VPN233 provider, subscription, and client services.

The workspace contains the three runtime services requested by the project:

- `provider-server`: authenticated provider API and `.sh` / `.ps1` node installers.
- `subscribe-server`: local/remote subscription aggregation, password-protected Clash/Mihomo output, token quotas with calendar/creation-day reset cycles, expiry and renewal APIs, and the embedded admin API.
- `vpn233-client`: local SOCKS5 and HTTP proxy entry points with connection and byte counters.

Shared Clash/Mihomo-compatible models and protocol extension points live in `vpn233-core-lib-rust`. The `admin-web` directory is the Vite/React control room served by `subscribe-server` in production.

## Security defaults

- The default administrator is `root` / `root` for first boot only.
- Set `VPN233_ADMIN_USER` and `VPN233_ADMIN_PASSWORD` or change the YAML values before exposing a service publicly.
- Subscription access requires both a token and its separate password. Passwords are stored as Argon2id hashes.
- Real configuration files, runtime data, generated installers, bundles, and secrets are ignored by Git. Do not commit `config.yaml`, service-unit overrides, SSH material, or exported subscription links.

## Build

Requirements: Rust 1.85 or newer and Node.js 20 or newer for the admin web build.

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release --workspace

cd admin-web
npm ci
npm run build
```

On Windows, `install.ps1` installs a release bundle's prebuilt `bin/` files when available, or builds all Rust services from source. On Linux/macOS, use `install.sh`. Both scripts support `update` to install a verified release bundle or a locally supplied offline bundle. The installers also copy the bundled `admin-web/dist` static files without touching existing configuration or runtime data.

## Local services

Copy the relevant `config.example.yaml` to an untracked `config.yaml` and adjust the ports and data paths.

```bash
VPN233_CONFIG=provider-server/config.yaml \
  ./target/release/vpn233-provider-server

VPN233_CONFIG=subscribe-server/config.yaml \
  ./target/release/vpn233-subscribe-server

VPN233_CONFIG=vpn233-client/config.yaml \
  ./target/release/vpn233-client
```

Default local endpoints:

| Service | Endpoint |
| --- | --- |
| Provider | `http://127.0.0.1:8080/api/v1/health` |
| Subscribe | `http://127.0.0.1:23444/` |
| Client | `http://127.0.0.1:8082/api/v1/health` |

The subscription server exposes `/sub/c?token=...&password=...` for Clash/Mihomo YAML and `/sub/links?token=...&password=...` for raw links. The admin API uses Basic Auth with the configured administrator.

### Access policy lifecycle

Subscription access is intentionally separate from billing, orders, and the account system. Those systems can create or renew a token through the authenticated admin API and attach an optional `other_uid` for external reconciliation.

- `quota_period`: `lifetime`, `calendar_month`, `monthly_from_creation`, `weekly`, or `daily`.
- `duration`: `seven_days`, `one_month`, or `one_year` when creating or renewing a token. An explicit `expires_at` timestamp takes precedence.
- `POST /api/admin/tokens/{token}/renew` renews access and can also change the quota period, quota limit, or `other_uid`.
- `POST /api/admin/tokens/{token}/reset` clears the current period's usage immediately.
- Expired tokens are automatically revoked and marked `expired`; their records remain for audit and later renewal rather than being physically deleted.

When `provider_registry_token` is configured, a provider can self-register with `registry_url`, send heartbeats, and publish its domain/IP/local addresses and node links. Registered providers are merged into subscriptions while enabled and fresh; `/sub/base64` provides a Base64 link feed for clients that expect that format. The registry token and provider heartbeat tokens are never returned by the admin API.

### Traffic analytics

`vpn233-client` persists Asia/Shanghai daily upload/download totals and the target host for each completed HTTP or SOCKS5 connection in its configured `stats_file` (default: `vpn233-client/data/stats.json`). Its authenticated `/api/v1/stats` endpoint exposes the retained history. To show actual proxy traffic and target hosts in the admin console, set `client_stats_url` and `client_stats_user` in the untracked subscribe config, and provide `VPN233_CLIENT_STATS_PASSWORD` through the service environment. If the relay is not configured, the console intentionally shows only subscription-response traffic and an empty domain state.

## Release bundles

Tag a commit with `vX.Y.Z` to run the GitHub Actions release workflow. It builds the provider, subscribe server, client, and admin UI for Linux, Windows, and macOS, then publishes platform bundles containing example configs and the shell/PowerShell installers.

For an offline update, set `VPN233_UPDATE_BUNDLE` to a local bundle path. For remote updates, optionally set `VPN233_UPDATE_URLS` to a `|`-separated list of trusted URLs and `VPN233_UPDATE_SHA256` to the expected SHA-256 digest.

## Mihomo scope

The Rust core library emits compatible Clash/Mihomo configuration and provides a `CustomProtocol` registry for extensions. It is the stable Rust boundary for future protocol work. Mihomo's complete runtime is not currently available as a stable Rust library, so platform-specific TUN, kernel routing, and transport execution remain an explicitly pinned external Mihomo boundary. The client only enables that boundary when configured; it does not claim unimplemented Rust transports are complete.

## Legacy source

The former Go implementation is retained under `legacy/go-v1/` for migration and rollback reference only. It is not built by the workspace, release workflow, or installation scripts.
