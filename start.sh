#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${VPN233_BIN_DIR:-$ROOT_DIR/generated}"
if [[ ! -x "$BIN_DIR/vpn233-provider-server" && -d "$ROOT_DIR/target/release" ]]; then
  BIN_DIR="$ROOT_DIR/target/release"
fi
CONFIG_DIR="${VPN233_CONFIG_DIR:-$ROOT_DIR/config}"
mkdir -p "$CONFIG_DIR"

for binary in vpn233-provider-server vpn233-subscribe-server vpn233-client; do
  if [[ ! -x "$BIN_DIR/$binary" ]]; then
    echo "Missing Rust binary: $binary. Run ./install.sh first." >&2
    exit 1
  fi
done

"$BIN_DIR/vpn233-provider-server" --config "${VPN233_PROVIDER_CONFIG:-$CONFIG_DIR/provider-server.yaml}" >"$ROOT_DIR/provider-server.log" 2>&1 &
echo $! >"$ROOT_DIR/provider-server.pid"
"$BIN_DIR/vpn233-subscribe-server" --config "${VPN233_SUBSCRIBE_CONFIG:-$CONFIG_DIR/subscribe-server.yaml}" >"$ROOT_DIR/subscribe-server.log" 2>&1 &
echo $! >"$ROOT_DIR/subscribe-server.pid"
"$BIN_DIR/vpn233-client" --config "${VPN233_CLIENT_CONFIG:-$CONFIG_DIR/vpn233-client.yaml}" >"$ROOT_DIR/vpn233-client.log" 2>&1 &
echo $! >"$ROOT_DIR/vpn233-client.pid"

echo "VPN233 Rust services started."
