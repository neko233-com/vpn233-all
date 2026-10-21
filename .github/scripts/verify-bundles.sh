#!/usr/bin/env bash
# Safe, non-extracting verifier shared by pre-publication and download checks.
set -Eeuo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"
xtask="${VPN233_XTASK_BIN:-}"
if [ -z "$xtask" ]; then xtask="$(bash scripts/xtask.sh --path)"; fi
exec "$xtask" verify-release "${1:?assets directory required}" "${2:?expected version required}"
