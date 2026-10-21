#!/usr/bin/env bash
# shellcheck shell=bash
#
# VPN233 GitHub mirror acceleration helpers.
#
# This file is sourced by install.sh, update.sh and the offline script tests.
# It intentionally does not change shell options and is bash 3.2 compatible
# (macOS ships bash 3.2 by default).
#
# Public contract:
#   * release asset prefixes, in order:
#       https://ghproxy.net/
#       https://gh.llkk.cc/
#       https://ghfast.top/
#       https://gh-proxy.com/
#       ""                      (direct GitHub, last)
#   * raw file prefixes: the same list.
#   * jsDelivr form: https://cdn.jsdelivr.net/gh/{owner}/{repo}@{ref}/{path}
#   * environment:
#       VPN233_GH_MIRROR        single prefix override
#       VPN233_GH_MIRRORS       "|" separated prefix list override
#       VPN233_NO_MIRROR=1      disable mirrors (direct only)
#       VPN233_MIRROR_MODE      set by installers for --mirror auto|off|PREFIX
#       VPN233_HOME             directory that stores mirror-cache
#   * every download goes to a temporary file and is moved into place with
#     mv, so a failed download never truncates a previously good file.

VPN233_REPO_OWNER="${VPN233_REPO_OWNER:-neko233-com}"
VPN233_REPO_NAME="${VPN233_REPO_NAME:-vpn233-all}"
VPN233_REPO_REF="${VPN233_REPO_REF:-main}"
VPN233_GH_CONNECT_TIMEOUT="${VPN233_GH_CONNECT_TIMEOUT:-5}"
VPN233_GH_MAX_TIME="${VPN233_GH_MAX_TIME:-300}"
VPN233_LAST_MIRROR=""

# Return the mirror prefix that produced the last successful download.
# An empty result means direct GitHub was used.
vpn233_last_mirror() {
  printf '%s\n' "${VPN233_LAST_MIRROR}"
}

# Machine-checkable mirror contract anchors. The CI docs consistency check and
# the deployment documentation grep these exact strings, so keep them in sync
# with scripts/lib/mirrors.ps1 and docs/mirrors.md.
VPN233_GH_MIRROR_PREFIXES_DEFAULT="https://ghproxy.net/ https://gh.llkk.cc/ https://ghfast.top/ https://gh-proxy.com/ "
VPN233_GH_RAW_PREFIXES_DEFAULT="https://ghproxy.net/ https://gh.llkk.cc/ https://ghfast.top/ https://gh-proxy.com/ "

# Expand the space separated contract anchor into one prefix per line and
# append direct GitHub ("") as the final fallback.
vpn233_mirror_print_contract_prefixes() {
  local list="$1" item
  while IFS= read -r item; do
    if [[ -n "$item" ]]; then
      printf '%s\n' "$item"
    fi
  done <<< "$(printf '%s\n' "$list" | tr ' ' '\n')"
  printf '%s\n' ''
}

# Print the default release asset mirror prefixes (direct GitHub is last).
vpn233_mirror_default_release_prefixes() {
  vpn233_mirror_print_contract_prefixes "$VPN233_GH_MIRROR_PREFIXES_DEFAULT"
}

# Print the default raw file mirror prefixes (direct GitHub is last).
vpn233_mirror_default_raw_prefixes() {
  vpn233_mirror_print_contract_prefixes "$VPN233_GH_RAW_PREFIXES_DEFAULT"
}

# Normalise a mirror prefix so it always ends with a single slash.
# An empty prefix means "direct GitHub" and stays empty.
vpn233_mirror_normalize_prefix() {
  local prefix="${1:-}" host
  if [[ -z "$prefix" ]]; then
    printf '%s\n' ''
    return 0
  fi
  case "$prefix" in
    http://?*|https://?*) ;;
    *)
      printf 'VPN233: mirror prefix must start with http:// or https://.\n' >&2
      return 1
      ;;
  esac
  host="${prefix#*://}"
  host="${host%%/*}"
  if [[ -z "$host" || "$host" == :* ]]; then
    printf 'VPN233: mirror prefix requires a host.\n' >&2
    return 1
  fi
  case "$prefix" in
    *'@'*|*'?'*|*'#'*|*[[:space:]]*)
      printf 'VPN233: mirror prefix cannot contain credentials, query, fragment or whitespace.\n' >&2
      return 1 ;;
  esac
  while [[ "$prefix" == */ ]]; do
    prefix="${prefix%/}"
  done
  if [[ -z "$prefix" ]]; then
    printf 'VPN233: invalid mirror prefix.\n' >&2
    return 1
  fi
  printf '%s/\n' "$prefix"
}

# Print the active mirror mode: auto, list, prefix or off.
vpn233_mirror_mode() {
  case "${VPN233_NO_MIRROR:-0}" in
    1|true|TRUE|yes|YES)
      printf 'off\n'
      return 0
      ;;
  esac
  if [[ -n "${VPN233_MIRROR_MODE:-}" ]]; then
    case "${VPN233_MIRROR_MODE}" in
      auto) printf 'auto\n' ;;
      off|none|direct) printf 'off\n' ;;
      *) printf 'prefix\n' ;;
    esac
    return 0
  fi
  if [[ -n "${VPN233_GH_MIRRORS:-}" ]]; then
    printf 'list\n'
    return 0
  fi
  if [[ -n "${VPN233_GH_MIRROR:-}" ]]; then
    printf 'prefix\n'
    return 0
  fi
  printf 'auto\n'
}

# Set the mirror mode from a --mirror CLI value. Empty or "auto" keeps auto.
vpn233_mirror_set_mode() {
  local value="${1:-}"
  case "$value" in
    ''|auto|AUTO)
      unset VPN233_MIRROR_MODE
      return 0
      ;;
    off|OFF|none|NONE|direct|DIRECT)
      VPN233_MIRROR_MODE="off"
      return 0
      ;;
  esac
  if ! VPN233_MIRROR_MODE="$(vpn233_mirror_normalize_prefix "$value")"; then
    printf 'VPN233: invalid --mirror value (use auto, off or a URL prefix).\n' >&2
    return 1
  fi
  return 0
}

# Split a "|" separated mirror list and normalise every entry.
# A trailing "|" produces one final empty entry, which means direct GitHub.
vpn233_mirror_split_list() {
  local rest="${1:-}" item
  while :; do
    case "$rest" in
      *'|'*)
        item="${rest%%|*}"
        rest="${rest#*|}"
        item="$(vpn233_mirror_normalize_prefix "$item")" || return 1
        printf '%s\n' "$item"
        ;;
      *)
        item="$(vpn233_mirror_normalize_prefix "$rest")" || return 1
        printf '%s\n' "$item"
        break
        ;;
    esac
  done
  return 0
}

# Print the effective prefix list for a kind ("release" or "raw").
vpn233_mirror_list() {
  local kind="${1:-release}" mode
  mode="$(vpn233_mirror_mode)"
  case "$mode" in
    off)
      printf '%s\n' ''
      ;;
    list)
      vpn233_mirror_split_list "${VPN233_GH_MIRRORS:-}"
      ;;
    prefix)
      vpn233_mirror_normalize_prefix "${VPN233_MIRROR_MODE:-${VPN233_GH_MIRROR:-}}"
      ;;
    *)
      case "$kind" in
        raw) vpn233_mirror_default_raw_prefixes ;;
        *) vpn233_mirror_default_release_prefixes ;;
      esac
      ;;
  esac
}

# Directory that stores the non-sensitive mirror-cache file.
vpn233_mirror_cache_dir() {
  if [[ -n "${VPN233_HOME:-}" ]]; then
    printf '%s\n' "${VPN233_HOME}"
    return 0
  fi
  if [[ -n "${HOME:-}" ]]; then
    printf '%s\n' "${HOME}/.vpn233"
    return 0
  fi
  printf '%s\n' "${TMPDIR:-/tmp}/vpn233"
}

# Remember which prefix worked for a kind. Only mirror prefixes are stored,
# never URLs with tokens or any other private data.
vpn233_mirror_cache_store() {
  local kind="$1" prefix="${2:-}" dir file tmp line
  dir="$(vpn233_mirror_cache_dir)"
  if ! mkdir -p "$dir" 2>/dev/null; then
    return 0
  fi
  file="$dir/mirror-cache"
  tmp="$dir/.mirror-cache.tmp.$$"
  if ! : > "$tmp" 2>/dev/null; then
    return 0
  fi
  if [[ -f "$file" ]]; then
    while IFS= read -r line; do
      case "$line" in
        "$kind "*) ;;
        '') ;;
        *) printf '%s\n' "$line" >> "$tmp" 2>/dev/null || true ;;
      esac
    done < "$file"
  fi
  printf '%s %s\n' "$kind" "$prefix" >> "$tmp" 2>/dev/null || true
  mv -f "$tmp" "$file" 2>/dev/null || rm -f "$tmp" 2>/dev/null || true
  return 0
}

# Read the cached prefix for a kind. A successful return with empty output
# means direct GitHub was the last known good source.
vpn233_mirror_cache_get() {
  local kind="$1" file line
  file="$(vpn233_mirror_cache_dir)/mirror-cache"
  if [[ ! -f "$file" ]]; then
    return 1
  fi
  while IFS= read -r line; do
    case "$line" in
      "$kind "*) printf '%s\n' "${line#"$kind "}"; return 0 ;;
    esac
  done < "$file"
  return 1
}

# Print prefixes with the cached one first, then the rest in contract order.
vpn233_mirror_ordered_prefixes() {
  local kind="${1:-release}" cached list_file prefix found=0
  if ! cached="$(vpn233_mirror_cache_get "$kind")"; then
    vpn233_mirror_list "$kind"
    return $?
  fi
  list_file="$(mktemp "${TMPDIR:-/tmp}/vpn233-mirror-list.XXXXXX" 2>/dev/null)" || {
    vpn233_mirror_list "$kind"
    return 0
  }
  if ! vpn233_mirror_list "$kind" > "$list_file"; then
    rm -f "$list_file"
    return 1
  fi
  if [[ -z "$cached" ]]; then
    if awk 'length($0) == 0 { found = 1 } END { exit(found ? 0 : 1) }' "$list_file"; then
      found=1
    fi
  else
    if grep -F -x -q -- "$cached" "$list_file"; then
      found=1
    fi
  fi
  if [[ "$found" -eq 0 ]]; then
    rm -f "$list_file"
    vpn233_mirror_list "$kind"
    return 0
  fi
  if [[ -z "$cached" ]]; then
    printf '%s\n' ''
    while IFS= read -r prefix; do
      if [[ -n "$prefix" ]]; then
        printf '%s\n' "$prefix"
      fi
    done < "$list_file"
  else
    printf '%s\n' "$cached"
    while IFS= read -r prefix; do
      if [[ "$prefix" != "$cached" ]]; then
        printf '%s\n' "$prefix"
      fi
    done < "$list_file"
  fi
  rm -f "$list_file"
  return 0
}

# Convert "1.2.3" / "v1.2.3" into "v1.2.3".
vpn233_normalize_version_tag() {
  local version="${1:-}"
  vpn233_normalize_version "$version" >/dev/null || return 1
  case "$version" in
    v*) ;;
    V*) version="v${version#V}" ;;
    *) version="v${version}" ;;
  esac
  printf '%s\n' "$version"
}

# Convert "1.2.3" / "v1.2.3" into "1.2.3".
vpn233_normalize_version() {
  local version="${1:-}"
  version="${version#v}"
  version="${version#V}"
  if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]; then
    printf 'VPN233: version must be a semantic version such as 2.1.0.\n' >&2
    return 1
  fi
  printf '%s\n' "$version"
}

# Build the canonical GitHub release URL for an asset.
vpn233_github_release_url() {
  local asset="$1" version="${2:-}" tag
  if [[ -n "$version" ]]; then
    tag="$(vpn233_normalize_version_tag "$version")" || return 1
    printf 'https://github.com/%s/%s/releases/download/%s/%s\n' \
      "$VPN233_REPO_OWNER" "$VPN233_REPO_NAME" "$tag" "$asset"
  else
    printf 'https://github.com/%s/%s/releases/latest/download/%s\n' \
      "$VPN233_REPO_OWNER" "$VPN233_REPO_NAME" "$asset"
  fi
}

# Build the canonical raw.githubusercontent.com URL for a repository path.
vpn233_github_raw_url() {
  local path="$1" ref="${2:-$VPN233_REPO_REF}"
  path="${path#/}"
  printf 'https://raw.githubusercontent.com/%s/%s/%s/%s\n' \
    "$VPN233_REPO_OWNER" "$VPN233_REPO_NAME" "$ref" "$path"
}

# Build a jsDelivr URL for a repository path (optional raw fallback).
vpn233_jsdelivr_raw_url() {
  local path="$1" ref="${2:-$VPN233_REPO_REF}"
  path="${path#/}"
  printf 'https://cdn.jsdelivr.net/gh/%s/%s@%s/%s\n' \
    "$VPN233_REPO_OWNER" "$VPN233_REPO_NAME" "$ref" "$path"
}

# Apply a mirror prefix to a canonical URL. Empty prefix = direct.
vpn233_mirror_apply() {
  local prefix="${1:-}" url="$2"
  if [[ -z "$prefix" ]]; then
    printf '%s\n' "$url"
  else
    printf '%s%s\n' "$prefix" "$url"
  fi
}

# Deliver one URL to a destination file atomically.
vpn233_download_atomic() {
  local url="$1" dest="$2" tmp
  tmp="$(mktemp "${dest}.part.XXXXXX" 2>/dev/null)" || return 1
  if _vpn233_fetch_url "$url" "$tmp"; then
    if [[ -s "$tmp" ]]; then
      if mv -f "$tmp" "$dest"; then
        return 0
      fi
    fi
  fi
  rm -f "$tmp" 2>/dev/null || true
  return 1
}

# Downloader primitive. Tests override this function to run offline.
_vpn233_fetch_url() {
  local url="$1" dest="$2" max_time="${3:-$VPN233_GH_MAX_TIME}" connect_timeout="${4:-$VPN233_GH_CONNECT_TIMEOUT}" timing=""
  [[ "${5:-}" != timed ]] || timing='%{time_total}'
  if command -v curl >/dev/null 2>&1; then
    if curl -q --fail --location --silent --show-error --proto '=https,http' --proto-redir '=https,http' \
      --connect-timeout "$connect_timeout" \
      --max-time "$max_time" --write-out "$timing" --header 'Accept-Encoding: identity' \
      --output "$dest" "$url"; then
      return 0
    fi
    return 1
  fi
  if command -v wget >/dev/null 2>&1; then
    if wget --quiet --tries=1 --timeout="$max_time" \
      --output-document="$dest" "$url"; then
      return 0
    fi
    return 1
  fi
  printf 'VPN233: curl or wget is required to download files.\n' >&2
  return 1
}

# Resume only the current source's private partial file. curl rejects an HTTP
# server that ignores Range or returns a mismatched resume position. A final
# digest check below is still required; a successful HTTP response is not trust.
_vpn233_resume_url() {
  local url="$1" dest="$2" max_time="$3" connect_timeout="$4"
  command -v curl >/dev/null 2>&1 || return 1
  curl -q --fail --location --silent --proto '=https,http' --proto-redir '=https,http' \
    --connect-timeout "$connect_timeout" --max-time "$max_time" \
    --header 'Accept-Encoding: identity' --write-out '%{time_total}' \
    --continue-at - --output "$dest" "$url"
}

# Atomic archive download with an expected digest. A source gets at most three
# requests: the initial transfer and two same-source resumptions after transport
# failures with nonempty partial data. Complete but wrong content immediately
# falls back to the next source. No partial file is shared between sources.
vpn233_download_verified_atomic() {
  local url="$1" dest="$2" expected="$3" tmp attempt=1 transferred=0 transfer_status=0
  local remaining transfer_time connect_timeout
  [[ "$expected" =~ ^[0-9A-Fa-f]{64}$ ]] || return 1
  remaining="$(LC_ALL=C awk -v budget="$VPN233_GH_MAX_TIME" 'BEGIN {
    if (budget !~ /^[0-9]+([.][0-9]+)?$/ || budget < 0.001) exit 1
    printf "%.6f", budget
  }')" || return 1
  connect_timeout="$(LC_ALL=C awk -v connect="$VPN233_GH_CONNECT_TIMEOUT" -v budget="$remaining" 'BEGIN {
    if (connect !~ /^[0-9]+([.][0-9]+)?$/ || connect < 0.001) exit 1
    printf "%.6f", (connect < budget ? connect : budget)
  }')" || return 1
  tmp="$(mktemp "${dest}.part.XXXXXX" 2>/dev/null)" || return 1
  while [[ "$attempt" -le 3 ]]; do
    transferred=0
    transfer_status=0
    if [[ "$attempt" -eq 1 ]]; then
      if transfer_time="$(_vpn233_fetch_url "$url" "$tmp" "$remaining" "$connect_timeout" timed 2>/dev/null)"; then transferred=1; else transfer_status=$?; fi
    else
      if transfer_time="$(_vpn233_resume_url "$url" "$tmp" "$remaining" "$connect_timeout" 2>/dev/null)"; then transferred=1; else transfer_status=$?; fi
    fi
    if [[ -s "$tmp" ]] && vpn233_sha256_matches "$tmp" "$expected"; then
      if mv -f "$tmp" "$dest"; then return 0; fi
      break
    fi
    # A complete response with the wrong digest must not be continued, and a
    # transfer with no data cannot be safely resumed. Neither is cached.
    if [[ "$transferred" -eq 1 || ! -s "$tmp" ]]; then break; fi
    # HTTP errors and unsupported/invalid resume responses cannot be repaired
    # by repeating the same Range request. Network interruptions remain bounded.
    case "$transfer_status" in 22|33|36|63) break ;; esac
    # curl's transfer timer includes connection and body time. All requests to
    # one source share the configured network budget, including partial retries.
    remaining="$(LC_ALL=C awk -v budget="$remaining" -v used="$transfer_time" 'BEGIN {
      # curl rounds timeouts to milliseconds; a smaller remainder becomes zero
      # (unlimited), so stop instead of accidentally removing the time bound.
      if (used !~ /^[0-9]+([.][0-9]+)?$/ || used < 0 || budget - used < 0.001) exit 1
      printf "%.6f", budget - used
    }')" || break
    connect_timeout="$(LC_ALL=C awk -v connect="$VPN233_GH_CONNECT_TIMEOUT" -v budget="$remaining" 'BEGIN {
      printf "%.6f", (connect < budget ? connect : budget)
    }')" || break
    attempt=$((attempt + 1))
  done
  rm -f "$tmp" 2>/dev/null || true
  return 1
}

# Download a release asset through the mirror list with auto fallback.
# Arguments: <asset> <destination> [version] [quiet] [expected SHA-256]
vpn233_download_release_asset() {
  local asset="$1" dest="$2" version="${3:-}" quiet="${4:-0}" expected="${5:-}"
  local base prefix candidate prefixes downloaded source_index=0
  if [[ -n "$expected" && ! "$expected" =~ ^[0-9A-Fa-f]{64}$ ]]; then
    printf 'VPN233: expected SHA-256 must contain exactly 64 hexadecimal characters.\n' >&2
    return 1
  fi
  base="$(vpn233_github_release_url "$asset" "$version")" || return 1
  prefixes="$(vpn233_mirror_ordered_prefixes release || exit 1; printf '\001')" || return 1
  prefixes="${prefixes%$'\001'}"
  while IFS= read -r prefix; do
    source_index=$((source_index + 1))
    candidate="$(vpn233_mirror_apply "$prefix" "$base")"
    downloaded=0
    if [[ -n "$expected" ]]; then
      if vpn233_download_verified_atomic "$candidate" "$dest" "$expected"; then downloaded=1; fi
    elif vpn233_download_atomic "$candidate" "$dest" 2>/dev/null; then
      downloaded=1
    fi
    if [[ "$downloaded" -eq 1 ]]; then
      vpn233_mirror_cache_store release "$prefix"
      VPN233_LAST_MIRROR="$prefix"
      if [[ "$quiet" != "1" ]]; then
        printf '  downloaded release asset from source %s\n' "$source_index"
      fi
      return 0
    fi
    printf '  release source %s unavailable or unverified (trying next source)\n' "$source_index" >&2
  done <<< "${prefixes%$'\n'}"
  printf 'VPN233: unable to download %s from any known GitHub source.\n' "$asset" >&2
  return 1
}

# Download a repository raw file through the mirror list with auto fallback.
# Arguments: <repository path> <destination> [quiet]
vpn233_download_raw_file() {
  local path="$1" dest="$2" quiet="${3:-0}"
  local base prefix candidate prefixes
  base="$(vpn233_github_raw_url "$path")"
  prefixes="$(vpn233_mirror_ordered_prefixes raw || exit 1; printf '\001')" || return 1
  prefixes="${prefixes%$'\001'}"
  while IFS= read -r prefix; do
    candidate="$(vpn233_mirror_apply "$prefix" "$base")"
    if vpn233_download_atomic "$candidate" "$dest"; then
      vpn233_mirror_cache_store raw "$prefix"
      VPN233_LAST_MIRROR="$prefix"
      if [[ "$quiet" != "1" ]]; then
        printf '  downloaded %s via %s\n' "$path" "${prefix:-direct GitHub}"
      fi
      return 0
    fi
    printf '  mirror unavailable: %s (trying next source)\n' "${prefix:-direct GitHub}" >&2
  done <<< "${prefixes%$'\n'}"
  printf 'VPN233: unable to download %s from any known GitHub source.\n' "$path" >&2
  return 1
}

# Compute the lowercase SHA-256 hex digest of a file.
vpn233_sha256_file() {
  local file="$1" out
  if [[ ! -f "$file" ]]; then
    printf 'VPN233: cannot hash missing file: %s\n' "$file" >&2
    return 1
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    out="$(sha256sum "$file")" || return 1
    printf '%s\n' "${out%% *}"
    return 0
  fi
  if command -v shasum >/dev/null 2>&1; then
    out="$(shasum -a 256 "$file")" || return 1
    printf '%s\n' "${out%% *}"
    return 0
  fi
  if command -v openssl >/dev/null 2>&1; then
    out="$(openssl dgst -sha256 "$file")" || return 1
    printf '%s\n' "${out##* }"
    return 0
  fi
  printf 'VPN233: no SHA-256 tool found (need sha256sum, shasum or openssl).\n' >&2
  return 1
}

# Read the SHA-256 for one file name from a SHA256SUMS file.
# Tolerates comment lines, blank lines, CRLF and an optional leading '*'.
vpn233_sha256_from_sums() {
  local sums="$1" name="$2"
  if [[ ! -f "$sums" ]]; then
    return 1
  fi
  awk -v want="$name" '
    {
      sub(/\r$/, "")
    }
    /^[[:space:]]*#/ { next }
    /^[[:space:]]*$/ { next }
    NF < 2 { next }
    {
      file = $NF
      sub(/^\*/, "", file)
      if (file == want) {
        if (length($1) != 64 || $1 ~ /[^0-9A-Fa-f]/) { invalid = 1 }
        digest = tolower($1)
        found++
      }
    }
    END {
      if (found != 1 || invalid) { exit 1 }
      print digest
    }
  ' "$sums"
}

# Compare a file against an expected SHA-256 hex digest (case insensitive).
vpn233_sha256_matches() {
  local file="$1" expected="$2" actual
  expected="$(printf '%s' "$expected" | tr 'A-F' 'a-f')"
  if [[ ! "$expected" =~ ^[0-9a-f]{64}$ ]]; then
    return 1
  fi
  actual="$(vpn233_sha256_file "$file")" || return 1
  if [[ "$actual" == "$expected" ]]; then
    return 0
  fi
  return 1
}

# Build the release asset name for a platform string such as linux-x86_64.
# Windows bundles use .zip, every other platform uses .tar.gz.
vpn233_release_asset_name() {
  local platform="$1"
  case "$platform" in
    windows-*) printf 'vpn233-rust-%s.zip\n' "$platform" ;;
    *) printf 'vpn233-rust-%s.tar.gz\n' "$platform" ;;
  esac
}
