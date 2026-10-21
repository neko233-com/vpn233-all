# VPN233 Admin Web

Single-page operations console for the VPN233 subscribe server. React 19 + Vite 8,
no UI framework, no external fonts/CDNs/telemetry. Everything in the browser is
bundled locally, and the production build is checked for external resource
references before it can ship.

## Requirements

- Node.js 20.19+ or 22.12+ (Vite 8 requirement; CI uses Node 24)
- npm 10+

## Install and develop

```sh
cd admin-web
npm ci                 # reproducible install from package-lock.json
npm run dev            # Vite dev server on http://127.0.0.1:4173
```

Development environment variables (optional):

| Variable | Default | Purpose |
| --- | --- | --- |
| `VITE_API_BASE_URL` | `http://127.0.0.1:23444` in dev, same origin in production | Subscribe-server API base |
| `VITE_PUBLIC_SUBSCRIPTION_ORIGIN` | current browser origin | Absolute origin used when generating subscription links |

The dev server gets a relaxed CSP (inline react-refresh preamble + HMR WebSocket)
from `vite.config.js`; the production build keeps the strict policy from
`index.html`.

## Build and verify

```sh
npm run build     # production bundle in dist/
npm run check     # verify dist/ (privacy + policy + bundle budget)
npm run verify    # build && check — the release gate
```

`scripts/check-dist.mjs` fails the build when any of the following is true:

- `dist/index.html` is missing, has no React root, or lost the hardened metas
  (`Content-Security-Policy` with `connect-src 'self' http://127.0.0.1:*
  http://localhost:*`, and `referrer: no-referrer`).
- `dist/assets/` has fewer than two files, or no JS/CSS bundle exists.
- Source maps (`.map`), inline scripts, inline event handlers, or
  `sourceMappingURL` comments are present.
- `index.html` references a non-relative (external or protocol-relative)
  resource, or references a missing file.
- Any `.js`/`.css`/`.html` file contains an external resource reference. The only
  allowed URL strings are documented library identifiers (React/Redux/Immer error
  documentation URLs and W3C XML namespaces) that are emitted by dependencies and
  never fetched, plus the two local dev origins inside the CSP meta.
- The main JS bundle exceeds 900 KB raw. Raw and gzip sizes for the main bundle,
  styles, and the whole `dist/` are reported on success.

The console builds external links (GitHub repositories, release downloads, mirror
prefixes, installer raw URLs) from fragments at runtime, so the bundle embeds no
external URL literal. Selecting a mirror only rewrites the link it renders or
copies; nothing is fetched by the console itself.

## Features

- **Overview / providers / nodes** — service status, provider registry, usage
  allocation, node inventory.
- **Access tokens** — policy creation, quotas, periods, renewals, reset, disable,
  delete. Tokens are masked by default (first 6 + last 4 characters) with a
  per-row show/hide control; copy actions always use the full value.
- **Accounts and links** — accounts, multiple scoped subscription links per
  account, password rotation, groups.
- Subscription format center — reads `GET /api/v1/formats` (public endpoint) and renders
  the nine subscription formats (`clash`, `singbox`, `v2ray`, `links`, `surge`,
  `quantumultx`, `loon`, `surfboard`, `stash`). When the endpoint is unavailable
  (for example an older core that returns 404) the console falls back to a
  built-in list. After choosing an enabled access token it issues an access token
  and builds one subscription URL per format:

  ```
  {origin}/sub/{id}?token={token}&access_token={access_token}&filename=vpn233-{id}
  ```

  Each card exposes copy-link, copy-curl, and a locally generated QR code
  (`qrcode` → data URL; the subscription URL never leaves the browser).
- **Protocol × format capability matrix** — rendered from the `protocols` and
  per-format `protocols` fields of `/api/v1/formats`.
- **Client import matrix** — official clients per platform (Windows, macOS,
  Android, iOS, Linux, router) with repository/store links, recommended format,
  one-line import steps, and a "get subscription link" shortcut. GitHub release
  downloads honour a mirror prefix (direct, `ghproxy.net`, `gh.llkk.cc`,
  `ghfast.top`, or `gh-proxy.com`); the choice is persisted in `localStorage`
  under `vpn233-gh-mirror`. Third-party clients are not bundled by this project.
- **Deployments** — one-command install blocks for PowerShell
  (`install.ps1 -Release -Yes`) and Linux/macOS (`install.sh --release --yes`) from
  `raw.githubusercontent.com/neko233-com/vpn233-all/main/`, mirror-aware, with
  checksum/systemd/scheduled-task notes and the version reported by
  `GET /api/status`.
- **Traffic** — daily usage, destination domains, per-policy resets.

## Privacy and security notes

- The admin JWT is stored in `sessionStorage`, never in `localStorage`. A legacy
  `vpn233-admin-jwt` value found in `localStorage` is migrated once and removed;
  logout clears both stores.
- `index.html` sets `referrer: no-referrer` and a strict CSP
  (`default-src 'self'`, inline styles only, images from `self`/`data:`, same-origin
  and local-dev API connections, `frame-ancestors 'none'`).
- No external fonts, CDNs, analytics, or telemetry. No real credentials, tokens, or
  server addresses live in the sources or the build.
- QR codes are generated in the browser; subscription URLs and passwords are never
  sent anywhere except the configured subscribe server.
- Example values in the UI (`vpn233`, `subscriber-001`, `example.invalid`) are
  placeholders only.

## API endpoints used

`/api/auth/login`, `/api/status`, `/api/nodes`, `/api/v1/formats`,
`/api/admin/tokens`, `/api/admin/tokens/{token}` (+ `/link`, `/reset`, `/renew`),
`/api/admin/accounts`, `/api/admin/accounts/{id}` (+ `/links`),
`/api/admin/groups`, `/api/admin/providers`, `/api/admin/traffic`,
and the read-only subscription routes under `/sub/*` (one route per format id,
for example `/sub/clash` and `/sub/v2ray`).

If `VITE_API_BASE_URL` is built with an absolute cross-origin URL, extend the
`connect-src` of the CSP meta in `index.html` to that exact origin; the default
production build talks to the same origin only.

## Layout

The console is responsive down to 375 px: the sidebar becomes a drawer, metric
grids collapse, tables scroll horizontally, and the format/deploy/client sections
stack into a single column.
