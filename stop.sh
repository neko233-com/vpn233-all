#!/usr/bin/env bash
set -Eeuo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for pid_file in "$ROOT_DIR"/*.pid; do
  [[ -f "$pid_file" ]] || continue
  pid="$(cat "$pid_file")"
  kill "$pid" 2>/dev/null || true
  rm -f "$pid_file"
done
echo "VPN233 Rust services stopped."
