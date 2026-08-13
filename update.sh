#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${VPN233_BIN_DIR:-$ROOT_DIR/generated}"
WEB_DIR="${VPN233_WEB_DIR:-$ROOT_DIR/admin-web/dist}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

if [[ -n "${VPN233_UPDATE_BUNDLE:-}" && -f "$VPN233_UPDATE_BUNDLE" ]]; then
  case "$VPN233_UPDATE_BUNDLE" in
    *.zip) cp "$VPN233_UPDATE_BUNDLE" "$TMP_DIR/bundle.zip" ;;
    *) cp "$VPN233_UPDATE_BUNDLE" "$TMP_DIR/bundle.tar.gz" ;;
  esac
else
SOURCES="${VPN233_UPDATE_URLS:-https://github.com/neko233-com/vpn233-all/releases/latest/download/vpn233-rust-linux-x86_64.tar.gz|https://ghproxy.net/https://github.com/neko233-com/vpn233-all/releases/latest/download/vpn233-rust-linux-x86_64.tar.gz|https://mirror.ghproxy.com/https://github.com/neko233-com/vpn233-all/releases/latest/download/vpn233-rust-linux-x86_64.tar.gz}"
  IFS='|' read -r -a URLS <<< "$SOURCES"
  downloaded=0
  for url in "${URLS[@]}"; do
    echo "Trying update source: $url"
    if curl --fail --location --connect-timeout 8 --max-time 120 --silent --show-error "$url" -o "$TMP_DIR/downloaded.bundle"; then
      case "$url" in *.zip) mv "$TMP_DIR/downloaded.bundle" "$TMP_DIR/bundle.zip" ;; *) mv "$TMP_DIR/downloaded.bundle" "$TMP_DIR/bundle.tar.gz" ;; esac
      downloaded=1; break
    fi
  done
  if [[ "$downloaded" -ne 1 ]]; then
    echo "No update source reachable. Use VPN233_UPDATE_BUNDLE for an offline bundle." >&2
    exit 1
  fi
fi

if [[ -z "${VPN233_UPDATE_SHA256:-}" && -z "${VPN233_ALLOW_UNSIGNED_UPDATE:-}" ]]; then
  echo "Refusing remote update without VPN233_UPDATE_SHA256. Set the published SHA-256 or explicitly opt into an unsigned update with VPN233_ALLOW_UNSIGNED_UPDATE=1." >&2
  exit 1
fi

if [[ -n "${VPN233_UPDATE_SHA256:-}" ]]; then
  bundle_path="$(find "$TMP_DIR" -maxdepth 1 -type f -name 'bundle.*' -print -quit)"
  echo "${VPN233_UPDATE_SHA256}  $bundle_path" | sha256sum --check --status
fi
mkdir -p "$TMP_DIR/extract"
bundle_path="$(find "$TMP_DIR" -maxdepth 1 -type f -name 'bundle.*' -print -quit)"
case "$bundle_path" in
  *.zip) unzip -q "$bundle_path" -d "$TMP_DIR/extract" ;;
  *) tar -xzf "$bundle_path" -C "$TMP_DIR/extract" ;;
esac
mkdir -p "$BIN_DIR"
find "$TMP_DIR/extract" -maxdepth 5 -type f -name 'vpn233-*' -exec install -m 0755 {} "$BIN_DIR" \;
web_source="$(find "$TMP_DIR/extract" -type d -path '*/admin-web/dist' -print -quit)"
if [[ -z "$web_source" ]]; then
  web_index="$(find "$TMP_DIR/extract" -type f -path '*/admin-web/index.html' -print -quit)"
  if [[ -n "$web_index" ]]; then
    web_source="$(dirname "$web_index")"
  fi
fi
if [[ -n "$web_source" ]]; then
  mkdir -p "$WEB_DIR"
  cp -a "$web_source/." "$WEB_DIR/"
fi
echo "Update installed into $BIN_DIR"
