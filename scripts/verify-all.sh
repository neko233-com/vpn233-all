#!/usr/bin/env bash
# Quick omits release builds, E2E and network audits; never a release sign-off.
set -Eeuo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
quick=0; no_quick_cache=0; skip_rust=0; skip_web=0; skip_scripts=0; skip_docs=0; skip_e2e=0; skip_audit=0
for option in "$@"; do
  case "$option" in
    --quick) quick=1 ;; --no-quick-cache) no_quick_cache=1 ;; --skip-rust) skip_rust=1 ;; --skip-web) skip_web=1 ;;
    --skip-scripts) skip_scripts=1 ;; --skip-docs) skip_docs=1 ;; --skip-e2e) skip_e2e=1 ;; --skip-audit) skip_audit=1 ;;
    -h|--help) echo 'verify-all.sh [--quick] [--no-quick-cache] [--skip-rust] [--skip-web] [--skip-scripts] [--skip-docs] [--skip-e2e] [--skip-audit]'; exit 0 ;;
    *) echo "Unknown option: $option" >&2; exit 2 ;;
  esac
done
started=$SECONDS
checks=0
step() { printf '\n==> %s\n' "$*"; checks=$((checks + 1)); "$@"; }
require() { command -v "$1" >/dev/null || { echo "Required tool missing: $1" >&2; exit 1; }; }
require cargo
xtask="$(bash scripts/xtask.sh --path)"
export VPN233_XTASK_BIN="$xtask"
candidates() { "$xtask" candidates "$1"; }
release_dir="$("$xtask" release-dir)"
cache_ticket=''; cache_finished=0; cache_engine="$(command -v bash)"
cleanup_quick_cache() {
  local status=$?
  if [ "$quick" -eq 1 ] && [ "$cache_finished" -eq 0 ]; then
    if [ -n "$cache_ticket" ]; then
      "$xtask" quick-cache invalidate --flavor bash --engine "$cache_engine" --ticket "$cache_ticket" >/dev/null 2>&1 || true
    else
      "$xtask" quick-cache clear --flavor bash --engine "$cache_engine" >/dev/null 2>&1 || true
    fi
  fi
  return "$status"
}
trap cleanup_quick_cache EXIT
export CARGO_ENCODED_RUSTFLAGS
CARGO_ENCODED_RUSTFLAGS="$("$xtask" build-environment)"
step "$xtask" version
step "$xtask" privacy
step git diff --check
if [ "$quick" -eq 0 ]; then
  step "$xtask" privacy --history
  step gitleaks git --config .gitleaks.toml --redact --no-banner --exit-code 1
else
  step "$xtask" privacy --history-only
fi
if [ "$quick" -eq 1 ] && [ "$((no_quick_cache + skip_rust + skip_web + skip_scripts + skip_docs + skip_e2e + skip_audit))" -eq 0 ]; then
  if cache_result="$("$xtask" quick-cache probe --flavor bash --engine "$cache_engine")"; then
    read -r cache_hit cache_ticket cache_message < <(printf '%s' "$cache_result" | "$xtask" cache-lines)
    printf '%s\n' "$cache_message"
    if [ "$cache_hit" -eq 1 ]; then
      cache_finished=1
      printf '\nQUICK CACHED (not release validation): live gates passed in %ss\n' "$((SECONDS - started))"
      exit 0
    fi
    if [ "$cache_ticket" = '-' ]; then cache_ticket=''; fi
  else
    echo 'Quick cache unavailable; real checks required'
  fi
fi
if [ "$skip_rust" -eq 0 ]; then
  step cargo fmt --all -- --check
  step cargo clippy --locked --workspace --all-targets -- -D warnings
  step cargo test --locked --workspace --no-fail-fast
  if [ "$quick" -eq 0 ]; then step cargo build --locked --release --workspace --exclude vpn233-xtask; fi
fi
if [ "$skip_web" -eq 0 ]; then
  lock_hash="$("$xtask" sha256 admin-web/package-lock.json)"
  marker='admin-web/node_modules/.vpn233-lock.sha256'
  if [ "$quick" -eq 0 ] || [ ! -f "$marker" ] || [ "$(tr -d '\r\n' < "$marker")" != "$lock_hash" ]; then
    step npm ci --prefix admin-web --prefer-offline --no-audit
    printf '%s\n' "$lock_hash" > "$marker"
  fi
  step npm run build --prefix admin-web
  step npm run check --prefix admin-web
fi
if [ "$skip_scripts" -eq 0 ]; then
  require shellcheck; require actionlint; require pwsh
  step actionlint -color
  while IFS= read -r -d '' script; do
    [ -f "$script" ] || continue
    step bash -n "$script"
    step shellcheck -S warning "$script"
  done < <(candidates '.sh')
  step bash scripts/tests/test-mirrors.sh
  step bash scripts/tests/test-installers.sh
  step bash scripts/tests/test-services.sh
  step pwsh -NoProfile -File scripts/validate-powershell.ps1
  step pwsh -NoProfile -File scripts/tests/test-mirrors.ps1
  step pwsh -NoProfile -File scripts/tests/test-installers.ps1
  step pwsh -NoProfile -File scripts/tests/test-services.ps1
fi
if [ "$skip_docs" -eq 0 ]; then step bash scripts/e2e/check-docs.sh; fi
if [ "$quick" -eq 0 ] && [ "$skip_audit" -eq 0 ]; then
  step cargo audit
  step npm audit --prefix admin-web --audit-level=high
fi
if [ "$quick" -eq 0 ] && [ "$skip_e2e" -eq 0 ]; then
  check_official_parser() (
    local fixture_dir
    fixture_dir="$(mktemp -d "${TMPDIR:-/tmp}/vpn233-core-fixtures.XXXXXXXX")"
    trap 'rm -rf -- "$fixture_dir"' EXIT
    cargo run --locked --quiet -p vpn233-core-lib-rust --example mihomo_fixtures -- "$fixture_dir"
    "$xtask" network mihomo --download-core --cache-dir .cache/tools/mihomo --provider-bin "$release_dir/vpn233-provider-server" --core-fixtures-dir "$fixture_dir"
  )
  step check_official_parser
  BIN_DIR="$release_dir" step bash scripts/e2e/e2e.sh
  step npx --prefix admin-web playwright install chromium
  VPN233_UI_BINARY="$release_dir/vpn233-subscribe-server" step npm run test:ui --prefix admin-web
  case "$(uname -s):$(uname -m)" in
    Linux:x86_64) platform=linux-x86_64; suffix='' ;;
    Darwin:arm64) platform=macos-aarch64; suffix='' ;;
    Darwin:x86_64) platform=macos-x86_64; suffix='' ;;
    *) echo 'Unsupported local release packaging platform' >&2; exit 1 ;;
  esac
  PLATFORM="$platform" TARGET=local EXE_SUFFIX="$suffix" VERSION="$(tr -d '\r\n' < version.txt)" TARGET_DIR="$release_dir" BUNDLE_ROOT=target/verify-package/assets step bash .github/scripts/assemble-bundle.sh
fi
if [ -n "$cache_ticket" ]; then
  if cache_result="$("$xtask" quick-cache store --flavor bash --engine "$cache_engine" --ticket "$cache_ticket" --checks "$checks" --elapsed "$((SECONDS - started))")"; then
    printf '%s' "$cache_result" | "$xtask" json-field /message
  fi
  cache_finished=1
fi
mode='FULL'
if [ "$quick" -eq 1 ]; then mode='QUICK (not release validation)'
fi
if [ "$((skip_rust + skip_web + skip_scripts + skip_docs + skip_e2e + skip_audit))" -gt 0 ]; then mode='PARTIAL (not release validation)'; fi
printf '\n%s: %s checks passed in %ss\n' "$mode" "$checks" "$((SECONDS - started))"
