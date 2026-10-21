#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "$ROOT_DIR/scripts/lib/layout.sh" ]]; then
  # shellcheck source=scripts/lib/layout.sh
  . "$ROOT_DIR/scripts/lib/layout.sh"
  if vpn233_layout_load "$ROOT_DIR"; then :
  else result=$?; [[ "$result" -eq 1 ]] || { printf 'Invalid private runtime layout.\n' >&2; exit 1; };
    if vpn233_layout_legacy "$ROOT_DIR" "${VPN233_BIN_DIR:-$ROOT_DIR/bin}"; then :; else result=$?; [[ "$result" -eq 1 || -n "${VPN233_CONFIG_DIR:-}" ]] || exit 1; fi
  fi
elif [[ -e "$ROOT_DIR/.vpn233-layout" ]]; then printf 'Missing layout parser.\n' >&2; exit 1; fi
BIN_DIR="${VPN233_BIN_DIR:-${VPN233_SAVED_BIN:-$ROOT_DIR/bin}}"
if [[ ! -d "$BIN_DIR" && -d "$ROOT_DIR/generated" ]]; then BIN_DIR="$ROOT_DIR/generated"; fi
if [[ ! -x "$BIN_DIR/vpn233-provider-server" && -d "$ROOT_DIR/target/release" ]]; then
  BIN_DIR="$ROOT_DIR/target/release"
fi
CONFIG_DIR="${VPN233_CONFIG_DIR:-${VPN233_SAVED_CONFIG:-$ROOT_DIR/config}}"
[[ $# -eq 0 ]] || { printf 'Usage: bash start.sh\n' >&2; exit 2; }
BIN_DIR="$(cd "$BIN_DIR" && pwd)"
if [[ -f "$ROOT_DIR/scripts/lib/services.sh" ]]; then
  # shellcheck source=scripts/lib/services.sh
  . "$ROOT_DIR/scripts/lib/services.sh"
  if vpn233_manage_services start; then printf 'VPN233 services started.\n'; exit 0
  else result=$?; [[ "$result" -eq 1 ]] || { printf 'Unable to start installed services.\n' >&2; exit "$result"; }; fi
fi
umask 077
mkdir -p "$CONFIG_DIR"
mkdir -p "$ROOT_DIR/runtime" "$ROOT_DIR/logs"
chmod 0700 "$ROOT_DIR/runtime" "$ROOT_DIR/logs"

for binary in vpn233-provider-server vpn233-subscribe-server vpn233-client; do
  if [[ ! -x "$BIN_DIR/$binary" ]]; then
    echo "Missing Rust binary: $binary. Run ./install.sh first." >&2
    exit 1
  fi
done
for config in provider-server subscribe-server vpn233-client; do
  [[ -f "$CONFIG_DIR/$config.yaml" ]] || { printf 'Missing configuration for %s. Run install.sh first.\n' "$config" >&2; exit 1; }
done
cd "$ROOT_DIR"
for name in provider-server subscribe-server vpn233-client; do
  binary="vpn233-$name"
  [[ "$name" != vpn233-client ]] || binary="$name"
  pid_file="$ROOT_DIR/runtime/$name.pid"
  if [[ -f "$pid_file" ]]; then
    process_id="$(cat "$pid_file")"
    if [[ "$process_id" =~ ^[0-9]+$ ]] && kill -0 "$process_id" 2>/dev/null; then
      identity=""
      [[ ! -f "$pid_file.identity" ]] || identity="$(cat "$pid_file.identity")"
      if declare -F vpn233_process_matches >/dev/null 2>&1 && vpn233_process_matches "$process_id" "$BIN_DIR/$binary" "$CONFIG_DIR/$name.yaml" "$identity"; then continue; fi
    fi
  fi
  nohup "$BIN_DIR/$binary" --config "$CONFIG_DIR/$name.yaml" >"$ROOT_DIR/logs/$name.log" 2>&1 &
  printf '%s\n' "$!" > "$pid_file"
  ps -p "$(cat "$pid_file")" -o lstart= > "$pid_file.identity" 2>/dev/null || true
  sleep 0.1
  kill -0 "$(cat "$pid_file")" 2>/dev/null || { printf 'A VPN233 process exited during startup. Inspect private logs locally.\n' >&2; exit 1; }
done

echo "VPN233 Rust services started."
