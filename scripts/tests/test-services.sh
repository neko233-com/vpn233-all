#!/usr/bin/env bash
# Offline service ownership tests. systemctl/launchctl are never invoked.
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vpn233-service-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
# shellcheck source=../lib/services.sh
. "$ROOT/scripts/lib/services.sh"
BIN_DIR="$WORK/application with spaces/bin"
CONFIG_DIR="$WORK/application with spaces/config"
export XDG_CONFIG_HOME="$WORK/config-home"
CALLS="$WORK/calls"
: > "$CALLS"
PASS=0
ok() { PASS=$((PASS + 1)); printf '[PASS] %s\n' "$1"; }
reject() { local label="$1"; shift; if "$@"; then printf '[FAIL] %s\n' "$label" >&2; exit 1; fi; ok "$label"; }
# shellcheck disable=SC2329
uname() { printf 'Linux\n'; }
# shellcheck disable=SC2329
systemctl() { printf '%s\n' "$*" >> "$CALLS"; }
UNIT_DIR="$XDG_CONFIG_HOME/systemd/user"
mkdir -p "$UNIT_DIR"
UNIT="$UNIT_DIR/vpn233-provider.service"
printf '[Service]\nExecStart=%s --config %s\n' "$(vpn233_unit_quote "$BIN_DIR/vpn233-provider-server")" "$(vpn233_unit_quote "$CONFIG_DIR/provider-server.yaml")" > "$UNIT"
vpn233_unit_owned "$UNIT" provider; ok 'quoted paths with spaces recognized as owned'
reject 'different installation cannot claim service' env BIN_DIR="$WORK/another/bin" bash -c '. "$1"; vpn233_unit_owned "$2" provider' _ "$ROOT/scripts/lib/services.sh" "$UNIT"
printf '[Service]\nExecStart=/another/install/vpn233-subscribe-server --config /another/config.yaml\n' > "$UNIT_DIR/vpn233-subscribe.service"
vpn233_manage_services stop
[[ "$(cat "$CALLS")" == '--user stop vpn233-provider.service' ]]; ok 'only owned unit stopped'
: > "$CALLS"
vpn233_manage_services start
[[ "$(cat "$CALLS")" == '--user start vpn233-provider.service' ]]; ok 'only owned unit started'
: > "$CALLS"
vpn233_manage_services uninstall
[[ ! -e "$UNIT" && -e "$UNIT_DIR/vpn233-subscribe.service" ]]; ok 'uninstall retains other installation unit'
grep -F -q -- '--user disable --now vpn233-provider.service' "$CALLS"; ok 'uninstall disables owned service before removal'
reject 'missing unit is never owned' vpn233_unit_owned "$UNIT" provider
reject 'invalid PID rejected' vpn233_process_matches not-a-pid /example/invalid
reject 'PID one rejected' vpn233_process_matches 1 /example/invalid
reject 'current process cannot match unrelated executable' vpn233_process_matches "$$" /example/invalid
printf 'test-services.sh: %s passed\n' "$PASS"
