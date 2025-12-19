# Neko233 VPN

A simple client-server VPN (SOCKS5 over WebSocket) built with Bun, TypeScript, and Vite-Rolldown.

## Structure

- `packages/server`: The VPN server (Linux).
- `packages/client`: The VPN client (Windows/macOS/Linux).

## Development

### Server
```bash
cd packages/server
bun install
bun run dev
```

### Client
```bash
cd packages/client
bun install
bun run dev
```

## Packaging (One-click)

To package both client and server for all platforms:

```bash
bun run package:all
```

The executables will be generated in:
- `packages/server/bin/`
- `packages/client/bin/`

## Features

- **Encryption**: AES-256-GCM encryption for all tunnel traffic.
- **Rule-based Routing**: Client supports bypass rules with fuzzy matching (e.g., `192.168.*.*` or `*.baidu.com`).
- **Web UI**: Manage bypass rules and monitor connection status via a web browser.
- **Cross-platform**: One-click packaging for Windows, Linux, and macOS.

## Configuration

### Client (`packages/client/config.json`)
- `serverUrl`: VPN Server WebSocket URL.
- `encryptionKey`: 32-character key for AES encryption.
- `rules.direct`: List of domains/IPs that should not go through the VPN. Supports `*` wildcard.
- `webPort`: Port for the Web UI (default: 23333).

## Web UI
Access the client management interface at `http://localhost:23333`.
