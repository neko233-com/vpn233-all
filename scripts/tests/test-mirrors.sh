#!/usr/bin/env bash
# shellcheck shell=bash
#
# Offline unit tests for scripts/lib/mirrors.sh.
#
# The tests never touch the network: _vpn233_fetch_url is replaced with an
# in-process fake that records every attempted URL. Run with:
#   bash scripts/tests/test-mirrors.sh
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/mirrors.sh
. "$SCRIPT_DIR/../lib/mirrors.sh"

PASS_COUNT=0
FAIL_COUNT=0
FAIL_NAMES=""

pass() {
  PASS_COUNT=$((PASS_COUNT + 1))
  printf '[PASS] %s\n' "$1"
}

fail() {
  FAIL_COUNT=$((FAIL_COUNT + 1))
  FAIL_NAMES="${FAIL_NAMES}${1}
"
  printf '[FAIL] %s\n' "$1" >&2
  if [[ $# -ge 3 ]]; then
    printf '       expected: %s\n' "$2" >&2
    printf '       actual  : %s\n' "$3" >&2
  fi
}

assert_eq() {
  local name="$1" expected="$2" actual="$3"
  if [[ "$expected" == "$actual" ]]; then
    pass "$name"
  else
    fail "$name" "$expected" "$actual"
  fi
}

assert_ok() {
  local name="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    pass "$name"
  else
    fail "$name" "success" "non-zero exit"
  fi
}

assert_fail() {
  local name="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    fail "$name" "non-zero exit" "success"
  else
    pass "$name"
  fi
}

# Join a prefix list into "[a]|[b]|[]" so an empty direct prefix stays visible.
join_prefixes() {
  local kind="$1" out="" sep="" prefix
  while IFS= read -r prefix; do
    out="${out}${sep}[${prefix}]"
    sep='|'
  done < <(vpn233_mirror_list "$kind")
  printf '%s\n' "$out"
}

reset_env() {
  unset VPN233_GH_MIRROR || true
  unset VPN233_GH_MIRRORS || true
  unset VPN233_NO_MIRROR || true
  unset VPN233_MIRROR_MODE || true
  unset VPN233_HOME || true
  unset VPN233_LAST_MIRROR || true
}

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/vpn233-mirror-test.XXXXXX")"
cleanup() {
  rm -rf "$WORK_DIR" 2>/dev/null || true
}
trap cleanup EXIT

reset_env

# ---------------------------------------------------------------------------
# 1. Contract anchors and default prefix order
# ---------------------------------------------------------------------------
assert_eq "contract anchor release prefixes" \
  "https://ghproxy.net/ https://gh.llkk.cc/ https://ghfast.top/ https://gh-proxy.com/ " \
  "$VPN233_GH_MIRROR_PREFIXES_DEFAULT"
assert_eq "contract anchor raw prefixes" \
  "https://ghproxy.net/ https://gh.llkk.cc/ https://ghfast.top/ https://gh-proxy.com/ " \
  "$VPN233_GH_RAW_PREFIXES_DEFAULT"
assert_eq "default release prefix order" \
  "[https://ghproxy.net/]|[https://gh.llkk.cc/]|[https://ghfast.top/]|[https://gh-proxy.com/]|[]" \
  "$(join_prefixes release)"
assert_eq "default raw prefix order" \
  "[https://ghproxy.net/]|[https://gh.llkk.cc/]|[https://ghfast.top/]|[https://gh-proxy.com/]|[]" \
  "$(join_prefixes raw)"
assert_eq "default mode is auto" "auto" "$(vpn233_mirror_mode)"

# ---------------------------------------------------------------------------
# 2. URL construction
# ---------------------------------------------------------------------------
assert_eq "latest release asset url" \
  "https://github.com/neko233-com/vpn233-all/releases/latest/download/vpn233-rust-linux-x86_64.tar.gz" \
  "$(vpn233_github_release_url "vpn233-rust-linux-x86_64.tar.gz")"
assert_eq "pinned release asset url" \
  "https://github.com/neko233-com/vpn233-all/releases/download/v2.1.0/vpn233-rust-windows-x86_64.zip" \
  "$(vpn233_github_release_url "vpn233-rust-windows-x86_64.zip" "2.1.0")"
assert_eq "pinned release asset url keeps v prefix" \
  "https://github.com/neko233-com/vpn233-all/releases/download/v2.1.0/vpn233-rust-windows-x86_64.zip" \
  "$(vpn233_github_release_url "vpn233-rust-windows-x86_64.zip" "v2.1.0")"
assert_eq "raw github url" \
  "https://raw.githubusercontent.com/neko233-com/vpn233-all/main/deploy.sh" \
  "$(vpn233_github_raw_url "deploy.sh")"
assert_eq "raw github url with custom ref" \
  "https://raw.githubusercontent.com/neko233-com/vpn233-all/v2.1.0/scripts/lib/mirrors.sh" \
  "$(vpn233_github_raw_url "scripts/lib/mirrors.sh" "v2.1.0")"
assert_eq "jsdelivr raw url" \
  "https://cdn.jsdelivr.net/gh/neko233-com/vpn233-all@main/install.sh" \
  "$(vpn233_jsdelivr_raw_url "install.sh")"
assert_eq "mirror apply prefixes url" \
  "https://ghproxy.net/https://github.com/neko233-com/vpn233-all/releases/latest/download/SHA256SUMS" \
  "$(vpn233_mirror_apply "https://ghproxy.net/" "https://github.com/neko233-com/vpn233-all/releases/latest/download/SHA256SUMS")"
assert_eq "mirror apply direct keeps url" \
  "https://github.com/neko233-com/vpn233-all/releases/latest/download/SHA256SUMS" \
  "$(vpn233_mirror_apply "" "https://github.com/neko233-com/vpn233-all/releases/latest/download/SHA256SUMS")"
assert_eq "normalize version tag adds v" "v2.1.0" "$(vpn233_normalize_version_tag "2.1.0")"
assert_eq "normalize version tag keeps v" "v2.1.0" "$(vpn233_normalize_version_tag "v2.1.0")"
assert_eq "normalize version strips v" "2.1.0" "$(vpn233_normalize_version "v2.1.0")"
assert_eq "asset name linux" "vpn233-rust-linux-x86_64.tar.gz" \
  "$(vpn233_release_asset_name "linux-x86_64")"
assert_eq "asset name windows is zip" "vpn233-rust-windows-x86_64.zip" \
  "$(vpn233_release_asset_name "windows-x86_64")"
assert_eq "asset name macos arm64" "vpn233-rust-macos-aarch64.tar.gz" \
  "$(vpn233_release_asset_name "macos-aarch64")"

# Prefix normalisation.
assert_eq "normalize prefix adds slash" "https://mirror.example/" \
  "$(vpn233_mirror_normalize_prefix "https://mirror.example")"
assert_eq "normalize prefix keeps single slash" "https://mirror.example/" \
  "$(vpn233_mirror_normalize_prefix "https://mirror.example///")"
assert_eq "normalize empty prefix is direct" "" "$(vpn233_mirror_normalize_prefix "")"
assert_fail "normalize rejects prefix without scheme" \
  vpn233_mirror_normalize_prefix "mirror.example/"

# ---------------------------------------------------------------------------
# 3. Environment variable overrides
# ---------------------------------------------------------------------------
reset_env
export VPN233_GH_MIRROR="https://mirror.example/"
assert_eq "VPN233_GH_MIRROR single prefix" "[https://mirror.example/]" "$(join_prefixes release)"
assert_eq "VPN233_GH_MIRROR mode" "prefix" "$(vpn233_mirror_mode)"

reset_env
export VPN233_GH_MIRRORS="https://one.example/|https://two.example/"
assert_eq "VPN233_GH_MIRRORS list order" \
  "[https://one.example/]|[https://two.example/]" "$(join_prefixes release)"
assert_eq "VPN233_GH_MIRRORS mode" "list" "$(vpn233_mirror_mode)"

reset_env
export VPN233_GH_MIRRORS="https://one.example/||https://two.example/"
assert_eq "VPN233_GH_MIRRORS list keeps explicit direct" \
  "[https://one.example/]|[]|[https://two.example/]" "$(join_prefixes release)"

reset_env
export VPN233_NO_MIRROR=1
assert_eq "VPN233_NO_MIRROR=1 means direct only" "[]" "$(join_prefixes release)"
assert_eq "VPN233_NO_MIRROR=1 mode" "off" "$(vpn233_mirror_mode)"

reset_env
export VPN233_NO_MIRROR=1
export VPN233_GH_MIRROR="https://mirror.example/"
assert_eq "VPN233_NO_MIRROR wins over single prefix" "[]" "$(join_prefixes release)"

reset_env
assert_ok "mirror_set_mode auto" vpn233_mirror_set_mode "auto"
assert_eq "mirror_set_mode auto resets mode" "auto" "$(vpn233_mirror_mode)"
assert_ok "mirror_set_mode off" vpn233_mirror_set_mode "off"
assert_eq "mirror_set_mode off list" "[]" "$(join_prefixes release)"
assert_ok "mirror_set_mode custom" vpn233_mirror_set_mode "https://custom.example"
assert_eq "mirror_set_mode custom list" "[https://custom.example/]" "$(join_prefixes release)"
assert_eq "mirror_set_mode custom mode" "prefix" "$(vpn233_mirror_mode)"
assert_ok "mirror_set_mode auto again" vpn233_mirror_set_mode "auto"
assert_eq "mirror_set_mode auto default list restored" \
  "[https://ghproxy.net/]|[https://gh.llkk.cc/]|[https://ghfast.top/]|[https://gh-proxy.com/]|[]" \
  "$(join_prefixes release)"
assert_fail "mirror_set_mode rejects invalid prefix" vpn233_mirror_set_mode "ghproxy.net"
assert_fail "prefix rejects credential URL" vpn233_mirror_normalize_prefix 'https://user:private@example.invalid/'
assert_fail "prefix rejects query URL" vpn233_mirror_normalize_prefix 'https://example.invalid/?private=synthetic'
assert_fail "prefix rejects whitespace" vpn233_mirror_normalize_prefix 'https://example.invalid/ has-space'
assert_fail "list propagates invalid prefix error" vpn233_mirror_split_list 'https://valid.invalid/|bad-prefix'
assert_fail "version rejects path traversal" vpn233_normalize_version '../main'

# ---------------------------------------------------------------------------
# 4. Auto fallback with an injected fake downloader
# ---------------------------------------------------------------------------
FAKE_LOG="$WORK_DIR/fake-fetch.log"
FAKE_FAIL_PATTERN=""
FAKE_EMPTY=0
FAKE_BODY="fake-bundle-body"
: > "$FAKE_LOG"

# shellcheck disable=SC2317
_vpn233_fetch_url() {
  local url="$1" dest="$2"
  printf 'CALL %s\n' "$url" >> "$FAKE_LOG"
  if [[ -n "$FAKE_FAIL_PATTERN" ]]; then
    case "$url" in
      *"$FAKE_FAIL_PATTERN"*) return 1 ;;
    esac
  fi
  if [[ "$FAKE_EMPTY" -eq 1 ]]; then
    : > "$dest"
    return 0
  fi
  printf '%s\n' "$FAKE_BODY" > "$dest"
  return 0
}

reset_env
export VPN233_HOME="$WORK_DIR/home-fallback"
export VPN233_GH_MIRRORS="https://f1.example/|https://f2.example/|"
FAKE_FAIL_PATTERN="f1.example"
: > "$FAKE_LOG"
DEST="$WORK_DIR/fetched.bundle"
if vpn233_download_release_asset "vpn233-rust-linux-x86_64.tar.gz" "$DEST" "" 1 2>/dev/null; then
  pass "auto fallback download succeeds on second mirror"
else
  fail "auto fallback download succeeds on second mirror" "success" "non-zero exit"
fi
assert_eq "auto fallback wrote file body" "fake-bundle-body" "$(tr -d '\n' < "$DEST")"
assert_eq "auto fallback attempt order" \
  "CALL https://f1.example/https://github.com/neko233-com/vpn233-all/releases/latest/download/vpn233-rust-linux-x86_64.tar.gz
CALL https://f2.example/https://github.com/neko233-com/vpn233-all/releases/latest/download/vpn233-rust-linux-x86_64.tar.gz" \
  "$(cat "$FAKE_LOG")"
assert_eq "auto fallback reports selected mirror" "https://f2.example/" "$(vpn233_last_mirror)"
assert_eq "auto fallback stores mirror cache" "release https://f2.example/" \
  "$(grep '^release ' "$VPN233_HOME/mirror-cache")"

# The cached prefix must be tried first on the next download.
FAKE_FAIL_PATTERN=""
: > "$FAKE_LOG"
if vpn233_download_release_asset "version.txt" "$WORK_DIR/version.txt" "" 1 2>/dev/null; then
  pass "cached mirror download succeeds"
else
  fail "cached mirror download succeeds" "success" "non-zero exit"
fi
assert_eq "cached mirror tried first" \
  "CALL https://f2.example/https://github.com/neko233-com/vpn233-all/releases/latest/download/version.txt" \
  "$(head -n 1 "$FAKE_LOG")"

# Total failure keeps the previous destination intact and reports every try.
reset_env
export VPN233_HOME="$WORK_DIR/home-failure"
export VPN233_GH_MIRRORS="https://f1.example/|https://f2.example/|"
FAKE_FAIL_PATTERN=""
: > "$FAKE_LOG"
KEEP="$WORK_DIR/keep.bundle"
printf 'OLD-CONTENT\n' > "$KEEP"
FAKE_FAIL_PATTERN="https://"
if vpn233_download_release_asset "SHA256SUMS" "$KEEP" "" 1 2>/dev/null; then
  fail "total mirror failure returns non-zero" "non-zero exit" "success"
else
  pass "total mirror failure returns non-zero"
fi
assert_eq "failed download keeps old file" "OLD-CONTENT" "$(tr -d '\n' < "$KEEP")"
assert_eq "failed download tried every source" "3" "$(wc -l < "$FAKE_LOG" | tr -d ' ')"
if grep -q 'https://f1.example/https://github.com/' "$FAKE_LOG" && \
   grep -q 'https://f2.example/https://github.com/' "$FAKE_LOG" && \
   grep -q '^CALL https://github.com/' "$FAKE_LOG"; then
  pass "failed download includes direct source"
else
  fail "failed download includes direct source" "direct attempt present" "missing"
fi
assert_eq "no cache entry leaked on total failure" "" \
  "$(grep '^release ' "$VPN233_HOME/mirror-cache" 2>/dev/null || true)"

# Empty HTTP body is treated as a failed attempt.
reset_env
export VPN233_HOME="$WORK_DIR/home-empty"
export VPN233_NO_MIRROR=1
FAKE_FAIL_PATTERN=""
FAKE_EMPTY=1
: > "$FAKE_LOG"
if vpn233_download_release_asset "SHA256SUMS" "$WORK_DIR/empty.bundle" "" 1 2>/dev/null; then
  fail "empty body is rejected" "non-zero exit" "success"
else
  pass "empty body is rejected"
fi
if [[ -e "$WORK_DIR/empty.bundle" ]]; then
  fail "empty body leaves no destination file" "absent" "present"
else
  pass "empty body leaves no destination file"
fi

# Direct GitHub can be cached and then preferred.
reset_env
export VPN233_HOME="$WORK_DIR/home-direct"
export VPN233_GH_MIRRORS="https://f1.example/||"
FAKE_FAIL_PATTERN="https://f1.example/"
FAKE_EMPTY=0
: > "$FAKE_LOG"
if vpn233_download_raw_file "install.sh" "$WORK_DIR/install.sh.dl" 1 2>/dev/null; then
  pass "direct raw download succeeds"
else
  fail "direct raw download succeeds" "success" "non-zero exit"
fi
assert_eq "direct mirror cache entry stored" "raw " \
  "$(sed -n '1p' "$VPN233_HOME/mirror-cache" | sed 's/^raw.*$/raw /')"
FAKE_FAIL_PATTERN=""
: > "$FAKE_LOG"
if vpn233_download_raw_file "install.sh" "$WORK_DIR/install2.sh.dl" 1 2>/dev/null; then
  pass "direct preference download succeeds"
else
  fail "direct preference download succeeds" "success" "non-zero exit"
fi
assert_eq "cached direct is tried first" \
  "CALL https://raw.githubusercontent.com/neko233-com/vpn233-all/main/install.sh" \
  "$(head -n 1 "$FAKE_LOG")"

# ---------------------------------------------------------------------------
# 5. SHA-256 helpers and SHA256SUMS parsing
# ---------------------------------------------------------------------------
reset_env
SUM_FILE="$WORK_DIR/artifact.bin"
printf 'vpn233 test artifact\n' > "$SUM_FILE"
EXPECTED="$(vpn233_sha256_file "$SUM_FILE")"
assert_eq "sha256_file returns lowercase hex" "$EXPECTED" "$(vpn233_sha256_file "$SUM_FILE")"
assert_ok "sha256_matches accepts digest" vpn233_sha256_matches "$SUM_FILE" "$EXPECTED"
assert_ok "sha256_matches accepts uppercase digest" \
  vpn233_sha256_matches "$SUM_FILE" "$(printf '%s' "$EXPECTED" | tr 'a-f' 'A-F')"
assert_fail "sha256_matches rejects wrong digest" \
  vpn233_sha256_matches "$SUM_FILE" "0000000000000000000000000000000000000000000000000000000000000000"
assert_fail "sha256_file rejects missing file" vpn233_sha256_file "$WORK_DIR/missing.bin"

SUMS="$WORK_DIR/SHA256SUMS"
{
  printf '# SHA256SUMS for vpn233 test bundles\n'
  printf '\n'
  printf '%s  vpn233-rust-linux-x86_64.tar.gz\n' "$EXPECTED"
  printf '%s *vpn233-rust-windows-x86_64.zip\r\n' "$EXPECTED"
  printf '%s  some-other-file.txt\n' "1111111111111111111111111111111111111111111111111111111111111111"
} > "$SUMS"
assert_eq "sums lookup plain file name" "$EXPECTED" \
  "$(vpn233_sha256_from_sums "$SUMS" "vpn233-rust-linux-x86_64.tar.gz")"
assert_eq "sums lookup star prefixed file name" "$EXPECTED" \
  "$(vpn233_sha256_from_sums "$SUMS" "vpn233-rust-windows-x86_64.zip")"
assert_fail "sums lookup rejects unknown file" \
  vpn233_sha256_from_sums "$SUMS" "vpn233-rust-macos-aarch64.tar.gz"
assert_fail "sums lookup rejects missing file" \
  vpn233_sha256_from_sums "$WORK_DIR/no-such-sums" "anything"
printf '%s  duplicate.bin\n%s  duplicate.bin\n' "$EXPECTED" "$EXPECTED" > "$SUMS"
assert_fail "sums lookup rejects duplicate filename" vpn233_sha256_from_sums "$SUMS" duplicate.bin
printf 'not-a-digest  invalid.bin\n' > "$SUMS"
assert_fail "sums lookup rejects malformed digest" vpn233_sha256_from_sums "$SUMS" invalid.bin
assert_fail "sha256 rejects embedded whitespace" vpn233_sha256_matches "$SUM_FILE" "$EXPECTED "

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
printf '\n'
printf 'test-mirrors: %s passed, %s failed\n' "$PASS_COUNT" "$FAIL_COUNT"
if [[ "$FAIL_COUNT" -ne 0 ]]; then
  printf 'failed tests:\n%s' "$FAIL_NAMES" >&2
  exit 1
fi
exit 0
