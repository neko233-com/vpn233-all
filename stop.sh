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
if [[ ! -d "$BIN_DIR" && -d "$ROOT_DIR/target/release" ]]; then BIN_DIR="$ROOT_DIR/target/release"; fi
CONFIG_DIR="${VPN233_CONFIG_DIR:-${VPN233_SAVED_CONFIG:-$ROOT_DIR/config}}"
[[ $# -eq 0 ]] || { printf 'Usage: bash stop.sh\n' >&2; exit 2; }
if [[ -d "$BIN_DIR" ]]; then BIN_DIR="$(cd "$BIN_DIR" && pwd)"; fi
if [[ -f "$ROOT_DIR/scripts/lib/services.sh" ]]; then
  # shellcheck source=scripts/lib/services.sh
  . "$ROOT_DIR/scripts/lib/services.sh"
  if vpn233_manage_services stop; then :
  else result=$?; [[ "$result" -eq 1 ]] || { printf 'Unable to stop installed services.\n' >&2; exit "$result"; }; fi
fi
for pid_file in "$ROOT_DIR/runtime"/*.pid; do
  [[ -f "$pid_file" ]] || continue
  pid="$(cat "$pid_file")"
  if [[ "$pid" =~ ^[0-9]+$ ]] && [[ "$pid" -gt 1 ]]; then
    name="${pid_file##*/}"; name="${name%.pid}"
    binary="vpn233-$name"; [[ "$name" != vpn233-client ]] || binary="$name"
    expected="$BIN_DIR/$binary"
    identity=""
    [[ ! -f "$pid_file.identity" ]] || identity="$(cat "$pid_file.identity")"
    if declare -F vpn233_process_matches >/dev/null 2>&1 && vpn233_process_matches "$pid" "$expected" "$CONFIG_DIR/$name.yaml" "$identity"; then
      kill "$pid" 2>/dev/null || true
      for _ in 1 2 3 4 5 6 7 8 9 10; do
        if ! vpn233_process_matches "$pid" "$expected" "$CONFIG_DIR/$name.yaml" "$identity"; then break; fi
        sleep 0.2
      done
      if vpn233_process_matches "$pid" "$expected" "$CONFIG_DIR/$name.yaml" "$identity"; then printf 'A VPN233 process did not stop; refusing to discard its PID.\n' >&2; exit 1; fi
    fi
  fi
  rm -f "$pid_file"
  rm -f "$pid_file.identity"
done
echo "VPN233 Rust services stopped."
