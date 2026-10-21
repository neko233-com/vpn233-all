#!/usr/bin/env bash
# Assemble one release bundle directory for a single platform.
#
# Invoked by .github/workflows/release.yml on every build-matrix entry. The
# script runs under Git Bash on Windows and under bash on Linux/macOS, so it
# only uses portable shell plus coreutils/tar (and PowerShell for zip on
# Windows).
#
# Required environment variables:
#   PLATFORM      e.g. linux-x86_64
#   TARGET        rust target triple, e.g. x86_64-unknown-linux-gnu
#   EXE_SUFFIX    "" or ".exe"
#   VERSION       release version without the leading "v"
# Optional:
#   BUNDLE_ROOT   output directory (default: "dist")
#
# Produces:
#   dist/bundle/vpn233-rust-<platform>/...    bundle tree
#   dist/vpn233-rust-<platform>.tar.gz
#   dist/vpn233-rust-<platform>.zip

set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

PLATFORM="${PLATFORM:?PLATFORM is required}"
TARGET="${TARGET:?TARGET is required}"
EXE_SUFFIX="${EXE_SUFFIX:-}"
VERSION="${VERSION:?VERSION is required}"
BUNDLE_ROOT="${BUNDLE_ROOT:-dist}"
case "$PLATFORM" in linux-x86_64|windows-x86_64|macos-x86_64|macos-aarch64) ;; *) echo 'invalid platform' >&2; exit 1 ;; esac
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'invalid release version' >&2; exit 1; }
# All recursive removal stays inside the explicitly named workspace output.
[[ "$BUNDLE_ROOT" =~ ^[A-Za-z0-9_./-]+$ && "$BUNDLE_ROOT" != /* && "/$BUNDLE_ROOT/" != *'/../'* ]] || { echo 'unsafe bundle output directory' >&2; exit 1; }

BUNDLE_NAME="vpn233-rust-${PLATFORM}"
BUNDLE_DIR="${BUNDLE_ROOT}/bundle/${BUNDLE_NAME}"
TARGET_DIR="${TARGET_DIR:-target/${TARGET}/release}"

log() { printf '[bundle] %s\n' "$*"; }
die() { printf '[bundle] ERROR: %s\n' "$*" >&2; exit 1; }

[ -d "$TARGET_DIR" ] || die "missing Rust target directory: $TARGET_DIR"
[ -f "admin-web/dist/index.html" ] || die "missing admin-web/dist/index.html; run npm ci && npm run build first"
python scripts/release-version.py
[ "$(tr -d '\r\n' < version.txt)" = "$VERSION" ] || die 'bundle version differs from the shared version chain'

python -c 'from pathlib import Path; import sys; root=Path.cwd().resolve(); target=Path(sys.argv[1]).resolve(); target.relative_to(root); assert target != root' "$BUNDLE_DIR"
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR/bin" "$BUNDLE_DIR/examples" "$BUNDLE_DIR/admin-web" "$BUNDLE_DIR/docs" "$BUNDLE_DIR/scripts/lib"

# --- binaries -------------------------------------------------------------
for binary in vpn233-provider-server vpn233-subscribe-server vpn233-client; do
  source_file="${TARGET_DIR}/${binary}${EXE_SUFFIX}"
  [ -f "$source_file" ] || die "missing binary: $source_file"
  cp "$source_file" "$BUNDLE_DIR/bin/"
  chmod +x "$BUNDLE_DIR/bin/${binary}${EXE_SUFFIX}" 2>/dev/null || true
done

# --- admin web ------------------------------------------------------------
cp -R admin-web/dist/. "$BUNDLE_DIR/admin-web/dist/"
[ -f "$BUNDLE_DIR/admin-web/dist/index.html" ] || die "admin web copy failed"

# --- configuration templates ---------------------------------------------
cp provider-server/config.example.yaml "$BUNDLE_DIR/examples/provider-server.yaml"
cp subscribe-server/config.example.yaml "$BUNDLE_DIR/examples/subscribe-server.yaml"
cp vpn233-client/config.example.yaml "$BUNDLE_DIR/examples/vpn233-client.yaml"
cp scripts/lib/mirrors.sh scripts/lib/mirrors.ps1 scripts/lib/services.sh scripts/lib/layout.sh scripts/lib/layout.ps1 "$BUNDLE_DIR/scripts/lib/"

# --- operator scripts ----------------------------------------------------
required_scripts=(
  install.sh install.ps1
  deploy.sh deploy.ps1
  update.sh update.ps1
  uninstall.sh uninstall.ps1
  start.sh start.ps1
  stop.sh stop.ps1
)
for script in "${required_scripts[@]}"; do
  [ -f "$script" ] || die "missing operator script: $script (expected at repository root)"
  cp "$script" "$BUNDLE_DIR/"
done
chmod +x "$BUNDLE_DIR"/*.sh 2>/dev/null || true

# --- metadata and docs ---------------------------------------------------
printf '%s\n' "$VERSION" > "$BUNDLE_DIR/version.txt"
cp README.md README.zh-CN.md LICENSE SECURITY.md CONTRIBUTING.md CHANGELOG.md CODE_OF_CONDUCT.md "$BUNDLE_DIR/"
cp docs/*.md docs/_config.yml "$BUNDLE_DIR/docs/"

# Guard against accidentally shipping runtime state or credentials. Example
# configuration files are wanted; live `config.yaml`, data directories and
# logs are not.
if find "$BUNDLE_DIR" \( -name 'config.yaml' -o -name '*.log' -o -type d -name 'data' -o -type d -name 'backups' \) -print | grep -q .; then
  die "bundle contains runtime state or credentials; refusing to package"
fi

# --- archives ------------------------------------------------------------
mkdir -p "$BUNDLE_ROOT"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}" python scripts/create-archives.py "$BUNDLE_DIR" "$BUNDLE_ROOT"
log "wrote ${BUNDLE_ROOT}/${BUNDLE_NAME}.tar.gz"
[ -f "${BUNDLE_ROOT}/${BUNDLE_NAME}.zip" ] || die "zip archive was not created"
log "wrote ${BUNDLE_ROOT}/${BUNDLE_NAME}.zip"
printf '%s\n' "$VERSION" > "$BUNDLE_ROOT/version.txt"
(cd "$BUNDLE_ROOT" && sha256sum "$BUNDLE_NAME.tar.gz" "$BUNDLE_NAME.zip" > SHA256SUMS)
python scripts/verify-release.py "$BUNDLE_ROOT" "$VERSION" --partial

# --- summary -------------------------------------------------------------
log "bundle contents:"
find "$BUNDLE_DIR" -maxdepth 2 -type f | sed "s|^${BUNDLE_DIR}/||" | sort
