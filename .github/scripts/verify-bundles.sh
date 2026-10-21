#!/usr/bin/env bash
# Safe, non-extracting verifier shared by pre-publication and download checks.
set -Eeuo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
exec python "$ROOT_DIR/scripts/verify-release.py" "${1:?assets directory required}" "${2:?expected version required}"
