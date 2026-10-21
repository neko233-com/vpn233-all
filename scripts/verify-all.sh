#!/usr/bin/env bash
# Quick omits release builds, E2E and network audits; never a release sign-off.
set -Eeuo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
quick=0; skip_rust=0; skip_web=0; skip_scripts=0; skip_docs=0; skip_e2e=0; skip_audit=0
for option in "$@"; do
  case "$option" in
    --quick) quick=1 ;; --skip-rust) skip_rust=1 ;; --skip-web) skip_web=1 ;;
    --skip-scripts) skip_scripts=1 ;; --skip-docs) skip_docs=1 ;; --skip-e2e) skip_e2e=1 ;; --skip-audit) skip_audit=1 ;;
    -h|--help) echo 'verify-all.sh [--quick] [--skip-rust] [--skip-web] [--skip-scripts] [--skip-docs] [--skip-e2e] [--skip-audit]'; exit 0 ;;
    *) echo "Unknown option: $option" >&2; exit 2 ;;
  esac
done
started=$SECONDS
checks=0
step() { printf '\n==> %s\n' "$*"; checks=$((checks + 1)); "$@"; }
require() { command -v "$1" >/dev/null || { echo "Required tool missing: $1" >&2; exit 1; }; }
candidates() { git ls-files --cached --others --exclude-standard -z | python -c 'import sys; extension=sys.argv[1]; sys.stdout.buffer.write(b"\0".join(sorted(set(p for p in sys.stdin.buffer.read().split(b"\0") if p.endswith(extension.encode()) and not p.startswith(b"legacy/")))) + b"\0")' "$1"; }
require python
export CARGO_ENCODED_RUSTFLAGS
CARGO_ENCODED_RUSTFLAGS="$(python scripts/build-environment.py)"
step python scripts/release-version.py
step python scripts/check-privacy.py
step git diff --check
if [ "$quick" -eq 0 ]; then
  step python scripts/check-privacy.py --history
  step gitleaks git --config .gitleaks.toml --redact --no-banner --exit-code 1
fi
if [ "$skip_rust" -eq 0 ]; then
  step cargo fmt --all -- --check
  step cargo clippy --locked --workspace --all-targets -- -D warnings
  step cargo test --locked --workspace
  if [ "$quick" -eq 0 ]; then step cargo build --locked --release --workspace; fi
fi
if [ "$skip_web" -eq 0 ]; then
  lock_hash="$(python -c 'import hashlib; from pathlib import Path; print(hashlib.sha256(Path("admin-web/package-lock.json").read_bytes()).hexdigest())')"
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
  step python -m unittest discover -s scripts/tests -p 'test_*.py' -v
fi
if [ "$skip_docs" -eq 0 ]; then step bash scripts/e2e/check-docs.sh; fi
if [ "$quick" -eq 0 ] && [ "$skip_audit" -eq 0 ]; then
  step cargo audit
  step npm audit --prefix admin-web --audit-level=high
fi
if [ "$quick" -eq 0 ] && [ "$skip_e2e" -eq 0 ]; then
  step bash scripts/e2e/e2e.sh
  step npx --prefix admin-web playwright install chromium
  step npm run test:ui --prefix admin-web
  case "$(uname -s):$(uname -m)" in
    Linux:x86_64) platform=linux-x86_64; suffix='' ;;
    Darwin:arm64) platform=macos-aarch64; suffix='' ;;
    Darwin:x86_64) platform=macos-x86_64; suffix='' ;;
    *) echo 'Unsupported local release packaging platform' >&2; exit 1 ;;
  esac
  PLATFORM="$platform" TARGET=local EXE_SUFFIX="$suffix" VERSION="$(tr -d '\r\n' < version.txt)" TARGET_DIR="${CARGO_TARGET_DIR:-target}/release" BUNDLE_ROOT=target/verify-package step bash .github/scripts/assemble-bundle.sh
fi
mode='FULL'
if [ "$quick" -eq 1 ]; then mode='QUICK (not release validation)'
elif [ "$((skip_rust + skip_web + skip_scripts + skip_docs + skip_e2e + skip_audit))" -gt 0 ]; then mode='PARTIAL (not release validation)'; fi
printf '\n%s: %s checks passed in %ss\n' "$mode" "$checks" "$((SECONDS - started))"
