#!/usr/bin/env bash
# shellcheck shell=bash
#
# VPN233 one-command installer (Linux / macOS / Git Bash on Windows).
#
# Release mode installs a published bundle:
#   vpn233-rust-<platform>.tar.gz or .zip downloaded from GitHub releases
#   through the built-in GitHub mirror acceleration list, then verified
#   against SHA256SUMS before anything is replaced.
#
# Source mode (no arguments inside a checkout) keeps the legacy behaviour of
# copying a local build into generated/, admin-web/dist/ and config/.
#
# Every path that can hold credentials is created with mode 0600 and existing
# configuration is never overwritten.
set -Eeuo pipefail
export LC_ALL=C
umask 077

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

MODE="source"
VERSION=""
BUNDLE=""
SHA256_ARG=""
PREFIX=""
SYSTEM=0
SYSTEM_EXPLICIT=0
LAYOUT_SCOPE="user"
LAYOUT_LIBRARY=""
MIRROR_VALUE=""
NO_START=0
DRY_RUN=0
ALLOW_UNSIGNED="${VPN233_ALLOW_UNSIGNED:-0}"
SKIP_HEALTHCHECK="${VPN233_SKIP_HEALTHCHECK:-0}"

TEMP_DIR=""
BIN_EXT=""
BUNDLE_ROOT=""
BUNDLE_VERSION=""
INSTALL_STARTED=0
GENERATED_CREDS=0
ROLLBACK_ACTIVE=0
WEB_PREVIOUS=""
WEB_NEW=""

# Layout (filled in by resolve_layout).
BASE_DIR=""
BIN_DIR=""
WEB_DIR=""
CONFIG_DIR=""
DATA_DIR=""
LOG_DIR=""
BACKUP_DIR=""

# Service ports (filled in by resolve_ports).
ADMIN_USER="${VPN233_ADMIN_USER:-root}"
ADMIN_PASSWORD="${VPN233_ADMIN_PASSWORD:-}"
PROVIDER_TOKEN="${VPN233_PROVIDER_FEED_TOKEN:-}"
REGISTRY_TOKEN="${VPN233_REGISTRY_TOKEN:-}"
PROVIDER_PORT=""
SUBSCRIBE_PORT=""
CLIENT_SOCKS_PORT=""
CLIENT_HTTP_PORT=""
CLIENT_WEB_PORT=""

usage() {
  printf '%s\n' "VPN233 installer"
  printf '\n'
  printf '%s\n' "Usage:"
  printf '%s\n' "  bash install.sh                         install from this source checkout"
  printf '%s\n' "  bash install.sh --release [options]     install the latest published bundle"
  printf '%s\n' "  bash install.sh --bundle FILE [options] install a local release bundle offline"
  printf '%s\n' "  bash install.sh update [options]        alias for update.sh"
  printf '\n'
  printf '%s\n' "Options:"
  printf '%s\n' "  --release                 install the published GitHub release bundle"
  printf '%s\n' "  --version VERSION         install a specific MAJOR.MINOR.PATCH release"
  printf '%s\n' "  --bundle FILE             install a checksum-verified offline .tar.gz/.zip bundle"
  printf '%s\n' "  --sha256 HASH             expected SHA-256 for --bundle"
  printf '%s\n' "  --prefix DIR              install directory (release mode)"
  printf '%s\n' "  --system                  system wide install (needs sudo/root)"
  printf '%s\n' "  --mirror auto|off|PREFIX  GitHub mirror behaviour (release mode)"
  printf '%s\n' "  --no-start                install files/services but do not start them"
  printf '%s\n' "  --yes                     assume yes for non-interactive use"
  printf '%s\n' "  --dry-run                 print planned actions, write nothing"
  printf '%s\n' "  -h, --help                show this help"
  printf '\n'
  printf '%s\n' "Environment overrides:"
  printf '%s\n' "  VPN233_INSTALL_DIR VPN233_BIN_DIR VPN233_WEB_DIR VPN233_CONFIG_DIR"
  printf '%s\n' "  VPN233_DATA_DIR VPN233_ADMIN_PASSWORD VPN233_PROVIDER_FEED_TOKEN"
  printf '%s\n' "  VPN233_REGISTRY_TOKEN VPN233_PROVIDER_PORT VPN233_SUBSCRIBE_PORT"
  printf '%s\n' "  VPN233_CLIENT_SOCKS_PORT VPN233_CLIENT_HTTP_PORT VPN233_CLIENT_WEB_PORT"
  printf '%s\n' "  VPN233_GH_MIRROR VPN233_GH_MIRRORS VPN233_NO_MIRROR VPN233_SKIP_HEALTHCHECK"
}

log() {
  printf '%s\n' "$*"
}

warn() {
  printf 'WARNING: %s\n' "$*" >&2
}

die() {
  printf 'VPN233 install: %s\n' "$*" >&2
  exit 1
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    die "required command not found: $1"
  fi
}

is_windows_shell() {
  case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) return 0 ;;
  esac
  return 1
}

list_contains() {
  local needle="$1" list="$2" item
  local old_ifs="$IFS"
  IFS=' '
  for item in $list; do
    if [[ "$item" == "$needle" ]]; then
      IFS="$old_ifs"
      return 0
    fi
  done
  IFS="$old_ifs"
  return 1
}

# ---------------------------------------------------------------------------
# Command line parsing
# ---------------------------------------------------------------------------
if [[ $# -gt 0 && "$1" == "update" ]]; then
  shift
  if [[ -x "$SCRIPT_DIR/update.sh" ]]; then
    exec "$SCRIPT_DIR/update.sh" "$@"
  fi
  exec bash "$SCRIPT_DIR/update.sh" "$@"
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --release)
      MODE="release"
      shift
      ;;
    --version)
      [[ $# -ge 2 ]] || die "--version requires a value"
      VERSION="$2"
      MODE="release"
      shift 2
      ;;
    --version=*)
      VERSION="${1#--version=}"
      MODE="release"
      shift
      ;;
    --bundle|--offline-bundle)
      [[ $# -ge 2 ]] || die "$1 requires a path"
      BUNDLE="$2"
      MODE="release"
      shift 2
      ;;
    --bundle=*|--offline-bundle=*)
      BUNDLE="${1#*=}"
      MODE="release"
      shift
      ;;
    --sha256)
      [[ $# -ge 2 ]] || die "--sha256 requires a value"
      SHA256_ARG="$2"
      shift 2
      ;;
    --sha256=*)
      SHA256_ARG="${1#--sha256=}"
      shift
      ;;
    --prefix)
      [[ $# -ge 2 ]] || die "--prefix requires a path"
      PREFIX="$2"
      MODE="release"
      shift 2
      ;;
    --prefix=*)
      PREFIX="${1#--prefix=}"
      MODE="release"
      shift
      ;;
    --system)
      SYSTEM=1
      SYSTEM_EXPLICIT=1
      shift
      ;;
    --mirror)
      [[ $# -ge 2 ]] || die "--mirror requires a value"
      MIRROR_VALUE="$2"
      shift 2
      ;;
    --mirror=*)
      MIRROR_VALUE="${1#--mirror=}"
      shift
      ;;
    --no-start)
      NO_START=1
      shift
      ;;
    --yes|-y)
      # Accepted for non-interactive callers (deploy.sh/deploy.ps1 pass it).
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1 (use --help)"
      ;;
  esac
done

[[ -z "$SHA256_ARG" || "$SHA256_ARG" =~ ^[0-9A-Fa-f]{64}$ ]] || die "--sha256 requires exactly 64 hexadecimal characters"
[[ -z "$SHA256_ARG" || -n "$BUNDLE" ]] || die "--sha256 requires --bundle"

# ---------------------------------------------------------------------------
# Shared helpers
# ---------------------------------------------------------------------------
cleanup_temp() {
  local exit_code=$? name
  if [[ "$exit_code" -ne 0 && "$ROLLBACK_ACTIVE" -eq 1 ]]; then
    warn "installation failed; restoring previous application files (configuration/data preserved)"
    if [[ -f "$BASE_DIR/stop.sh" ]]; then VPN233_BIN_DIR="$BIN_DIR" bash "$BASE_DIR/stop.sh" >/dev/null 2>&1 || true; fi
    for name in vpn233-provider-server vpn233-subscribe-server vpn233-client; do
      if [[ -f "$TEMP_DIR/rollback/$name$BIN_EXT" ]]; then
        cp -f "$TEMP_DIR/rollback/$name$BIN_EXT" "$BIN_DIR/$name$BIN_EXT" || true
      elif [[ -f "$BIN_DIR/$name$BIN_EXT" ]]; then rm -f "$BIN_DIR/$name$BIN_EXT" || true; fi
    done
    if [[ -f "$TEMP_DIR/rollback/version.txt" ]]; then cp -f "$TEMP_DIR/rollback/version.txt" "$BASE_DIR/version.txt" || true; else rm -f "$BASE_DIR/version.txt" || true; fi
    if [[ -n "$WEB_PREVIOUS" && -d "$WEB_PREVIOUS" ]]; then
      rm -rf "$WEB_DIR" || true
      mv "$WEB_PREVIOUS" "$WEB_DIR" || true
      WEB_PREVIOUS=""
    fi
    warn "previous application files restored; restart locally after reviewing the failure"
  fi
  [[ -z "$WEB_PREVIOUS" || ! -d "$WEB_PREVIOUS" ]] || rm -rf "$WEB_PREVIOUS" 2>/dev/null || true
  [[ -z "$WEB_NEW" || ! -d "$WEB_NEW" ]] || rm -rf "$WEB_NEW" 2>/dev/null || true
  if [[ -n "$TEMP_DIR" && -d "$TEMP_DIR" ]]; then
    rm -rf "$TEMP_DIR" 2>/dev/null || true
  fi
}
trap cleanup_temp EXIT

make_temp_dir() {
  [[ -z "$TEMP_DIR" ]] || return 0
  TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/vpn233-install.XXXXXX")"
  if [[ -z "$TEMP_DIR" || ! -d "$TEMP_DIR" ]]; then
    die "failed to create a temporary directory"
  fi
}

gen_hex() {
  local bytes="$1" value=""
  if command -v openssl >/dev/null 2>&1; then
    value="$(openssl rand -hex "$bytes" 2>/dev/null || true)"
  fi
  if [[ -z "$value" && -r /dev/urandom ]] && command -v od >/dev/null 2>&1; then
    value="$(od -An -N "$bytes" -tx1 /dev/urandom | tr -d ' \n')"
  fi
  if [[ -z "$value" ]]; then
    die "could not generate random credentials (need openssl or od + /dev/urandom)"
  fi
  printf '%s\n' "$value"
}

sanitize_version() {
  local value="$1"
  value="${value#v}"
  value="${value#V}"
  [[ "$value" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]] || die "invalid semantic release version"
  printf '%s\n' "$value"
}

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$arch" in
    x86_64|amd64|x64) arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *)
      die "unsupported CPU architecture: $arch (published bundles: x86_64, macOS aarch64)"
      ;;
  esac
  case "$os" in
    Linux*)
      if [[ "$arch" != "x86_64" ]]; then
        die "no published Linux bundle for $arch; only linux-x86_64 is released"
      fi
      printf 'linux-%s\n' "$arch"
      ;;
    Darwin*) printf 'macos-%s\n' "$arch" ;;
    MINGW*|MSYS*|CYGWIN*)
      if [[ "$arch" != "x86_64" ]]; then
        die "no published Windows bundle for $arch; only windows-x86_64 is released"
      fi
      printf 'windows-%s\n' "$arch"
      ;;
    *)
      die "unsupported operating system: $os"
      ;;
  esac
}

# ---------------------------------------------------------------------------
# Mirror library loading (only needed when downloading a release)
# ---------------------------------------------------------------------------
load_mirror_library() {
  local candidate
  for candidate in \
    "$SCRIPT_DIR/scripts/lib/mirrors.sh" \
    "$SCRIPT_DIR/lib/mirrors.sh" \
    "$SCRIPT_DIR/mirrors.sh"; do
    if [[ -f "$candidate" ]]; then
      # shellcheck source=scripts/lib/mirrors.sh
      . "$candidate"
      return 0
    fi
  done
  # Standalone public bootstrap: this is the same public helper shipped in the
  # release bundle. Application archives are still SHA-256 verified below.
  if [[ "$DRY_RUN" -eq 0 ]]; then
    make_temp_dir
    local prefix url
    for prefix in 'https://ghproxy.net/' 'https://gh.llkk.cc/' 'https://ghfast.top/' 'https://gh-proxy.com/' ''; do
      if [[ "${VPN233_NO_MIRROR:-0}" == "1" || "$MIRROR_VALUE" == "off" ]]; then prefix=''; fi
      if [[ -n "$MIRROR_VALUE" && "$MIRROR_VALUE" != auto && "$MIRROR_VALUE" != off ]]; then
        [[ "$MIRROR_VALUE" == https://* || "$MIRROR_VALUE" == http://* ]] || die "invalid mirror prefix"
        case "$MIRROR_VALUE" in *'@'*|*'?'*|*'#'*|*[[:space:]]*) die "mirror prefix cannot contain credentials or query parameters" ;; esac
        prefix="${MIRROR_VALUE%/}/"
      fi
      url="${prefix}https://raw.githubusercontent.com/neko233-com/vpn233-all/main/scripts/lib/mirrors.sh"
      if curl --fail --location --silent --show-error --connect-timeout 5 --max-time 30 --output "$TEMP_DIR/mirrors.sh.part" "$url" 2>/dev/null && [[ -s "$TEMP_DIR/mirrors.sh.part" ]]; then
        mv "$TEMP_DIR/mirrors.sh.part" "$TEMP_DIR/mirrors.sh"
        # shellcheck disable=SC1091
        . "$TEMP_DIR/mirrors.sh"
        return 0
      fi
    done
  fi
  return 1
}

# ---------------------------------------------------------------------------
# Layout resolution
# ---------------------------------------------------------------------------
resolve_layout() {
  local layout_status library legacy_bin
  if [[ "$MODE" == "source" ]]; then
    BASE_DIR="$SCRIPT_DIR"
    BIN_DIR="${VPN233_BIN_DIR:-$SCRIPT_DIR/generated}"
    WEB_DIR="${VPN233_WEB_DIR:-$SCRIPT_DIR/admin-web/dist}"
    CONFIG_DIR="${VPN233_CONFIG_DIR:-$SCRIPT_DIR/config}"
    DATA_DIR="${VPN233_DATA_DIR:-$SCRIPT_DIR/generated/data}"
    LOG_DIR="$SCRIPT_DIR"
    BACKUP_DIR="$SCRIPT_DIR/generated/backups"
    return 0
  fi
  if [[ -n "$PREFIX" ]]; then
    BASE_DIR="$PREFIX"
  elif [[ "$SYSTEM" -eq 1 ]]; then
    BASE_DIR="/opt/vpn233"
  else
    BASE_DIR="${VPN233_INSTALL_DIR:-${VPN233_HOME:-${HOME:-/tmp}/.vpn233}}"
  fi
  case "$BASE_DIR" in /*) ;; *) BASE_DIR="$PWD/$BASE_DIR" ;; esac
  for library in "$SCRIPT_DIR/scripts/lib/layout.sh" "$BASE_DIR/scripts/lib/layout.sh" "${BUNDLE_ROOT:-}/scripts/lib/layout.sh"; do
    if [[ -f "$library" ]]; then
      # shellcheck disable=SC1090
      . "$library"
      LAYOUT_LIBRARY="$library"
      break
    fi
  done
  if declare -F vpn233_layout_load >/dev/null 2>&1; then
    if vpn233_layout_load "$BASE_DIR"; then :
    else
      layout_status=$?
      [[ "$layout_status" -eq 1 ]] || die "invalid private runtime layout; restore it locally before installing"
      if [[ "$SYSTEM_EXPLICIT" -eq 0 && -z "${VPN233_CONFIG_DIR:-}" ]]; then
        legacy_bin="${VPN233_BIN_DIR:-$BASE_DIR/bin}"
        if [[ -z "${VPN233_BIN_DIR:-}" && ! -d "$BASE_DIR/bin" && -d "$BASE_DIR/generated" ]]; then legacy_bin="$BASE_DIR/generated"; fi
        if vpn233_layout_legacy "$BASE_DIR" "$legacy_bin"; then :
        else layout_status=$?; [[ "$layout_status" -eq 1 ]] || die "cannot determine existing install layout; specify --system or explicit directory overrides"; fi
      fi
    fi
    LAYOUT_SCOPE="${VPN233_SAVED_SCOPE:-user}"
    if [[ "$SYSTEM_EXPLICIT" -eq 1 ]]; then LAYOUT_SCOPE=system; fi
    if [[ "$LAYOUT_SCOPE" == system ]]; then SYSTEM=1; fi
  elif [[ -e "$BASE_DIR/.vpn233-layout" ]]; then
    die "private runtime layout exists but its public parser is missing; restore scripts/lib/layout.sh"
  fi
  BIN_DIR="${VPN233_BIN_DIR:-${VPN233_SAVED_BIN:-$BASE_DIR/bin}}"
  WEB_DIR="${VPN233_WEB_DIR:-${VPN233_SAVED_WEB:-$BASE_DIR/admin-web/dist}}"
  if [[ "$SYSTEM" -eq 1 ]]; then
    LAYOUT_SCOPE=system
    CONFIG_DIR="${VPN233_CONFIG_DIR:-${VPN233_SAVED_CONFIG:-/etc/vpn233}}"
    DATA_DIR="${VPN233_DATA_DIR:-${VPN233_SAVED_DATA:-/var/lib/vpn233}}"
  else
    CONFIG_DIR="${VPN233_CONFIG_DIR:-${VPN233_SAVED_CONFIG:-$BASE_DIR/config}}"
    DATA_DIR="${VPN233_DATA_DIR:-${VPN233_SAVED_DATA:-$BASE_DIR/data}}"
  fi
  LOG_DIR="$BASE_DIR/logs"
  BACKUP_DIR="$BASE_DIR/backups"
  local path
  for path in "$BASE_DIR" "$BIN_DIR" "$WEB_DIR" "$CONFIG_DIR" "$DATA_DIR"; do
    [[ "$path" != *$'\n'* && "$path" != *$'\r'* ]] || die "installation paths cannot contain line breaks"
  done
}

print_layout() {
  printf '%s\n' "  install directory : $BASE_DIR"
  printf '%s\n' "  binaries          : $BIN_DIR"
  printf '%s\n' "  admin web         : $WEB_DIR"
  printf '%s\n' "  configuration     : $CONFIG_DIR (0600, never overwritten)"
  printf '%s\n' "  data              : $DATA_DIR"
  printf '%s\n' "  logs              : $LOG_DIR"
  printf '%s\n' "  backups           : $BACKUP_DIR"
}

# ---------------------------------------------------------------------------
# Ports
# ---------------------------------------------------------------------------
port_in_use() {
  local port="$1"
  if [[ -z "$port" ]]; then
    return 1
  fi
  if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
    return 0
  fi
  return 1
}

pick_port() {
  local candidate="$1" used="$2" guard=0
  while :; do
    guard=$((guard + 1))
    if [[ "$guard" -gt 200 ]]; then
      die "could not find a free TCP port near $1"
    fi
    if ! list_contains "$candidate" "$used"; then
      if ! port_in_use "$candidate"; then
        printf '%s\n' "$candidate"
        return 0
      fi
    fi
    candidate=$((candidate + 1))
    if [[ "$candidate" -gt 65535 ]]; then
      die "no free TCP port found"
    fi
  done
}

read_yaml_port() {
  local file="$1" pattern="$2"
  if [[ ! -f "$file" ]]; then
    return 1
  fi
  awk -v re="$pattern" '$0 ~ re { if (match($0, /[0-9]+/)) { print substr($0, RSTART, RLENGTH); exit } }' "$file"
}

read_web_port() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    return 1
  fi
  awk '
    /^web_addr:/ {
      if (match($0, /:[0-9]+/)) {
        print substr($0, RSTART + 1, RLENGTH - 1)
        exit
      }
    }
  ' "$file"
}

resolve_ports() {
  local used="" parsed
  if [[ -f "$CONFIG_DIR/provider-server.yaml" ]]; then
    parsed="$(read_yaml_port "$CONFIG_DIR/provider-server.yaml" '^listen_port:')"
    if [[ -n "$parsed" ]]; then
      PROVIDER_PORT="$parsed"
    fi
  fi
  if [[ -f "$CONFIG_DIR/subscribe-server.yaml" ]]; then
    parsed="$(read_yaml_port "$CONFIG_DIR/subscribe-server.yaml" '^  http_port:')"
    if [[ -n "$parsed" ]]; then
      SUBSCRIBE_PORT="$parsed"
    fi
  fi
  if [[ -f "$CONFIG_DIR/vpn233-client.yaml" ]]; then
    parsed="$(read_yaml_port "$CONFIG_DIR/vpn233-client.yaml" '^socks_port:')"
    if [[ -n "$parsed" ]]; then
      CLIENT_SOCKS_PORT="$parsed"
    fi
    parsed="$(read_yaml_port "$CONFIG_DIR/vpn233-client.yaml" '^http_port:')"
    if [[ -n "$parsed" ]]; then
      CLIENT_HTTP_PORT="$parsed"
    fi
    parsed="$(read_web_port "$CONFIG_DIR/vpn233-client.yaml")"
    if [[ -n "$parsed" ]]; then
      CLIENT_WEB_PORT="$parsed"
    fi
  fi
  if [[ -n "$PROVIDER_PORT" ]]; then used="$used $PROVIDER_PORT"; else
    if [[ -n "${VPN233_PROVIDER_PORT:-}" ]]; then PROVIDER_PORT="$VPN233_PROVIDER_PORT"; else
      PROVIDER_PORT="$(pick_port 8080 "$used")"
    fi
    used="$used $PROVIDER_PORT"
  fi
  if [[ -n "$SUBSCRIBE_PORT" ]]; then used="$used $SUBSCRIBE_PORT"; else
    if [[ -n "${VPN233_SUBSCRIBE_PORT:-}" ]]; then SUBSCRIBE_PORT="$VPN233_SUBSCRIBE_PORT"; else
      SUBSCRIBE_PORT="$(pick_port 23444 "$used")"
    fi
    used="$used $SUBSCRIBE_PORT"
  fi
  if [[ -n "$CLIENT_SOCKS_PORT" ]]; then used="$used $CLIENT_SOCKS_PORT"; else
    if [[ -n "${VPN233_CLIENT_SOCKS_PORT:-}" ]]; then CLIENT_SOCKS_PORT="$VPN233_CLIENT_SOCKS_PORT"; else
      CLIENT_SOCKS_PORT="$(pick_port 1080 "$used")"
    fi
    used="$used $CLIENT_SOCKS_PORT"
  fi
  if [[ -n "$CLIENT_HTTP_PORT" ]]; then used="$used $CLIENT_HTTP_PORT"; else
    if [[ -n "${VPN233_CLIENT_HTTP_PORT:-}" ]]; then CLIENT_HTTP_PORT="$VPN233_CLIENT_HTTP_PORT"; else
      CLIENT_HTTP_PORT="$(pick_port 1081 "$used")"
    fi
    used="$used $CLIENT_HTTP_PORT"
  fi
  if [[ -n "$CLIENT_WEB_PORT" ]]; then used="$used $CLIENT_WEB_PORT"; else
    if [[ -n "${VPN233_CLIENT_WEB_PORT:-}" ]]; then CLIENT_WEB_PORT="$VPN233_CLIENT_WEB_PORT"; else
      CLIENT_WEB_PORT="$(pick_port 8082 "$used")"
    fi
    used="$used $CLIENT_WEB_PORT"
  fi
  local port seen=""
  for port in "$PROVIDER_PORT" "$SUBSCRIBE_PORT" "$CLIENT_SOCKS_PORT" "$CLIENT_HTTP_PORT" "$CLIENT_WEB_PORT"; do
    [[ "$port" =~ ^[0-9]+$ && ${#port} -le 5 ]] || die "ports must be integers in 1..65535"
    (( 10#$port >= 1 && 10#$port <= 65535 )) || die "ports must be integers in 1..65535"
    list_contains "$port" "$seen" && die "configuration contains conflicting TCP ports"
    seen="$seen $port"
  done
}

# ---------------------------------------------------------------------------
# Config generation
# ---------------------------------------------------------------------------
sed_escape_replacement() {
  printf '%s' "$1" | sed -e 's/[\\&|]/\\&/g'
}

yaml_quoted() {
  local value="$1"
  [[ "$value" != *$'\n'* && "$value" != *$'\r'* ]] || die "configuration values cannot contain line breaks"
  value="${value//\'/\'\'}"
  printf "'%s'" "$value"
}

yaml_replacement() {
  local quoted
  quoted="$(yaml_quoted "$1")" || return 1
  sed_escape_replacement "$quoted"
}

patch_line() {
  local file="$1" expression="$2" tmp
  tmp="${file}.patch.$$"
  if sed -e "$expression" "$file" > "$tmp"; then
    mv -f "$tmp" "$file"
  else
    rm -f "$tmp" 2>/dev/null || true
    die "failed to update configuration file: $file"
  fi
}

render_provider_config() {
  local file="$1" password token port
  password="$(yaml_replacement "$ADMIN_PASSWORD")"
  token="$(yaml_replacement "$PROVIDER_TOKEN")"
  port="$(sed_escape_replacement "$PROVIDER_PORT")"
  patch_line "$file" "s|^listen_port:.*|listen_port: ${port}|"
  patch_line "$file" "s|^listen_addr:.*|listen_addr: '127.0.0.1'|"
  patch_line "$file" "s|^admin_user:.*|admin_user: $(yaml_replacement "$ADMIN_USER")|"
  patch_line "$file" "s|^admin_password:.*|admin_password: ${password}|"
  patch_line "$file" "s|^provider_access_token:.*|provider_access_token: ${token}|"
  patch_line "$file" "s|^registry_url:.*|registry_url: \"http://127.0.0.1:$(sed_escape_replacement "$SUBSCRIBE_PORT")\"|"
  patch_line "$file" "s|^registry_token:.*|registry_token: $(yaml_replacement "$REGISTRY_TOKEN")|"
}

render_subscribe_config() {
  local file="$1" password token port data web stats
  password="$(yaml_replacement "$ADMIN_PASSWORD")"
  token="$(yaml_replacement "$PROVIDER_TOKEN")"
  port="$(sed_escape_replacement "$SUBSCRIBE_PORT")"
  data="$(yaml_replacement "$DATA_DIR/subscribe-server")"
  web="$(yaml_replacement "$WEB_DIR")"
  patch_line "$file" "s|^  host:.*|  host: '127.0.0.1'|"
  patch_line "$file" "s|^  http_port:.*|  http_port: ${port}|"
  patch_line "$file" "s|^  admin_user:.*|  admin_user: $(yaml_replacement "$ADMIN_USER")|"
  patch_line "$file" "s|^  admin_password:.*|  admin_password: ${password}|"
  patch_line "$file" "s|^  provider_registry_token:.*|  provider_registry_token: $(yaml_replacement "$REGISTRY_TOKEN")|"
  patch_line "$file" "s|^  data_dir:.*|  data_dir: ${data}|"
  patch_line "$file" "s|^  web_dir:.*|  web_dir: ${web}|"
  patch_line "$file" "s|^    url: \"http://127.0.0.1:[0-9][0-9]*/api/v1/nodes\"|    url: \"http://127.0.0.1:$(sed_escape_replacement "$PROVIDER_PORT")/api/v1/nodes\"|"
  patch_line "$file" "s|^    access_token:.*|    access_token: ${token}|"
}

render_client_config() {
  local file="$1" password socks http web stats
  password="$(yaml_replacement "$ADMIN_PASSWORD")"
  socks="$(sed_escape_replacement "$CLIENT_SOCKS_PORT")"
  http="$(sed_escape_replacement "$CLIENT_HTTP_PORT")"
  web="$(sed_escape_replacement "$CLIENT_WEB_PORT")"
  stats="$(yaml_replacement "$DATA_DIR/vpn233-client/stats.json")"
  patch_line "$file" "s|^socks_port:.*|socks_port: ${socks}|"
  patch_line "$file" "s|^http_port:.*|http_port: ${http}|"
  patch_line "$file" "s|^web_addr:.*|web_addr: \"127.0.0.1:${web}\"|"
  patch_line "$file" "s|^admin_user:.*|admin_user: $(yaml_replacement "$ADMIN_USER")|"
  patch_line "$file" "s|^admin_password:.*|admin_password: ${password}|"
  patch_line "$file" "s|^stats_file:.*|stats_file: ${stats}|"
}

create_config_file() {
  local template="$1" destination="$2" renderer="$3" tmp
  tmp="${destination}.tmp.$$"
  cp "$template" "$tmp" || die "failed to read configuration template: $template"
  "$renderer" "$tmp"
  chmod 0600 "$tmp" 2>/dev/null || true
  mv -f "$tmp" "$destination" || die "failed to write configuration: $destination"
}

install_configs() {
  local provider_tpl="$1" subscribe_tpl="$2" client_tpl="$3"
  local created=0
  mkdir -p "$CONFIG_DIR"
  if [[ ! -f "$CONFIG_DIR/provider-server.yaml" ]]; then
    create_config_file "$provider_tpl" "$CONFIG_DIR/provider-server.yaml" render_provider_config
    created=1
  else
    log "  kept existing config: $CONFIG_DIR/provider-server.yaml"
  fi
  if [[ ! -f "$CONFIG_DIR/subscribe-server.yaml" ]]; then
    create_config_file "$subscribe_tpl" "$CONFIG_DIR/subscribe-server.yaml" render_subscribe_config
    created=1
  else
    log "  kept existing config: $CONFIG_DIR/subscribe-server.yaml"
  fi
  if [[ ! -f "$CONFIG_DIR/vpn233-client.yaml" ]]; then
    create_config_file "$client_tpl" "$CONFIG_DIR/vpn233-client.yaml" render_client_config
    created=1
  else
    log "  kept existing config: $CONFIG_DIR/vpn233-client.yaml"
  fi
  if [[ "$created" -eq 1 ]]; then
    GENERATED_CREDS=1
  fi
}

generate_credentials_if_needed() {
  if [[ -z "$ADMIN_PASSWORD" ]]; then
    ADMIN_PASSWORD="$(gen_hex 18)"
  fi
  if [[ -z "$PROVIDER_TOKEN" ]]; then
    PROVIDER_TOKEN="$(gen_hex 24)"
  fi
  if [[ -z "$REGISTRY_TOKEN" ]]; then
    REGISTRY_TOKEN="$(gen_hex 24)"
  fi
  case "$ADMIN_PASSWORD" in root|change-me|change-me-admin-password) die "use a non-default VPN233_ADMIN_PASSWORD or omit it to generate a random password" ;; esac
}

# ---------------------------------------------------------------------------
# Bundle discovery / verification / extraction
# ---------------------------------------------------------------------------
bundle_root_looks_valid() {
  local dir="$1"
  if [[ ! -f "$dir/version.txt" ]]; then
    return 1
  fi
  if [[ -d "$dir/bin" && ( -d "$dir/examples" || -d "$dir/config" ) ]]; then
    return 0
  fi
  return 1
}

find_bundle_root() {
  local dir="$1" candidate
  if bundle_root_looks_valid "$dir"; then
    printf '%s\n' "$dir"
    return 0
  fi
  for candidate in "$dir"/*; do
    if [[ -d "$candidate" ]] && bundle_root_looks_valid "$candidate"; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

detect_bin_extension() {
  local root="$1" name="vpn233-provider-server"
  if [[ -f "$root/bin/$name" ]]; then
    BIN_EXT=""
    return 0
  fi
  if [[ -f "$root/bin/$name.exe" ]]; then
    BIN_EXT=".exe"
    return 0
  fi
  die "bundle is missing bin/$name"
}

validate_bundle_contents() {
  local root="$1" name templates
  templates="$root/examples"
  [[ -d "$templates" ]] || templates="$root/config"
  for name in vpn233-provider-server vpn233-subscribe-server vpn233-client; do
    if [[ ! -f "$root/bin/$name$BIN_EXT" ]]; then
      die "bundle is missing bin/$name$BIN_EXT"
    fi
  done
  if [[ ! -f "$root/admin-web/dist/index.html" ]]; then
    die "bundle is missing admin-web/dist/index.html"
  fi
  if [[ ! -f "$templates/provider-server.yaml" ]]; then
    die "bundle is missing config/provider-server.yaml"
  fi
  if [[ ! -f "$templates/subscribe-server.yaml" ]]; then
    die "bundle is missing config/subscribe-server.yaml"
  fi
  if [[ ! -f "$templates/vpn233-client.yaml" ]]; then
    die "bundle is missing config/vpn233-client.yaml"
  fi
}

read_checksum_sidecar() {
  local bundle="$1" candidate
  for candidate in "${bundle}.sha256" "${bundle}.sha256sum"; do
    if [[ -f "$candidate" ]]; then
      awk 'NF >= 1 { print tolower($1); exit }' "$candidate"
      return 0
    fi
  done
  if [[ -f "$(dirname "$bundle")/SHA256SUMS" ]]; then
    vpn233_sha256_from_sums "$(dirname "$bundle")/SHA256SUMS" "$(basename "$bundle")"
    return 0
  fi
  return 1
}

verify_bundle() {
  local bundle="$1" expected=""
  if [[ ! -f "$bundle" ]]; then
    die "bundle not found: $bundle"
  fi
  if [[ -n "$SHA256_ARG" ]]; then
    expected="$SHA256_ARG"
  elif [[ -n "${VPN233_BUNDLE_SHA256:-}" ]]; then
    expected="$VPN233_BUNDLE_SHA256"
  else
    expected="$(read_checksum_sidecar "$bundle" || true)"
  fi
  if [[ -z "$expected" ]]; then
    if [[ "$ALLOW_UNSIGNED" != "1" ]]; then
      die "no SHA-256 checksum available for $bundle. Put it in ${bundle}.sha256, add it to SHA256SUMS, pass --sha256 HASH, or set VPN233_ALLOW_UNSIGNED=1 to accept the risk."
    fi
    warn "no checksum for offline bundle; continuing because VPN233_ALLOW_UNSIGNED=1"
    return 0
  fi
  if ! vpn233_sha256_matches "$bundle" "$expected"; then
    die "SHA-256 verification failed for $bundle; refusing to install and keeping the current version."
  fi
  log "  SHA-256 verified: $(basename "$bundle")"
}

extract_bundle() {
  local bundle="$1" destination="$2" entries details
  mkdir -p "$destination"
  entries="$TEMP_DIR/archive-entries"
  details="$TEMP_DIR/archive-details"
  if [[ "$bundle" == *.zip ]]; then
    require_command unzip
    unzip -Z1 "$bundle" > "$entries" || die "could not inspect zip bundle"
    unzip -Z -l "$bundle" > "$details" || die "could not inspect zip entry types"
    if awk '/^[lbcps]/ { found=1 } END { exit(found ? 0 : 1) }' "$details"; then die "archive contains symbolic links or special files"; fi
  else
    tar -tzf "$bundle" > "$entries" || die "could not inspect bundle"
    tar -tvzf "$bundle" > "$details" || die "could not inspect bundle entry types"
    if awk 'substr($0,1,1) != "-" && substr($0,1,1) != "d" { found=1 } END { exit(found ? 0 : 1) }' "$details"; then die "archive contains links or special files"; fi
  fi
  if awk '/(^[/\\]|(^|[/\\])\.\.([/\\]|$)|:)/ { found=1 } END { exit(found ? 0 : 1) }' "$entries"; then
    die "archive contains an unsafe extraction path"
  fi
  case "$bundle" in
    *.zip)
      if command -v unzip >/dev/null 2>&1; then
        unzip -q "$bundle" -d "$destination" || die "failed to extract $bundle"
      else
        die "cannot extract .zip bundles: install unzip"
      fi
      ;;
    *.tar.gz|*.tgz)
      tar -xzf "$bundle" -C "$destination" || die "failed to extract $bundle"
      ;;
    *)
      die "unsupported bundle format: $bundle (expected .tar.gz or .zip)"
      ;;
  esac
}

download_release_bundle() {
  local temp="$1" platform asset expects sums_file
  platform="$(detect_platform)"
  asset="$(vpn233_release_asset_name "$platform")"
  log "  platform: $platform"
  log "  asset   : $asset"
  if [[ -n "$VERSION" ]]; then
    log "  version : $(vpn233_normalize_version "$VERSION")"
  else
    log "  version : latest"
    if vpn233_download_release_asset "version.txt" "$temp/version.txt" "" 1; then
      BUNDLE_VERSION="$(sanitize_version "$(head -n 1 "$temp/version.txt" | tr -d '\r')")"
      VERSION="$BUNDLE_VERSION"
      log "  latest release version: $BUNDLE_VERSION"
    else
      die "could not resolve the latest release version; specify --version"
    fi
  fi
  sums_file="$temp/SHA256SUMS"
  if ! vpn233_download_release_asset "SHA256SUMS" "$sums_file" "$VERSION" 0; then
    die "could not download SHA256SUMS; refusing to install an unverified release bundle"
  fi
  expects="$(vpn233_sha256_from_sums "$sums_file" "$asset" || true)"
  if [[ -z "$expects" ]]; then
    die "SHA256SUMS does not list $asset; refusing to install (fail closed)"
  fi
  if ! vpn233_download_release_asset "$asset" "$temp/$asset" "$VERSION" 0 "$expects"; then
    die "could not download and verify release asset: $asset"
  fi
  if ! vpn233_sha256_matches "$temp/$asset" "$expects"; then
    die "SHA-256 verification failed for $asset; refusing to install and keeping the current version."
  fi
  log "  SHA-256 verified: $asset"
  BUNDLE="$temp/$asset"
  if [[ -n "$VERSION" ]]; then
    BUNDLE_VERSION="$(sanitize_version "$VERSION")"
  fi
}

acquire_bundle() {
  local temp="$1" root
  if [[ -n "$BUNDLE" ]]; then
    [[ ! -d "$BUNDLE" ]] || die "use a checksum-verified archive for --bundle, not an unpacked directory"
    log "  offline bundle: $BUNDLE"
    verify_bundle "$BUNDLE"
    extract_bundle "$BUNDLE" "$temp/extract"
    root="$(find_bundle_root "$temp/extract" || true)"
    if [[ -z "$root" ]]; then
      die "extracted bundle does not contain bin/, config/ and version.txt"
    fi
    BUNDLE_ROOT="$root"
    return 0
  fi
  download_release_bundle "$temp"
  extract_bundle "$BUNDLE" "$temp/extract"
  root="$(find_bundle_root "$temp/extract" || true)"
  if [[ -z "$root" ]]; then
    die "downloaded bundle does not contain bin/, config/ and version.txt"
  fi
  BUNDLE_ROOT="$root"
}

# ---------------------------------------------------------------------------
# File installation
# ---------------------------------------------------------------------------
install_binary() {
  local source="$1" destination_dir="$2" name="$3" tmp
  mkdir -p "$destination_dir"
  tmp="$destination_dir/.${name}.tmp.$$"
  cp "$source" "$tmp" || die "failed to copy binary: $source"
  chmod 0755 "$tmp" 2>/dev/null || true
  mv -f "$tmp" "$destination_dir/$name" || die "failed to install binary: $name"
}

install_binaries() {
  local root="$1" name
  for name in vpn233-provider-server vpn233-subscribe-server vpn233-client; do
    install_binary "$root/bin/$name$BIN_EXT" "$BIN_DIR" "$name$BIN_EXT"
    log "  installed $BIN_DIR/$name$BIN_EXT"
  done
}

install_web() {
  local root="$1" source staging old parent
  if [[ -d "$root/admin-web/dist" ]]; then
    source="$root/admin-web/dist"
  else
    source="$root/admin-web"
  fi
  if [[ ! -f "$source/index.html" ]]; then
    die "bundle admin-web build is missing index.html"
  fi
  parent="$(dirname "$WEB_DIR")"
  mkdir -p "$parent"
  staging="$parent/.dist.new.$$"
  old="$parent/.dist.old.$$"
  WEB_NEW="$staging"
  rm -rf "$staging" 2>/dev/null || true
  rm -rf "$old" 2>/dev/null || true
  mkdir -p "$staging"
  cp -a "$source/." "$staging/" || die "failed to stage admin web files"
  if [[ -d "$WEB_DIR" ]]; then
    mv "$WEB_DIR" "$old" || die "failed to move the current admin web build"
    WEB_PREVIOUS="$old"
  fi
  if ! mv "$staging" "$WEB_DIR"; then
    [[ ! -d "$old" ]] || mv "$old" "$WEB_DIR"
    die "failed to activate the admin web build"
  fi
  log "  installed admin web: $WEB_DIR"
}

prepare_payload_rollback() {
  local name library result
  mkdir -p "$TEMP_DIR/rollback"
  for name in vpn233-provider-server vpn233-subscribe-server vpn233-client; do
    [[ ! -f "$BIN_DIR/$name$BIN_EXT" ]] || cp "$BIN_DIR/$name$BIN_EXT" "$TEMP_DIR/rollback/$name$BIN_EXT"
  done
  [[ ! -f "$BASE_DIR/version.txt" ]] || cp "$BASE_DIR/version.txt" "$TEMP_DIR/rollback/version.txt"
  ROLLBACK_ACTIVE=1
  for library in "$BUNDLE_ROOT/scripts/lib/services.sh" "$SCRIPT_DIR/scripts/lib/services.sh"; do
    if [[ -f "$library" ]]; then
      # shellcheck disable=SC1090
      . "$library"
      if vpn233_manage_services stop; then :
      else result=$?; [[ "$result" -eq 1 ]] || die "could not stop owned services"; fi
      break
    fi
  done
  if declare -F vpn233_stop_owned_processes >/dev/null 2>&1; then
    vpn233_stop_owned_processes "$BASE_DIR" || die "could not stop previous owned processes"
  elif [[ -f "$BASE_DIR/stop.sh" ]]; then
    die "release bundle is missing the ownership helper required to stop the previous installation safely"
  fi
}

write_version_file() {
  local version="$1" destination="$BASE_DIR/version.txt" tmp
  tmp="${destination}.tmp.$$"
  printf '%s\n' "$version" > "$tmp" || die "failed to write version file"
  mv -f "$tmp" "$destination" || die "failed to update version file"
  log "  installed version: $version"
}

copy_runtime_scripts() {
  local root="$1" relative
  if ! bundle_root_looks_valid "$root"; then
    return 0
  fi
  for relative in \
    install.sh update.sh deploy.sh uninstall.sh start.sh stop.sh \
    install.ps1 update.ps1 deploy.ps1 uninstall.ps1 start.ps1 stop.ps1; do
    if [[ -f "$root/$relative" ]]; then
      cp -f "$root/$relative" "$BASE_DIR/$relative" || die "failed to copy $relative"
      case "$relative" in
        *.sh) chmod 0755 "$BASE_DIR/$relative" 2>/dev/null || true ;;
      esac
    fi
  done
  if [[ -d "$root/lib" ]]; then
    mkdir -p "$BASE_DIR/lib"
    cp -f "$root/lib/"* "$BASE_DIR/lib/" 2>/dev/null || true
  fi
  if [[ -d "$root/scripts/lib" ]]; then
    mkdir -p "$BASE_DIR/scripts/lib"
    cp -f "$root/scripts/lib/"* "$BASE_DIR/scripts/lib/" 2>/dev/null || true
  fi
  if [[ -n "$LAYOUT_LIBRARY" ]]; then
    mkdir -p "$BASE_DIR/scripts/lib"
    [[ "$LAYOUT_LIBRARY" == "$BASE_DIR/scripts/lib/layout.sh" ]] || cp -f "$LAYOUT_LIBRARY" "$BASE_DIR/scripts/lib/layout.sh"
  fi
}

# ---------------------------------------------------------------------------
# Service management
# ---------------------------------------------------------------------------
service_manager() {
  local os
  os="$(uname -s)"
  case "$os" in
    Linux*)
      if command -v systemctl >/dev/null 2>&1; then
        if [[ "$SYSTEM" -eq 1 ]] && [[ -d /run/systemd/system ]]; then
          printf 'systemd-system\n'
          return 0
        fi
        if [[ "$SYSTEM" -eq 0 ]] && systemctl --user show-environment >/dev/null 2>&1; then
          printf 'systemd-user\n'
          return 0
        fi
      fi
      ;;
    Darwin*)
      printf 'launchd\n'
      return 0
      ;;
  esac
  printf 'none\n'
}

service_unit_names() {
  printf '%s' 'provider subscribe client'
}

service_binary_name() {
  case "$1" in
    provider) printf '%s' 'vpn233-provider-server' ;;
    subscribe) printf '%s' 'vpn233-subscribe-server' ;;
    client) printf '%s' 'vpn233-client' ;;
  esac
}

service_config_file() {
  case "$1" in
    provider) printf '%s' 'provider-server.yaml' ;;
    subscribe) printf '%s' 'subscribe-server.yaml' ;;
    client) printf '%s' 'vpn233-client.yaml' ;;
  esac
}

write_systemd_unit() {
  local unit_dir="$1" unit_scope="$2" service="$3"
  local binary config file wanted_by service_user
  binary="$(service_binary_name "$service")"
  config="$(service_config_file "$service")"
  file="$unit_dir/vpn233-${service}.service"
  service_user="${VPN233_SERVICE_USER:-}"
  [[ -z "$service_user" || "$service_user" =~ ^[A-Za-z_][A-Za-z0-9_-]*\$?$ ]] || die "invalid VPN233_SERVICE_USER"
  if [[ -e "$file" ]]; then
    declare -F vpn233_unit_owned >/dev/null 2>&1 && vpn233_unit_owned "$file" "$service" || die "refusing to overwrite a service unit owned by another installation"
  fi
  if [[ "$unit_scope" == "system" ]]; then
    wanted_by="multi-user.target"
  else
    wanted_by="default.target"
  fi
  mkdir -p "$unit_dir"
  {
    printf '[Unit]\n'
    printf 'Description=VPN233 %s service\n' "$service"
    printf 'After=network-online.target\n'
    printf 'Wants=network-online.target\n'
    printf '\n'
    printf '[Service]\n'
    printf 'Type=simple\n'
    # This setting is a single path, not an ExecStart argument list: systemd
    # retains enclosing quotes and rejects the resulting non-absolute path.
    # The trailing /. also preserves a literal trailing space or backslash.
    printf 'WorkingDirectory=%s/.\n' "${BASE_DIR//\%/%%}"
    printf 'ExecStart=%s --config %s\n' "$(vpn233_unit_quote "$BIN_DIR/$binary$BIN_EXT")" "$(vpn233_unit_quote "$CONFIG_DIR/$config")"
    printf 'Restart=on-failure\n'
    printf 'RestartSec=3\n'
    if [[ -n "$service_user" && "$unit_scope" == "system" ]]; then
      printf 'User=%s\n' "$service_user"
    fi
    printf 'NoNewPrivileges=true\n'
    printf 'PrivateTmp=true\n'
    printf '\n'
    printf '[Install]\n'
    printf 'WantedBy=%s\n' "$wanted_by"
  } > "$file" || die "failed to write systemd unit: $file"
  log "  wrote $file"
}

write_launchd_plist() {
  local plist_dir="$1" service="$2"
  local binary config file
  binary="$(service_binary_name "$service")"
  config="$(service_config_file "$service")"
  file="$plist_dir/com.neko233.vpn233.${service}.plist"
  if [[ -e "$file" ]]; then
    declare -F vpn233_plist_owned >/dev/null 2>&1 && vpn233_plist_owned "$file" "$service" || die "refusing to overwrite a launchd unit owned by another installation"
  fi
  mkdir -p "$plist_dir"
  {
    printf '<?xml version="1.0" encoding="UTF-8"?>\n'
    printf '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n'
    printf '<plist version="1.0">\n'
    printf '<dict>\n'
    printf '  <key>Label</key>\n'
    printf '  <string>com.neko233.vpn233.%s</string>\n' "$service"
    printf '  <key>ProgramArguments</key>\n'
    printf '  <array>\n'
    printf '    <string>%s</string>\n' "$(vpn233_xml_escape "$BIN_DIR/$binary$BIN_EXT")"
    printf '    <string>--config</string>\n'
    printf '    <string>%s</string>\n' "$(vpn233_xml_escape "$CONFIG_DIR/$config")"
    printf '  </array>\n'
    printf '  <key>WorkingDirectory</key>\n'
    printf '  <string>%s</string>\n' "$(vpn233_xml_escape "$BASE_DIR")"
    printf '  <key>RunAtLoad</key>\n'
    printf '  <true/>\n'
    printf '  <key>KeepAlive</key>\n'
    printf '  <dict>\n'
    printf '    <key>SuccessfulExit</key>\n'
    printf '    <false/>\n'
    printf '  </dict>\n'
    printf '  <key>StandardOutPath</key>\n'
    printf '  <string>%s</string>\n' "$(vpn233_xml_escape "$LOG_DIR/$service.log")"
    printf '  <key>StandardErrorPath</key>\n'
    printf '  <string>%s</string>\n' "$(vpn233_xml_escape "$LOG_DIR/$service.log")"
    printf '</dict>\n'
    printf '</plist>\n'
  } > "$file" || die "failed to write launchd plist: $file"
  log "  wrote $file"
}

install_systemd_services() {
  local scope="$1" unit_dir service units=""
  if [[ "$scope" == "system" ]]; then
    if [[ "$(id -u)" -ne 0 ]]; then
      die "system mode requires root: re-run with sudo"
    fi
    unit_dir="/etc/systemd/system"
  else
    unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
  fi
  for service in $(service_unit_names); do
    write_systemd_unit "$unit_dir" "$scope" "$service"
    units="$units vpn233-${service}.service"
  done
  if [[ "$scope" == "system" ]]; then
    systemctl daemon-reload || die "systemctl daemon-reload failed"
    if [[ "$NO_START" -eq 1 ]]; then
      # shellcheck disable=SC2086
      systemctl enable $units || die "systemctl enable failed"
    else
      # shellcheck disable=SC2086
      systemctl enable --now $units || die "systemctl enable --now failed; inspect private logs locally"
      INSTALL_STARTED=1
    fi
  else
    systemctl --user daemon-reload || die "systemctl --user daemon-reload failed"
    if [[ "$NO_START" -eq 1 ]]; then
      # shellcheck disable=SC2086
      systemctl --user enable $units || die "systemctl --user enable failed"
    else
      # shellcheck disable=SC2086
      systemctl --user enable --now $units || die "systemctl --user enable --now failed"
      INSTALL_STARTED=1
    fi
  fi
}

install_launchd_services() {
  local plist_dir service plist
  if [[ "$SYSTEM" -eq 1 ]]; then
    plist_dir="/Library/LaunchDaemons"
  else
    plist_dir="${HOME}/Library/LaunchAgents"
  fi
  for service in $(service_unit_names); do
    write_launchd_plist "$plist_dir" "$service"
    if [[ "$NO_START" -eq 0 ]]; then
      plist="$plist_dir/com.neko233.vpn233.${service}.plist"
      if [[ "$SYSTEM" -eq 1 ]]; then
        launchctl load -w "$plist" || die "launchctl load failed (try system mode with sudo)"
      else
        launchctl load -w "$plist" || die "launchctl load failed"
      fi
      INSTALL_STARTED=1
    fi
  done
}

start_with_scripts() {
  if [[ "$NO_START" -eq 1 ]]; then
    log "  --no-start: services were not started"
    return 0
  fi
  if [[ "$SYSTEM" -eq 1 ]]; then
    die "system mode requires a supported service manager; use --no-start and configure the service locally"
  fi
  if [[ -x "$BASE_DIR/start.sh" ]]; then
    log "  no service manager detected; starting services with start.sh"
    if VPN233_BIN_DIR="$BIN_DIR" VPN233_CONFIG_DIR="$CONFIG_DIR" bash "$BASE_DIR/start.sh"; then
      INSTALL_STARTED=1
    else
      die "start.sh reported an error; inspect private logs locally"
    fi
  else
    die "no service manager detected and start.sh is missing; rerun with --no-start or use the full bundle"
  fi
}

install_services() {
  local manager
  manager="$(service_manager)"
  log "  service manager: $manager"
  if [[ "$manager" != none ]]; then
    declare -F vpn233_unit_quote >/dev/null 2>&1 || die "release bundle is missing scripts/lib/services.sh; refusing incomplete service setup"
  fi
  case "$manager" in
    systemd-system) install_systemd_services system ;;
    systemd-user) install_systemd_services user ;;
    launchd) install_launchd_services ;;
    *) start_with_scripts ;;
  esac
}

# ---------------------------------------------------------------------------
# Health checks
# ---------------------------------------------------------------------------
tail_logs() {
  local file found=0
  printf '--- recent log output ---\n'
  for file in "$LOG_DIR"/*.log; do
    if [[ -f "$file" ]]; then
      found=1
      printf '== %s ==\n' "$file"
      tail -n 15 "$file" 2>/dev/null || true
    fi
  done
  if [[ "$found" -eq 0 ]]; then
    printf 'no log files found in %s\n' "$LOG_DIR"
  fi
}

poll_health() {
  local url="$1" attempts=20 delay=1 counter=0
  if ! command -v curl >/dev/null 2>&1; then
    warn "curl is unavailable for health checks"
    return 1
  fi
  while [[ "$counter" -lt "$attempts" ]]; do
    if curl --fail --silent --show-error --max-time 2 --output /dev/null "$url" 2>/dev/null; then
      return 0
    fi
    counter=$((counter + 1))
    sleep "$delay"
  done
  return 1
}

run_health_checks() {
  if [[ "$SKIP_HEALTHCHECK" == "1" ]]; then
    log "  health checks skipped (VPN233_SKIP_HEALTHCHECK=1)"
    return 0
  fi
  if [[ "$INSTALL_STARTED" -eq 0 ]]; then
    return 0
  fi
  local failures=0
  if ! poll_health "http://127.0.0.1:${PROVIDER_PORT}/api/v1/health"; then
    warn "provider-server health check failed on port ${PROVIDER_PORT}"
    failures=$((failures + 1))
  fi
  if ! poll_health "http://127.0.0.1:${SUBSCRIBE_PORT}/api/status"; then
    warn "subscribe-server health check failed on port ${SUBSCRIBE_PORT}"
    failures=$((failures + 1))
  fi
  if ! poll_health "http://127.0.0.1:${CLIENT_WEB_PORT}/api/v1/health"; then
    warn "vpn233-client health check failed on port ${CLIENT_WEB_PORT}"
    failures=$((failures + 1))
  fi
  if [[ "$failures" -gt 0 ]]; then
    die "installed service health checks failed; inspect private logs locally"
  else
    log "  health checks passed"
  fi
  return 0
}

# ---------------------------------------------------------------------------
# Summary output
# ---------------------------------------------------------------------------
print_summary() {
  printf '\n'
  printf 'VPN233 installed successfully.\n'
  printf '%s\n' "  version       : $BUNDLE_VERSION"
  printf '%s\n' "  admin web     : http://127.0.0.1:${SUBSCRIBE_PORT}/"
  printf '%s\n' "  provider API  : http://127.0.0.1:${PROVIDER_PORT}/api/v1/health"
  printf '%s\n' "  client stats  : http://127.0.0.1:${CLIENT_WEB_PORT}/api/v1/stats"
  printf '%s\n' "  config        : $CONFIG_DIR"
  printf '%s\n' "  logs          : $LOG_DIR"
  printf '%s\n' "  next update   : bash $BASE_DIR/update.sh --check"
  printf '%s\n' "  uninstall     : bash $BASE_DIR/uninstall.sh"
  printf '\n'
  if [[ "$GENERATED_CREDS" -eq 1 ]]; then
    printf ' Random credentials are stored only in the private config files.\n'
    printf ' Inspect them locally with your editor. The config files\n'
    printf ' are mode 0600 and are not overwritten on reinstall/update.\n'
    printf '============================================================\n'
  else
    printf 'Existing configuration and credentials were preserved.\n'
  fi
}

# ---------------------------------------------------------------------------
# Source checkout mode (legacy compatible)
# ---------------------------------------------------------------------------
find_source_binaries() {
  local candidate name valid
  if [[ -n "${VPN233_SOURCE_BIN_DIR:-}" ]]; then
    for name in vpn233-provider-server vpn233-subscribe-server vpn233-client; do
      [[ -x "${VPN233_SOURCE_BIN_DIR}/$name$BIN_EXT" ]] || die "VPN233_SOURCE_BIN_DIR is missing an executable: $name$BIN_EXT"
    done
    printf '%s\n' "$VPN233_SOURCE_BIN_DIR"
    return 0
  fi
  for candidate in "$SCRIPT_DIR/bin" "$SCRIPT_DIR/generated" "$SCRIPT_DIR/target/release"; do
    valid=1
    for name in vpn233-provider-server vpn233-subscribe-server vpn233-client; do [[ -x "$candidate/$name$BIN_EXT" ]] || valid=0; done
    if [[ "$valid" -eq 1 ]]; then printf '%s\n' "$candidate"; return 0; fi
  done
  return 1
}

find_source_web() {
  local source
  if [[ -d "$SCRIPT_DIR/admin-web/dist" && -f "$SCRIPT_DIR/admin-web/dist/index.html" ]]; then
    printf '%s\n' "$SCRIPT_DIR/admin-web/dist"
    return 0
  fi
  return 1
}

source_config_template() {
  local config="$1" candidate
  candidate="$SCRIPT_DIR/$config/config.example.yaml"
  if [[ -f "$candidate" ]]; then printf '%s\n' "$candidate"; return 0; fi
  return 1
}

source_install() {
  local bin_source web_source build_required=0 needs_web_build=0 config destination template
  if is_windows_shell; then BIN_EXT=".exe"; fi
  log "VPN233 source checkout install"
  log "  binaries : $BIN_DIR"
  log "  admin web: $WEB_DIR"
  log "  config   : $CONFIG_DIR"
  if ! bin_source="$(find_source_binaries)"; then
    if [[ -n "${VPN233_SOURCE_BIN_DIR:-}" ]]; then
      bin_source="$VPN233_SOURCE_BIN_DIR"
    else
      build_required=1
      bin_source="$SCRIPT_DIR/target/release"
    fi
  fi
  if ! web_source="$(find_source_web)"; then
    needs_web_build=1
    web_source="$SCRIPT_DIR/admin-web/dist"
  fi
  if [[ "$DRY_RUN" -eq 1 ]]; then
    printf 'DRY RUN: source checkout install plan\n'
    if [[ "$build_required" -eq 1 ]]; then
      printf '  would run: cargo build --release --workspace (no prebuilt binaries found)\n'
    else
      printf '  would install binaries from: %s\n' "$bin_source"
    fi
    if [[ "$needs_web_build" -eq 1 ]]; then
      printf '  would run: npm ci && npm run build in admin-web\n'
    else
      printf '  would install admin web from: %s\n' "$web_source"
    fi
    printf '  would create: %s %s %s\n' "$BIN_DIR" "$WEB_DIR" "$CONFIG_DIR"
    printf '  would copy config templates without touching existing files\n'
    return 0
  fi
  if [[ "$build_required" -eq 1 ]]; then
    if ! command -v cargo >/dev/null 2>&1; then
      die "no prebuilt Rust binaries found and cargo is unavailable; install Rust or set VPN233_SOURCE_BIN_DIR"
    fi
    log "  no prebuilt binaries found; running cargo build --release --workspace"
    ( cd "$SCRIPT_DIR" && cargo build --release --workspace ) || die "cargo build failed"
    bin_source="$SCRIPT_DIR/target/release"
  fi
  if [[ "$needs_web_build" -eq 1 ]]; then
    if ! command -v npm >/dev/null 2>&1; then
      die "no admin web build found and npm is unavailable; provide admin-web/dist or install Node.js"
    fi
    log "  building admin web with npm"
    ( cd "$SCRIPT_DIR/admin-web" && npm ci && npm run build ) || die "admin web build failed"
    web_source="$SCRIPT_DIR/admin-web/dist"
  fi
  mkdir -p "$BIN_DIR"
  install_binary "$bin_source/vpn233-provider-server$BIN_EXT" "$BIN_DIR" "vpn233-provider-server$BIN_EXT"
  install_binary "$bin_source/vpn233-subscribe-server$BIN_EXT" "$BIN_DIR" "vpn233-subscribe-server$BIN_EXT"
  install_binary "$bin_source/vpn233-client$BIN_EXT" "$BIN_DIR" "vpn233-client$BIN_EXT"
  log "  installed Rust binaries in $BIN_DIR"
  if [[ "$web_source" != "$WEB_DIR" ]]; then
    mkdir -p "$WEB_DIR"
    cp -a "$web_source/." "$WEB_DIR/" || die "failed to copy admin web files"
  fi
  log "  installed admin web in $WEB_DIR"
  mkdir -p "$CONFIG_DIR" "$DATA_DIR/subscribe-server" "$DATA_DIR/vpn233-client"
  chmod 0700 "$CONFIG_DIR" "$DATA_DIR"
  resolve_release_ports_and_credentials
  install_configs "$(source_config_template provider-server)" "$(source_config_template subscribe-server)" "$(source_config_template vpn233-client)"
  # shellcheck source=scripts/lib/layout.sh
  . "$SCRIPT_DIR/scripts/lib/layout.sh"
  vpn233_layout_save "$BASE_DIR" source "$BIN_DIR" "$WEB_DIR" "$CONFIG_DIR" "$DATA_DIR" || die "failed to save private runtime layout"
  log "Done. Existing configuration files were preserved."
}

# ---------------------------------------------------------------------------
# Release mode
# ---------------------------------------------------------------------------
resolve_release_ports_and_credentials() {
  local needs_creds=0
  if [[ ! -f "$CONFIG_DIR/provider-server.yaml" || ! -f "$CONFIG_DIR/subscribe-server.yaml" || ! -f "$CONFIG_DIR/vpn233-client.yaml" ]]; then
    needs_creds=1
  fi
  if [[ "$needs_creds" -eq 1 ]]; then
    generate_credentials_if_needed
  fi
  resolve_ports
}

print_release_plan() {
  local platform asset
  printf 'DRY RUN: no files, services or downloads will be changed\n'
  if [[ -n "$BUNDLE" ]]; then
    printf '  would install offline bundle: %s\n' "$BUNDLE"
  elif bundle_root_looks_valid "$SCRIPT_DIR"; then
    printf '  would install the bundle unpacked next to the installer: %s\n' "$SCRIPT_DIR"
  else
    platform="$(detect_platform)"
    asset="$(vpn233_release_asset_name "$platform")"
    printf '  would download release asset through the mirror list: %s\n' "$asset"
    if [[ -n "$VERSION" ]]; then
      printf '  pinned version: %s\n' "$(vpn233_normalize_version "$VERSION")"
    else
      printf '  version: latest\n'
    fi
    printf '  would verify SHA-256 against SHA256SUMS (fail closed)\n'
  fi
  printf '  planned layout:\n'
  print_layout
  printf '  planned ports: provider=%s subscribe=%s client-socks=%s client-http=%s client-web=%s\n' \
    "${VPN233_PROVIDER_PORT:-8080}" "${VPN233_SUBSCRIBE_PORT:-23444}" "${VPN233_CLIENT_SOCKS_PORT:-1080}" \
    "${VPN233_CLIENT_HTTP_PORT:-1081}" "${VPN233_CLIENT_WEB_PORT:-8082}"
  if [[ ! -f "$CONFIG_DIR/provider-server.yaml" || ! -f "$CONFIG_DIR/subscribe-server.yaml" || ! -f "$CONFIG_DIR/vpn233-client.yaml" ]]; then
    printf '  would generate random admin password, provider feed token and registry token (printed once)\n'
  else
    printf '  existing configuration would be preserved (credentials unchanged)\n'
  fi
  printf '  would register a service with the detected service manager and start it unless --no-start\n'
}

release_install() {
  local root templates
  resolve_layout
  [[ -z "$VERSION" ]] || VERSION="$(sanitize_version "$VERSION")"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    # Dry-run is also usable with a standalone installer and never downloads helpers.
    [[ -z "$VERSION" ]] || VERSION="$(sanitize_version "$VERSION")"
    printf 'DRY RUN: no files, services or downloads will be changed\n'
    print_layout
    return 0
  fi
  if ! load_mirror_library; then
    die "mirror library not found (scripts/lib/mirrors.sh or lib/mirrors.sh next to install.sh); use deploy.sh or include lib/ in the bundle"
  fi
  [[ -z "$MIRROR_VALUE" ]] || vpn233_mirror_set_mode "$MIRROR_VALUE"
  [[ -z "$VERSION" ]] || VERSION="$(sanitize_version "$VERSION")"
  if [[ "$SYSTEM" -eq 1 ]] && [[ "$(id -u)" -ne 0 ]]; then
    die "system mode installs into /opt/vpn233, /etc/vpn233 and /var/lib/vpn233; re-run with sudo"
  fi
  log "VPN233 release installer"
  make_temp_dir
  acquire_bundle "$TEMP_DIR"
  root="$BUNDLE_ROOT"
  resolve_layout
  detect_bin_extension "$root"
  validate_bundle_contents "$root"
  BUNDLE_VERSION="$(sanitize_version "$(head -n 1 "$root/version.txt" | tr -d '\r')")"
  if [[ -z "$BUNDLE_VERSION" ]]; then
    die "bundle version.txt is empty"
  fi
  [[ -z "$VERSION" || "$BUNDLE_VERSION" == "$(sanitize_version "$VERSION")" ]] || die "bundle version does not match the requested release"
  log "  bundle version: $BUNDLE_VERSION"
  log "  installing into: $BASE_DIR"
  resolve_release_ports_and_credentials
  mkdir -p "$BASE_DIR" "$BIN_DIR" "$WEB_DIR" "$CONFIG_DIR" "$DATA_DIR" "$LOG_DIR" "$BACKUP_DIR"
  chmod 0700 "$CONFIG_DIR" "$DATA_DIR" "$LOG_DIR" "$BACKUP_DIR"
  prepare_payload_rollback
  install_binaries "$root"
  install_web "$root"
  write_version_file "$BUNDLE_VERSION"
  templates="$root/examples"
  [[ -d "$templates" ]] || templates="$root/config"
  install_configs "$templates/provider-server.yaml" "$templates/subscribe-server.yaml" "$templates/vpn233-client.yaml"
  mkdir -p "$DATA_DIR/subscribe-server" "$DATA_DIR/vpn233-client"
  copy_runtime_scripts "$root"
  install_services
  run_health_checks
  vpn233_layout_save "$BASE_DIR" "$LAYOUT_SCOPE" "$BIN_DIR" "$WEB_DIR" "$CONFIG_DIR" "$DATA_DIR" || die "failed to save private runtime layout"
  ROLLBACK_ACTIVE=0
  print_summary
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
resolve_layout
if [[ "$MODE" == "source" ]]; then
  source_install
else
  release_install
fi
