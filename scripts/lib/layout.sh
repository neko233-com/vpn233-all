#!/usr/bin/env bash
# Private runtime layout, parsed as data; never source/eval this metadata file.
vpn233_layout_clear() {
  VPN233_SAVED_SCOPE=""; VPN233_SAVED_BIN=""; VPN233_SAVED_WEB=""; VPN233_SAVED_CONFIG=""; VPN233_SAVED_DATA=""
}
vpn233_layout_path() {
  local path="$1"
  case "$path" in *[[:cntrl:]]*) printf 'VPN233: layout paths cannot contain control characters.\n' >&2; return 1 ;; esac
  case "$path" in /*) printf '%s\n' "$path" ;; *) printf '%s/%s\n' "$PWD" "$path" ;; esac
}
vpn233_layout_load() {
  local base="$1" file line key value count=0 expected mode
  file="$base/.vpn233-layout"
  vpn233_layout_clear
  [[ -e "$file" ]] || return 1
  [[ -f "$file" && ! -L "$file" ]] || { printf 'VPN233: invalid runtime layout file.\n' >&2; return 2; }
  mode="$(stat -c %a "$file" 2>/dev/null || stat -f %Lp "$file" 2>/dev/null)" || return 2
  [[ "$mode" =~ ^[0-7]{3,4}$ ]] && (( (8#$mode & 077) == 0 )) || { printf 'VPN233: runtime layout must be owner-only (chmod 600).\n' >&2; return 2; }
  [[ "$(wc -c < "$file")" -le 65536 ]] || return 2
  while IFS= read -r line || [[ -n "$line" ]]; do
    count=$((count + 1))
    if [[ "$count" -eq 1 ]]; then [[ "$line" == vpn233-layout-v1 ]] || return 2; continue; fi
    case "$line" in *$'\r'*|*$'\t'*$'\t'*) return 2 ;; esac
    [[ "$line" == *$'\t'* ]] || return 2
    key="${line%%$'\t'*}"; value="${line#*$'\t'}"
    case "$count" in 2) expected=scope ;; 3) expected=bin ;; 4) expected=web ;; 5) expected=config ;; 6) expected=data ;; *) return 2 ;; esac
    [[ "$key" == "$expected" && -n "$value" && "$value" != *[[:cntrl:]]* ]] || return 2
    if [[ "$key" == scope ]]; then [[ "$value" == user || "$value" == system || "$value" == source ]] || return 2
    else [[ "$value" == /* ]] || return 2; fi
    # shellcheck disable=SC2034 # Public parser outputs consumed by installer/runtime callers.
    case "$key" in scope) VPN233_SAVED_SCOPE="$value" ;; bin) VPN233_SAVED_BIN="$value" ;; web) VPN233_SAVED_WEB="$value" ;; config) VPN233_SAVED_CONFIG="$value" ;; data) VPN233_SAVED_DATA="$value" ;; esac
  done < "$file"
  [[ "$count" -eq 6 ]] || return 2
}
vpn233_layout_save() {
  local base="$1" scope="$2" bin="$3" web="$4" config="$5" data="$6" file temporary
  file="$base/.vpn233-layout"; temporary="$base/.vpn233-layout.tmp.$$"
  [[ "$scope" == user || "$scope" == system || "$scope" == source ]] || return 1
  [[ ! -L "$file" ]] || return 1
  bin="$(vpn233_layout_path "$bin")" || return 1
  web="$(vpn233_layout_path "$web")" || return 1
  config="$(vpn233_layout_path "$config")" || return 1
  data="$(vpn233_layout_path "$data")" || return 1
  (umask 077; printf 'vpn233-layout-v1\nscope\t%s\nbin\t%s\nweb\t%s\nconfig\t%s\ndata\t%s\n' "$scope" "$bin" "$web" "$config" "$data" > "$temporary") || return 1
  chmod 0600 "$temporary" || return 1
  mv -f "$temporary" "$file"
}
# Older system installs have no metadata. Only exact service ownership is proof.
vpn233_layout_legacy() {
  local base="$1" bin="$2" unit_dir="${3:-/etc/systemd/system}" plist_dir="${4:-/Library/LaunchDaemons}" name binary config file line found=0
  vpn233_layout_clear
  for name in provider subscribe client; do
    binary="vpn233-${name}-server"; config="${name}-server.yaml"
    [[ "$name" != client ]] || { binary=vpn233-client; config=vpn233-client.yaml; }
    file="$unit_dir/vpn233-${name}.service"
    if [[ -f "$file" && ! -L "$file" ]]; then
      line="ExecStart=$bin/$binary --config /etc/vpn233/$config"
      if grep -Fxq -- "$line" "$file" || grep -Fxq -- "ExecStart=\"$bin/$binary\" --config \"/etc/vpn233/$config\"" "$file"; then found=$((found + 1)); fi
    fi
    file="$plist_dir/com.neko233.vpn233.${name}.plist"
    if [[ -f "$file" && ! -L "$file" ]] && grep -Fxq "    <string>$bin/$binary</string>" "$file" && grep -Fxq "    <string>/etc/vpn233/$config</string>" "$file"; then found=$((found + 1)); fi
  done
  if [[ "$found" -gt 0 ]]; then
    # shellcheck disable=SC2034 # Legacy discovery outputs consumed by installer/runtime callers.
    VPN233_SAVED_SCOPE=system
    # shellcheck disable=SC2034 # Legacy discovery output consumed by installer/runtime callers.
    VPN233_SAVED_BIN="$bin"
    # shellcheck disable=SC2034 # Legacy discovery output consumed by installer/runtime callers.
    VPN233_SAVED_CONFIG=/etc/vpn233
    # shellcheck disable=SC2034 # Legacy discovery output consumed by installer/runtime callers.
    VPN233_SAVED_DATA=/var/lib/vpn233
    return 0
  fi
  # shellcheck disable=SC2034 # Legacy discovery outputs consumed by installer/runtime callers.
  if [[ -d "$base/config" ]]; then VPN233_SAVED_SCOPE=user; VPN233_SAVED_BIN="$bin"; return 0; fi
  if [[ -f "$base/version.txt" && -d "$bin" ]]; then
    printf 'VPN233: existing install layout is ambiguous; specify --system or VPN233_CONFIG_DIR/VPN233_DATA_DIR.\n' >&2
    return 2
  fi
  return 1
}
