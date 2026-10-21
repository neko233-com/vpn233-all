#!/usr/bin/env bash
# Local service ownership checks. Never manage a unit from another installation.
# Caller supplies ROOT_DIR, BIN_DIR and CONFIG_DIR.

vpn233_unit_quote() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//\%/%%}"
  value="${value//\$/\$\$}"
  printf '"%s"' "$value"
}

vpn233_xml_escape() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' -e 's/"/\&quot;/g' -e "s/'/\&apos;/g"
}

vpn233_unit_owned() {
  local file="$1" name="$2" binary config expected legacy
  binary="vpn233-${name}-server"
  config="${name}-server.yaml"
  if [[ "$name" == client ]]; then binary=vpn233-client; config=vpn233-client.yaml; fi
  [[ -f "$file" && ! -L "$file" ]] || return 1
  expected="ExecStart=$(vpn233_unit_quote "$BIN_DIR/$binary") --config $(vpn233_unit_quote "$CONFIG_DIR/$config")"
  legacy="ExecStart=$BIN_DIR/$binary --config $CONFIG_DIR/$config"
  grep -F -x -q -- "$expected" "$file" || grep -F -x -q -- "$legacy" "$file"
}

vpn233_plist_owned() {
  local file="$1" name="$2" binary
  binary="vpn233-${name}-server"
  [[ "$name" != client ]] || binary=vpn233-client
  [[ -f "$file" && ! -L "$file" ]] || return 1
  grep -F -x -q "    <string>$(vpn233_xml_escape "$BIN_DIR/$binary")</string>" "$file"
}

# Print scope + full file paths for owned units. Never follows a unit symlink.
vpn233_owned_services() {
  local name file directory scope
  case "$(uname -s)" in
    Linux*)
      command -v systemctl >/dev/null 2>&1 || return 0
      for scope in user system; do
        if [[ "$scope" == user ]]; then directory="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"; else directory=/etc/systemd/system; fi
        for name in provider subscribe client; do
          file="$directory/vpn233-${name}.service"
          if vpn233_unit_owned "$file" "$name"; then printf '%s|%s\n' "$scope" "$file"; fi
        done
      done ;;
    Darwin*)
      for scope in user system; do
        if [[ "$scope" == user ]]; then directory="$HOME/Library/LaunchAgents"; else directory=/Library/LaunchDaemons; fi
        for name in provider subscribe client; do
          file="$directory/com.neko233.vpn233.${name}.plist"
          if vpn233_plist_owned "$file" "$name"; then printf '%s|%s\n' "$scope" "$file"; fi
        done
      done ;;
  esac
}

# Return 0 when any owned service was handled, 1 when no service belongs here.
vpn233_manage_services() {
  local action="$1" scope file unit found=1
  while IFS='|' read -r scope file; do
    [[ -n "$file" ]] || continue
    found=0
    case "$(uname -s)" in
      Linux*)
        unit="${file##*/}"
        if [[ "$action" == uninstall ]]; then
          if [[ "$scope" == user ]]; then systemctl --user disable --now "$unit" || return 2
          else systemctl disable --now "$unit" || return 2; fi
          rm -f -- "$file" || return 2
        elif [[ "$scope" == user ]]; then systemctl --user "$action" "$unit" || return 2
        else systemctl "$action" "$unit" || return 2; fi ;;
      Darwin*)
        if [[ "$action" == start ]]; then launchctl load -w "$file" || return 2
        else
          launchctl unload -w "$file" 2>/dev/null || true
          if [[ "$action" == uninstall ]]; then rm -f -- "$file" || return 2; fi
        fi ;;
    esac
  done < <(vpn233_owned_services)
  if [[ "$action" == uninstall && "$found" -eq 0 && "$(uname -s)" == Linux* ]]; then
    systemctl --user daemon-reload 2>/dev/null || true
    if [[ "$(id -u)" -eq 0 ]]; then systemctl daemon-reload || return 2; fi
  fi
  return "$found"
}

# For Linux, /proc identifies the exact executable and original process start.
# macOS comm returns the executable path. Name-only matches are never accepted.
vpn233_process_matches() {
  local pid="$1" expected="$2" config="${3:-}" identity="${4:-}" actual argument previous="" found=0
  [[ "$pid" =~ ^[0-9]+$ && "$pid" -gt 1 ]] || return 1
  kill -0 "$pid" 2>/dev/null || return 1
  if [[ -e "/proc/$pid/exe" ]]; then
    actual="$(readlink "/proc/$pid/exe" 2>/dev/null || true)"
  else
    actual="$(ps -p "$pid" -o comm= 2>/dev/null || true)"
  fi
  [[ "$actual" == "$expected" ]] || return 1
  if [[ -n "$identity" ]]; then
    [[ "$(ps -p "$pid" -o lstart= 2>/dev/null || true)" == "$identity" ]] || return 1
  fi
  if [[ -n "$config" && -r "/proc/$pid/cmdline" ]]; then
    while IFS= read -r -d '' argument; do
      if [[ "$previous" == --config && "$argument" == "$config" ]]; then found=1; fi
      previous="$argument"
    done < "/proc/$pid/cmdline"
    [[ "$found" -eq 1 ]] || return 1
  fi
  return 0
}

vpn233_stop_owned_processes() {
  local base="$1" name binary pid_file process_id identity
  for name in provider-server subscribe-server vpn233-client; do
    binary="vpn233-$name"; [[ "$name" != vpn233-client ]] || binary="$name"
    pid_file="$base/runtime/$name.pid"
    [[ -f "$pid_file" ]] || continue
    process_id="$(cat "$pid_file")"
    identity=""
    [[ ! -f "$pid_file.identity" ]] || identity="$(cat "$pid_file.identity")"
    if vpn233_process_matches "$process_id" "$BIN_DIR/$binary" "$CONFIG_DIR/$name.yaml" "$identity"; then
      kill "$process_id" 2>/dev/null || return 1
      for _ in 1 2 3 4 5 6 7 8 9 10; do
        if ! vpn233_process_matches "$process_id" "$BIN_DIR/$binary" "$CONFIG_DIR/$name.yaml" "$identity"; then break; fi
        sleep 0.2
      done
      if vpn233_process_matches "$process_id" "$BIN_DIR/$binary" "$CONFIG_DIR/$name.yaml" "$identity"; then return 1; fi
    fi
    rm -f -- "$pid_file" "$pid_file.identity"
  done
}
