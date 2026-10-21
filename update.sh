#!/usr/bin/env bash
# Update through the same fail-closed installer used for first deployment.
set -Eeuo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARGS=()
CHECK=0
HAS_BUNDLE=0
DRY_RUN=0
MIRROR_VALUE=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) CHECK=1 ;;
    --bundle|--bundle=*|--offline-bundle|--offline-bundle=*) HAS_BUNDLE=1; ARGS+=("$1") ;;
    --dry-run) DRY_RUN=1; ARGS+=("$1") ;;
    --mirror)
      [[ $# -ge 2 ]] || { printf '%s\n' '--mirror requires a value' >&2; exit 1; }
      MIRROR_VALUE="$2"; ARGS+=("$1" "$2"); shift ;;
    --mirror=*) MIRROR_VALUE="${1#*=}"; ARGS+=("$1") ;;
    *) ARGS+=("$1") ;;
  esac
  shift
done
if [[ "$CHECK" -eq 1 ]]; then
  if [[ "$DRY_RUN" -eq 1 ]]; then printf 'DRY RUN: would check the latest verified release version\n'; exit 0; fi
  # shellcheck source=scripts/lib/mirrors.sh
  . "$ROOT_DIR/scripts/lib/mirrors.sh"
  [[ -z "$MIRROR_VALUE" ]] || vpn233_mirror_set_mode "$MIRROR_VALUE"
  TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/vpn233-update-check.XXXXXX")"
  trap 'rm -rf "$TMP_DIR"' EXIT
  vpn233_download_release_asset version.txt "$TMP_DIR/version.txt" '' 1
  LATEST="$(vpn233_normalize_version "$(tr -d '\r\n' < "$TMP_DIR/version.txt")")"
  CURRENT="unknown"
  [[ ! -f "$ROOT_DIR/version.txt" ]] || CURRENT="$(tr -d '\r\n' < "$ROOT_DIR/version.txt")"
  printf 'Installed: %s\nLatest published: %s\n' "$CURRENT" "$LATEST"
  exit 0
fi
if [[ "$HAS_BUNDLE" -eq 0 && -n "${VPN233_UPDATE_BUNDLE:-}" ]]; then ARGS+=(--bundle "$VPN233_UPDATE_BUNDLE"); fi
if [[ -n "${VPN233_UPDATE_SHA256:-}" ]]; then ARGS+=(--sha256 "$VPN233_UPDATE_SHA256"); fi
if [[ "${VPN233_ALLOW_UNSIGNED_UPDATE:-0}" == 1 ]]; then export VPN233_ALLOW_UNSIGNED=1; fi
if [[ -n "${VPN233_UPDATE_URLS:-}" ]]; then
  printf 'VPN233_UPDATE_URLS is no longer supported; use --mirror or VPN233_GH_MIRRORS and verified releases.\n' >&2
  exit 1
fi
exec bash "$ROOT_DIR/install.sh" --release --prefix "$ROOT_DIR" "${ARGS[@]}"
