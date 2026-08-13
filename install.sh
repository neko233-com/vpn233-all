#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ "${1:-}" == "update" ]]; then
  exec "$ROOT_DIR/update.sh"
fi

DEST_BIN_DIR="${VPN233_BIN_DIR:-$ROOT_DIR/generated}"
WEB_DEST_DIR="${VPN233_WEB_DIR:-$ROOT_DIR/admin-web/dist}"
CONFIG_DEST_DIR="${VPN233_CONFIG_DIR:-$ROOT_DIR/config}"
BIN_SOURCE_DIR="${VPN233_SOURCE_BIN_DIR:-}"

if [[ -z "$BIN_SOURCE_DIR" ]]; then
  for candidate in "$ROOT_DIR/bin" "$ROOT_DIR/generated" "$ROOT_DIR/target/release"; do
    if [[ -x "$candidate/vpn233-provider-server" && -x "$candidate/vpn233-subscribe-server" && -x "$candidate/vpn233-client" ]]; then
      BIN_SOURCE_DIR="$candidate"
      break
    fi
  done
fi

if [[ -z "$BIN_SOURCE_DIR" ]]; then
  if ! command -v cargo >/dev/null 2>&1; then
    echo "No prebuilt Rust binaries found and Cargo is unavailable. Install Rust from https://rustup.rs or provide VPN233_SOURCE_BIN_DIR." >&2
    exit 1
  fi
  cargo build --release --workspace
  BIN_SOURCE_DIR="$ROOT_DIR/target/release"
fi

WEB_SOURCE_DIR=""
if [[ -d "$ROOT_DIR/admin-web/dist" && -f "$ROOT_DIR/admin-web/dist/index.html" ]]; then
  WEB_SOURCE_DIR="$ROOT_DIR/admin-web/dist"
elif [[ -d "$ROOT_DIR/admin-web" && -f "$ROOT_DIR/admin-web/index.html" ]]; then
  WEB_SOURCE_DIR="$ROOT_DIR/admin-web"
elif command -v npm >/dev/null 2>&1; then
  (cd "$ROOT_DIR/admin-web" && npm ci && npm run build)
  WEB_SOURCE_DIR="$ROOT_DIR/admin-web/dist"
else
  echo "No admin web build found and Node.js/npm is unavailable. Provide admin-web/dist or install Node.js." >&2
  exit 1
fi

mkdir -p "$DEST_BIN_DIR"
for binary in vpn233-provider-server vpn233-subscribe-server vpn233-client; do
  install -m 0755 "$BIN_SOURCE_DIR/$binary" "$DEST_BIN_DIR/$binary"
done
if [[ "$WEB_SOURCE_DIR" != "$WEB_DEST_DIR" ]]; then
  mkdir -p "$WEB_DEST_DIR"
  cp -a "$WEB_SOURCE_DIR/." "$WEB_DEST_DIR/"
fi
mkdir -p "$CONFIG_DEST_DIR"
for config in provider-server subscribe-server vpn233-client; do
  destination="$CONFIG_DEST_DIR/$config.yaml"
  if [[ ! -f "$destination" ]]; then
    source="$ROOT_DIR/config/$config.yaml"
    [[ -f "$source" ]] || source="$ROOT_DIR/$config/config.example.yaml"
    if [[ -f "$source" ]]; then
      install -m 0600 "$source" "$destination"
    fi
  fi
done
echo "Installed Rust services in $DEST_BIN_DIR"
echo "Installed admin web in $WEB_DEST_DIR"
echo "Configuration templates are available in $CONFIG_DEST_DIR; existing files were preserved."
