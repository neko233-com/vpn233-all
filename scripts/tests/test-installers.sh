#!/usr/bin/env bash
# Offline release installer smoke tests. All configuration is synthetic.
set -Eeuo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vpn233-install-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
umask 077
unset VPN233_BIN_DIR VPN233_WEB_DIR VPN233_CONFIG_DIR VPN233_DATA_DIR VPN233_ADMIN_USER VPN233_ADMIN_PASSWORD
unset VPN233_PROVIDER_FEED_TOKEN VPN233_REGISTRY_TOKEN VPN233_ALLOW_UNSIGNED VPN233_BUNDLE_SHA256
unset VPN233_PROVIDER_PORT VPN233_SUBSCRIBE_PORT VPN233_CLIENT_SOCKS_PORT VPN233_CLIENT_HTTP_PORT VPN233_CLIENT_WEB_PORT
unset VPN233_UPDATE_BUNDLE VPN233_UPDATE_SHA256 VPN233_UPDATE_URLS VPN233_ALLOW_UNSIGNED_UPDATE
PASS=0
ok() { PASS=$((PASS + 1)); printf '[PASS] %s\n' "$1"; }
reject() {
  local label="$1"; shift
  if "$@" > "$WORK/output" 2>&1; then printf '[FAIL] %s: unexpectedly succeeded\n' "$label" >&2; exit 1; fi
  ok "$label"
}
FIXTURE="$WORK/fixture"
PREFIX="$WORK/install with spaces"
mkdir -p "$FIXTURE/bin" "$FIXTURE/examples" "$FIXTURE/admin-web/dist"
for name in vpn233-provider-server vpn233-subscribe-server vpn233-client; do
  printf '#!/usr/bin/env bash\nexit 0\n' > "$FIXTURE/bin/$name"
  chmod +x "$FIXTURE/bin/$name"
done
for name in provider-server subscribe-server vpn233-client; do cp "$ROOT/$name/config.example.yaml" "$FIXTURE/examples/$name.yaml"; done
printf '<!doctype html><title>offline fixture</title>\n' > "$FIXTURE/admin-web/dist/index.html"
printf '2.1.0\n' > "$FIXTURE/version.txt"
tar -czf "$WORK/fixture.tar.gz" -C "$WORK" fixture
# shellcheck source=../lib/mirrors.sh
. "$ROOT/scripts/lib/mirrors.sh"
HASH="$(vpn233_sha256_file "$WORK/fixture.tar.gz")"
ARGS=(--bundle "$WORK/fixture.tar.gz" --prefix "$PREFIX" --no-start)
bash "$ROOT/install.sh" "${ARGS[@]}" --dry-run > "$WORK/output"
[[ ! -e "$PREFIX" ]]; ok 'dry-run creates no installation'
reject 'missing checksum fails closed' bash "$ROOT/install.sh" "${ARGS[@]}"
[[ ! -e "$PREFIX" ]]
reject 'wrong checksum fails closed' bash "$ROOT/install.sh" "${ARGS[@]}" --sha256 "$(printf '%064d' 0)"
[[ ! -e "$PREFIX" ]]
reject 'bundle version mismatch fails closed' bash "$ROOT/install.sh" "${ARGS[@]}" --sha256 "$HASH" --version 9.9.9
[[ ! -e "$PREFIX" ]]
export VPN233_PROVIDER_PORT=70000
reject 'invalid port fails before installation' bash "$ROOT/install.sh" "${ARGS[@]}" --sha256 "$HASH"
[[ ! -e "$PREFIX" ]]
unset VPN233_PROVIDER_PORT
export VPN233_ADMIN_PASSWORD=root
reject 'default admin password rejected' bash "$ROOT/install.sh" "${ARGS[@]}" --sha256 "$HASH"
[[ ! -e "$PREFIX" ]]
export VPN233_ADMIN_PASSWORD="synthetic'quoted\"password\\with&dollar\$"
bash "$ROOT/install.sh" "${ARGS[@]}" --sha256 "$HASH" > "$WORK/output" 2>&1 || { cat "$WORK/output" >&2; exit 1; }
if grep -F -q "$VPN233_ADMIN_PASSWORD" "$WORK/output"; then printf 'credentials leaked to output\n' >&2; exit 1; fi
ok 'credentials never printed'
for name in vpn233-provider-server vpn233-subscribe-server vpn233-client; do [[ -x "$PREFIX/bin/$name" ]]; done
ok 'all binaries installed'
[[ -f "$PREFIX/admin-web/dist/index.html" ]]; ok 'compiled admin UI installed'
grep -F -q "listen_addr: '127.0.0.1'" "$PREFIX/config/provider-server.yaml"; ok 'provider local binding'
grep -F -q "host: '127.0.0.1'" "$PREFIX/config/subscribe-server.yaml"; ok 'subscribe local binding'
grep -F -q "admin_password: 'synthetic''quoted\"password\\with&dollar\$'" "$PREFIX/config/provider-server.yaml"; ok 'YAML password characters preserved'
BEFORE="$(vpn233_sha256_file "$PREFIX/config/provider-server.yaml")"
printf 'synthetic retained data\n' > "$PREFIX/data/retained.txt"
export VPN233_ADMIN_PASSWORD=replacement-must-not-overwrite
bash "$ROOT/install.sh" "${ARGS[@]}" --sha256 "$HASH" > "$WORK/output" 2>&1
[[ "$(vpn233_sha256_file "$PREFIX/config/provider-server.yaml")" == "$BEFORE" ]]; ok 'reinstall preserves configuration bytes'
[[ "$(cat "$PREFIX/data/retained.txt")" == 'synthetic retained data' ]]; ok 'reinstall preserves runtime data'
reject 'bad update rejected' bash "$ROOT/install.sh" "${ARGS[@]}" --sha256 "$(printf '%064d' 0)"
[[ "$(vpn233_sha256_file "$PREFIX/config/provider-server.yaml")" == "$BEFORE" ]]; ok 'bad update preserves prior configuration'
printf 'previous payload must return after failure\n' > "$PREFIX/bin/vpn233-provider-server"
printf 'previous web must return after failure\n' > "$PREFIX/admin-web/dist/index.html"
mkdir -p "$WORK/mocks"
printf '#!/usr/bin/env bash\nexit 1\n' > "$WORK/mocks/systemctl"
chmod +x "$WORK/mocks/systemctl"
reject 'failed activation returns nonzero' env PATH="$WORK/mocks:$PATH" bash "$ROOT/install.sh" --bundle "$WORK/fixture.tar.gz" --prefix "$PREFIX" --sha256 "$HASH"
[[ "$(cat "$PREFIX/bin/vpn233-provider-server")" == 'previous payload must return after failure' ]]; ok 'failed activation rolls binaries back'
[[ "$(cat "$PREFIX/admin-web/dist/index.html")" == 'previous web must return after failure' ]]; ok 'failed activation rolls admin UI back'
[[ "$(vpn233_sha256_file "$PREFIX/config/provider-server.yaml")" == "$BEFORE" && "$(cat "$PREFIX/data/retained.txt")" == 'synthetic retained data' ]]; ok 'failed activation preserves configuration and runtime data'
reject 'unverified directory bundle refused' bash "$ROOT/install.sh" --bundle "$FIXTURE" --prefix "$WORK/directory-install" --no-start
mkdir -p "$WORK/evil"
printf 'must never extract\n' > "$WORK/evil/safe.txt"
tar -czf "$WORK/traversal.tar.gz" --transform 's|safe.txt|../escaped.txt|' -C "$WORK/evil" safe.txt
EVIL_HASH="$(vpn233_sha256_file "$WORK/traversal.tar.gz")"
reject 'archive path traversal refused' bash "$ROOT/install.sh" --bundle "$WORK/traversal.tar.gz" --sha256 "$EVIL_HASH" --prefix "$WORK/evil-install" --no-start
[[ ! -e "$WORK/escaped.txt" && ! -e "$WORK/evil-install" ]]; ok 'unsafe archive leaves filesystem unchanged'
[[ ! -e "$FIXTURE/config" ]]; ok 'release fixture contains public examples only'
# Persist system/custom layouts, then update from the installed script without
# re-supplying directory environment variables. No real system unit is touched.
. "$ROOT/scripts/lib/layout.sh"
CUSTOM_BIN="$WORK/custom bins"
CUSTOM_WEB="$WORK/custom web"
CUSTOM_CONFIG="$WORK/custom private config"
CUSTOM_DATA="$WORK/custom private data"
mv "$PREFIX/bin" "$CUSTOM_BIN"
mv "$PREFIX/admin-web/dist" "$CUSTOM_WEB"
mv "$PREFIX/config" "$CUSTOM_CONFIG"
mv "$PREFIX/data" "$CUSTOM_DATA"
vpn233_layout_save "$PREFIX" system "$CUSTOM_BIN" "$CUSTOM_WEB" "$CUSTOM_CONFIG" "$CUSTOM_DATA"
mkdir -p "$PREFIX/scripts/lib"
cp "$ROOT/install.sh" "$ROOT/update.sh" "$ROOT/start.sh" "$ROOT/stop.sh" "$PREFIX/"
cp "$ROOT/scripts/lib/mirrors.sh" "$ROOT/scripts/lib/layout.sh" "$ROOT/scripts/lib/services.sh" "$PREFIX/scripts/lib/"
printf '#!/usr/bin/env bash\nprintf "0\\n"\n' > "$WORK/mocks/id"
printf '#!/usr/bin/env bash\nprintf "MINGW64_NT\\n"\n' > "$WORK/mocks/uname"
chmod +x "$WORK/mocks/id" "$WORK/mocks/uname"
VPN233_UPDATE_BUNDLE="$WORK/fixture.tar.gz" VPN233_UPDATE_SHA256="$HASH" PATH="$WORK/mocks:$PATH" bash "$PREFIX/update.sh" --no-start > "$WORK/output" 2>&1 || { cat "$WORK/output" >&2; exit 1; }
[[ "$(vpn233_sha256_file "$CUSTOM_CONFIG/provider-server.yaml")" == "$BEFORE" ]]; ok 'no-argument system update preserves external config bytes'
[[ "$(cat "$CUSTOM_DATA/retained.txt")" == 'synthetic retained data' ]]; ok 'no-argument system update preserves external data'
[[ ! -e "$PREFIX/config" && ! -e "$PREFIX/data" && ! -e "$PREFIX/bin" ]]; ok 'system update never creates alternate default layout'
vpn233_layout_load "$PREFIX"
[[ "$VPN233_SAVED_SCOPE" == system && "$VPN233_SAVED_CONFIG" == "$CUSTOM_CONFIG" && "$VPN233_SAVED_BIN" == "$CUSTOM_BIN" ]]; ok 'saved system scope and custom paths retained'
PATH="$WORK/mocks:$PATH" bash "$PREFIX/update.sh" --dry-run > "$WORK/output"
grep -F -q "$CUSTOM_CONFIG" "$WORK/output"; ok 'installed updater default target uses saved paths'
VPN233_CONFIG_DIR="$WORK/explicit override" PATH="$WORK/mocks:$PATH" bash "$PREFIX/update.sh" --dry-run > "$WORK/output"
grep -F -q "$WORK/explicit override" "$WORK/output"; ok 'explicit environment overrides saved layout'
if PATH="$WORK/mocks:$PATH" bash "$PREFIX/start.sh" > "$WORK/output" 2>&1; then :; fi
if grep -F -q 'Missing Rust binary' "$WORK/output" || grep -F -q 'Missing configuration' "$WORK/output"; then cat "$WORK/output" >&2; exit 1; fi
ok 'runtime start resolves external binary and configuration directories'
printf 'vpn233-layout-v1\nscope\tuser\nbin\trelative/path\nweb\t/tmp/web\nconfig\t/tmp/config\ndata\t/tmp/data\n' > "$PREFIX/.vpn233-layout"
chmod 600 "$PREFIX/.vpn233-layout"
reject 'malformed saved layout fails closed' bash "$PREFIX/update.sh" --dry-run
LEGACY_UNITS="$WORK/legacy-units"
mkdir -p "$LEGACY_UNITS"
printf 'ExecStart="%s/vpn233-provider-server" --config "/etc/vpn233/provider-server.yaml"\n' "$CUSTOM_BIN" > "$LEGACY_UNITS/vpn233-provider.service"
vpn233_layout_legacy "$PREFIX" "$CUSTOM_BIN" "$LEGACY_UNITS"
[[ "$VPN233_SAVED_SCOPE" == system && "$VPN233_SAVED_CONFIG" == /etc/vpn233 && "$VPN233_SAVED_DATA" == /var/lib/vpn233 ]]; ok 'legacy system scope inferred only from exact owned unit'
reject 'ambiguous legacy install refuses new configuration' vpn233_layout_legacy "$PREFIX" "$CUSTOM_BIN" "$WORK/no-legacy-units"
printf 'test-installers.sh: %s passed\n' "$PASS"
