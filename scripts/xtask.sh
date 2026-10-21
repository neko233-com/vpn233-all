#!/usr/bin/env bash
# Build the native development tool once, apart from product compilation flags.
set -Eeuo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
target_dir="$ROOT_DIR/target/xtask-bootstrap"
suffix=''
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) suffix='.exe' ;; esac
(
  unset CARGO_BUILD_TARGET
  CARGO_ENCODED_RUSTFLAGS='' RUSTFLAGS='' \
    cargo build --locked --quiet -p vpn233-xtask --target-dir "$target_dir"
)
tool="$target_dir/debug/vpn233-xtask$suffix"
[[ -f "$tool" ]] || { printf 'Rust development tool was not built.\n' >&2; exit 1; }
if [[ "${1:-}" == --path ]]; then
  [[ "$#" -eq 1 ]] || { printf -- '--path takes no additional arguments.\n' >&2; exit 2; }
  if [[ "$suffix" == '.exe' ]]; then cygpath -m "$tool"; else printf '%s\n' "$tool"; fi
  exit 0
fi
exec "$tool" "$@"
