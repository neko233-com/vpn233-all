#!/usr/bin/env bash
# Remove application binaries only; private configuration and data are retained.
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
if [[ $# -gt 0 ]]; then printf 'Usage: bash uninstall.sh (run from the installed directory)\n' >&2; exit 2; fi
if [[ ! -f "$ROOT_DIR/version.txt" || ! -d "$BIN_DIR" ]]; then
  printf 'Refusing uninstall: this is not an installed release directory.\n' >&2; exit 1
fi
bash "$ROOT_DIR/stop.sh"
# shellcheck disable=SC2034 # services.sh reads CONFIG_DIR when checking unit ownership.
CONFIG_DIR="${VPN233_CONFIG_DIR:-${VPN233_SAVED_CONFIG:-$ROOT_DIR/config}}"
if [[ -f "$ROOT_DIR/scripts/lib/services.sh" ]]; then
  # shellcheck source=scripts/lib/services.sh
  . "$ROOT_DIR/scripts/lib/services.sh"
  if vpn233_manage_services uninstall; then :
  else result=$?; [[ "$result" -eq 1 ]] || { printf 'Unable to remove installed services; binaries retained.\n' >&2; exit "$result"; }; fi
fi
for binary in vpn233-provider-server vpn233-subscribe-server vpn233-client; do
  rm -f -- "$BIN_DIR/$binary" "$BIN_DIR/$binary.exe"
done
printf 'Owned services and application binaries removed. Configuration, data, logs and backups retained.\n'
