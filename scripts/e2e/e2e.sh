#!/usr/bin/env bash
# End-to-end suite for the VPN233 Rust services.
#
# It starts real provider-server, subscribe-server and vpn233-client processes in
# a throw-away directory, drives the documented admin API and subscription
# endpoints, exercises every subscription format, checks quota/expiry/registry
# semantics, proxies a request through both client listeners and verifies the
# privacy guarantees (no secrets in logs, 0600 data files on Unix, persistence
# across restarts).
#
# This script never writes to the repository and never contacts a third party.
# It needs: bash 4+, curl, a prebuilt vpn233-xtask, and the three release
# binaries in target/release (or BIN_DIR).
#
# Usage:
#   bash scripts/e2e/e2e.sh
#   BIN_DIR=/path/to/bin bash scripts/e2e/e2e.sh
#   E2E_KEEP=1 bash scripts/e2e/e2e.sh   # keep the temp dir for debugging
set -Eeuo pipefail
umask 077
unset GH_TOKEN GITHUB_TOKEN GH_ENTERPRISE_TOKEN GITHUB_ENTERPRISE_TOKEN

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN_DIR="${BIN_DIR:-$ROOT_DIR/target/release}"
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/vpn233-e2e-XXXXXX")"
CONFIG_DIR="$WORK_DIR/config"
LOG_DIR="$WORK_DIR/logs"
DATA_DIR="$WORK_DIR/data"
WEB_DIR="$WORK_DIR/web"
RESP_BODY="$WORK_DIR/resp.body"
RESP_HEADERS="$WORK_DIR/resp.headers"

mkdir -p "$CONFIG_DIR" "$LOG_DIR" "$DATA_DIR/subscribe" "$DATA_DIR/client" "$WEB_DIR"
E2E_WEB_DIR="${E2E_WEB_DIR:-$ROOT_DIR/admin-web/dist}"
[ -f "$E2E_WEB_DIR/index.html" ] || { echo 'Build admin-web before E2E; generated UI is required' >&2; exit 1; }
cp -R "$E2E_WEB_DIR/." "$WEB_DIR/"

PIDS=()
STEP_NAMES=()
STEP_MS=()

cleanup() {
  local status=$?
  local pid
  for pid in "${PIDS[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  local attempt active
  for attempt in {1..30}; do
    active=0
    for pid in "${PIDS[@]}"; do
      if kill -0 "$pid" 2>/dev/null; then active=1; fi
    done
    [ "$active" -eq 1 ] || break
    sleep 0.1
  done
  for pid in "${PIDS[@]}"; do
    kill -KILL "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  done
  if [ "$status" -ne 0 ]; then
    printf '\nE2E working directory kept for diagnosis: %s\n' "$WORK_DIR" >&2
  elif [ "${E2E_KEEP:-0}" = "1" ]; then
    printf 'E2E working directory kept: %s\n' "$WORK_DIR"
  else
    rm -rf "$WORK_DIR"
  fi
  exit "$status"
}
trap cleanup EXIT

# --- helpers -------------------------------------------------------------
fail() {
  printf '\nE2E FAILED: %s\n' "$*" >&2
  printf 'Runtime responses and logs remain local; no payload is printed.\n' >&2
  exit 1
}

note() { printf '  - %s\n' "$*"; }

step_begin() {
  STEP_NAME="$1"
  STEP_START="$(date +%s%3N)"
  printf '\n[%s]\n' "$1"
}

step_end() {
  local elapsed=$(( $(date +%s%3N) - STEP_START ))
  STEP_NAMES+=("$STEP_NAME")
  STEP_MS+=("$elapsed")
  printf '  ok (%s ms)\n' "$elapsed"
}

XTASK="${VPN233_XTASK_BIN:-$ROOT_DIR/target/debug/vpn233-xtask}"
if [ ! -f "$XTASK" ] && [ -f "$XTASK.exe" ]; then XTASK="$XTASK.exe"; fi
[ -f "$XTASK" ] || fail "prebuild vpn233-xtask or set VPN233_XTASK_BIN before E2E"

random_hex() {
  "$XTASK" network e2e random-hex "$1"
}

json_get() {
  local document="$1"; shift
  printf '%s' "$document" | "$XTASK" network e2e "$@"
}

http() {
  # $1 = method, $2 = url, rest = curl arguments. Prints the HTTP status code.
  local method="$1"
  local url="$2"
  shift 2
  local code
  code="$(curl -sS --max-time 20 -o "$RESP_BODY" -D "$RESP_HEADERS" \
    -w '%{http_code}' -X "$method" "$@" "$url" 2>/dev/null)" || code="000"
  printf '%s' "$code"
}

header_value() {
  grep -i -m1 "^$1:" "$RESP_HEADERS" 2>/dev/null |
    sed -e 's/^[^:]*:[[:space:]]*//' -e 's/\r$//' || true
}

assert_status() {
  local expected="$1" actual="$2" label="$3"
  if [ "$actual" != "$expected" ]; then
    fail "$label: expected HTTP $expected, got $actual (response redacted)"
  fi
}

assert_header_contains() {
  local header="$1" needle="$2" label="$3" value
  value="$(header_value "$header")"
  case "$value" in
    *"$needle"*) ;;
    *) fail "$label: header '$header' did not match (value redacted)" ;;
  esac
}

assert_body_contains() {
  local needle="$1" label="$2"
  if ! grep -qF -- "$needle" "$RESP_BODY"; then
    fail "$label: response body did not match (value redacted)"
  fi
}

assert_body_omits() {
  local needle="$1" label="$2"
  if grep -qF -- "$needle" "$RESP_BODY"; then
    fail "$label: response body contains a forbidden value (redacted)"
  fi
}

wait_for_http() {
  # $1 = url, $2 = timeout seconds. Succeeds on any HTTP status (even 4xx/5xx).
  local url="$1" timeout="${2:-30}" deadline code
  deadline=$(( $(date +%s) + timeout ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    code="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 2 "$url" 2>/dev/null)" || code="000"
    if [ "$code" != "000" ]; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

stop_pid() {
  local pid="$1"
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

http_base() { printf 'http://127.0.0.1:%s' "$1"; }

# --- locate binaries -----------------------------------------------------
BIN_SUFFIX=""
for candidate in "" ".exe"; do
  if [ -f "$BIN_DIR/vpn233-provider-server${candidate}" ]; then
    BIN_SUFFIX="$candidate"
    break
  fi
done
for binary in vpn233-provider-server vpn233-subscribe-server vpn233-client; do
  if [ ! -f "$BIN_DIR/${binary}${BIN_SUFFIX}" ]; then
    fail "missing binary $BIN_DIR/${binary}${BIN_SUFFIX}; run: cargo build --release --workspace"
  fi
done
PROVIDER_BIN="$BIN_DIR/vpn233-provider-server${BIN_SUFFIX}"
SUBSCRIBE_BIN="$BIN_DIR/vpn233-subscribe-server${BIN_SUFFIX}"
CLIENT_BIN="$BIN_DIR/vpn233-client${BIN_SUFFIX}"

# --- ports and secrets ---------------------------------------------------
read -r SUB_PORT PROVIDER_PORT CLIENT_SOCKS_PORT CLIENT_HTTP_PORT CLIENT_WEB_PORT <<< "$("$XTASK" network e2e free-ports 5)"

SUB_ADMIN_PASSWORD="sub-$(random_hex 12)"
PROVIDER_ADMIN_PASSWORD="prov-$(random_hex 12)"
CLIENT_PASSWORD="client-$(random_hex 12)"
REGISTRY_TOKEN="registry-$(random_hex 16)"
PROVIDER_FEED_TOKEN="feed-$(random_hex 16)"
SUB_TOKEN="e2etoken$(random_hex 10)"
SUB_PASSWORD="e2epass$(random_hex 10)"
QUOTA_TOKEN="quotatoken$(random_hex 10)"
QUOTA_PASSWORD="quotapass$(random_hex 10)"
EXPIRED_TOKEN="expiredtoken$(random_hex 10)"
EXPIRED_PASSWORD="expiredpass$(random_hex 10)"

SUB_BASE="$(http_base "$SUB_PORT")"
PROVIDER_BASE="$(http_base "$PROVIDER_PORT")"
CLIENT_WEB_BASE="$(http_base "$CLIENT_WEB_PORT")"

# --- test node links (only reserved example domains) ---------------------
VLESS_LINK="vless://11111111-1111-4111-8111-111111111111@example.invalid:443?security=tls&type=ws&path=%2Fws&sni=example.invalid&fp=chrome#vless-e2e"
VMESS_JSON='{"v":"2","ps":"vmess-e2e","add":"example.invalid","port":"443","id":"22222222-2222-4222-8222-222222222222","aid":"0","scy":"auto","net":"ws","type":"none","host":"example.invalid","path":"/ws","tls":"tls","sni":"example.invalid","fp":"chrome"}'
VMESS_LINK="vmess://$(printf '%s' "$VMESS_JSON" | "$XTASK" network e2e base64-encode)"
TROJAN_LINK="trojan://e2e-password@example.invalid:443?sni=example.invalid#trojan-e2e"
SS_LINK="ss://$(printf '%s' 'aes-256-gcm:e2e-password' | "$XTASK" network e2e base64-encode)@example.invalid:8388#ss-e2e"
PROVIDER_NODE_LINK="vless://33333333-3333-4333-8333-333333333333@provider.example.invalid:443?security=tls&type=tcp&sni=provider.example.invalid#provider-e2e-node"

# --- generate configuration ----------------------------------------------
write_subscribe_config() {
  cat > "$CONFIG_DIR/subscribe.yaml" <<EOF
server:
  host: "127.0.0.1"
  http_port: ${SUB_PORT}
  default_format: "clash"
  data_dir: "${DATA_DIR}/subscribe"
  admin_user: "admin"
  admin_password: "${SUB_ADMIN_PASSWORD}"
  admin_origins: "http://127.0.0.1:${SUB_PORT}"
  web_dir: "${WEB_DIR}"
  provider_registry_token: "${REGISTRY_TOKEN}"
  provider_stale_after_secs: 120
  client_stats_url: ""
  client_stats_user: ""
  client_stats_password: ""
  traffic_timezone_offset_hours: 8
subscriptions:
  - name: "e2e-static"
    links:
      - "${VLESS_LINK}"
      - "${VMESS_LINK}"
      - "${TROJAN_LINK}"
      - "${SS_LINK}"
    url: ""
    access_token: ""
EOF
}

write_provider_config() {
  cat > "$CONFIG_DIR/provider.yaml" <<EOF
listen_addr: "127.0.0.1"
listen_port: ${PROVIDER_PORT}
admin_user: "root"
admin_password: "${PROVIDER_ADMIN_PASSWORD}"
default_node_ip: "127.0.0.1"
default_port_base: $((20000 + (PROVIDER_PORT % 1000)))
provider_access_token: "${PROVIDER_FEED_TOKEN}"
mihomo_url: ""
mihomo_sha256: ""
mihomo_version: "v1.19.0"
subscribe_verify_token: ""
node_links:
  - "${PROVIDER_NODE_LINK}"
registry_url: "${SUB_BASE}"
registry_token: "${REGISTRY_TOKEN}"
registry_provider_id: "e2e-provider"
registry_name: "e2e-provider"
heartbeat_interval_secs: 10
advertise_addresses:
  - kind: "local"
    value: "127.0.0.1"
enabled_protocols: []
auto_detect_public_ip: false
EOF
}

write_client_config() {
  cat > "$CONFIG_DIR/client.yaml" <<EOF
listen_addr: "127.0.0.1"
socks_port: ${CLIENT_SOCKS_PORT}
http_port: ${CLIENT_HTTP_PORT}
web_addr: "127.0.0.1:${CLIENT_WEB_PORT}"
admin_user: "root"
admin_password: "${CLIENT_PASSWORD}"
upstream_mode: "direct"
subscription_url: ""
stats_file: "${DATA_DIR}/client/stats.json"
stats_timezone_offset_hours: 8
upstream_socks5: ""
socks_user: ""
socks_password: ""
http_user: ""
http_password: ""
record_domains: false
EOF
}

write_subscribe_config
write_provider_config
write_client_config

# --- start services ------------------------------------------------------
LAST_PID=""
start_service() {
  # $1 = binary, $2 = config, $3 = log file. Sets LAST_PID.
  (
    for variable in "${!VPN233_@}"; do unset "$variable"; done
    exec "$1" --config "$2"
  ) >"$3" 2>&1 &
  LAST_PID=$!
  PIDS+=("$LAST_PID")
}

step_begin "start services"
start_service "$SUBSCRIBE_BIN" "$CONFIG_DIR/subscribe.yaml" "$LOG_DIR/subscribe.log"
SUB_PID="$LAST_PID"
start_service "$PROVIDER_BIN" "$CONFIG_DIR/provider.yaml" "$LOG_DIR/provider.log"
start_service "$CLIENT_BIN" "$CONFIG_DIR/client.yaml" "$LOG_DIR/client.log"
wait_for_http "$SUB_BASE/api/v1/formats" 30 || fail "subscribe-server did not become ready on $SUB_BASE"
wait_for_http "$PROVIDER_BASE/api/v1/health" 30 || fail "provider-server did not become ready on $PROVIDER_BASE"
wait_for_http "$CLIENT_WEB_BASE/api/v1/health" 30 || fail "vpn233-client did not become ready on $CLIENT_WEB_BASE"
code="$(http GET "$SUB_BASE/")"
assert_status 200 "$code" 'generated admin UI'
assert_body_contains 'id="root"' 'generated admin UI root'
note "subscribe=$SUB_BASE provider=$PROVIDER_BASE client=$CLIENT_WEB_BASE"
step_end

# --- admin API: login, account, link ------------------------------------
step_begin "admin login and access token"
AUTH=()
body="$(printf '{"username":"admin","password":"%s"}' "$SUB_ADMIN_PASSWORD")"
code="$(http POST "$SUB_BASE/api/auth/login" -H 'Content-Type: application/json' -d "$body")"
assert_status 200 "$code" "admin login"
ADMIN_JWT="$(json_get "$(cat "$RESP_BODY")" json-get /token)"
[ -n "$ADMIN_JWT" ] || fail "admin login returned an empty token"
AUTH=(-H "Authorization: Bearer $ADMIN_JWT")
note "admin JWT acquired"

code="$(http GET "$SUB_BASE/api/status" "${AUTH[@]}")"
assert_status 200 "$code" "authenticated /api/status"
code="$(http GET "$SUB_BASE/api/status")"
assert_status 401 "$code" "unauthenticated /api/status"
step_end

step_begin "capability endpoints"
code="$(http GET "$SUB_BASE/api/v1/formats")"
assert_status 200 "$code" "/api/v1/formats"
assert_header_contains "content-type" "application/json" "/api/v1/formats"
formats_body="$(cat "$RESP_BODY")"
[ "$(json_get "$formats_body" json-length /formats)" -ge 9 ] || fail "/api/v1/formats: expected at least 9 formats"
[ "$(json_get "$formats_body" json-contains /protocols vless)" = "true" ] || fail "/api/v1/formats: vless missing from protocols"

code="$(http POST "$PROVIDER_BASE/api/v1/login" -H 'Content-Type: application/json' \
  -d "$(printf '{"username":"root","password":"%s"}' "$PROVIDER_ADMIN_PASSWORD")")"
assert_status 200 "$code" "provider login"
PROVIDER_JWT="$(json_get "$(cat "$RESP_BODY")" json-get /token)"
PROVIDER_AUTH=(-H "Authorization: Bearer $PROVIDER_JWT")

code="$(http GET "$PROVIDER_BASE/api/v1/generate?format=json" "${PROVIDER_AUTH[@]}")"
assert_status 200 "$code" "provider generate json"
[ "$(json_get "$(cat "$RESP_BODY")" json-length /node/ports)" -ge 3 ] || fail "provider generate: fewer than 3 node ports"
code="$(http GET "$PROVIDER_BASE/api/v1/generate.sh" "${PROVIDER_AUTH[@]}")"
assert_status 200 "$code" "provider generate.sh"
assert_body_contains "#!/usr/bin/env bash" "provider generate.sh"
code="$(http GET "$PROVIDER_BASE/api/v1/generate.ps1" "${PROVIDER_AUTH[@]}")"
assert_status 200 "$code" "provider generate.ps1"
assert_body_contains "Set-StrictMode" "provider generate.ps1"
code="$(http GET "$PROVIDER_BASE/api/v1/nodes?token=wrong-token")"
assert_status 401 "$code" "provider nodes with wrong token"
code="$(http GET "$PROVIDER_BASE/api/v1/nodes" -H "x-provider-token: $PROVIDER_FEED_TOKEN")"
assert_status 200 "$code" "provider nodes feed"
assert_body_contains "$PROVIDER_NODE_LINK" "provider nodes feed"
step_end

step_begin "create account and subscription link"
code="$(http POST "$SUB_BASE/api/admin/accounts" "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d '{"username":"e2e-user","display_name":"E2E user"}')"
assert_status 201 "$code" "create account"
ACCOUNT_ID="$(json_get "$(cat "$RESP_BODY")" json-get /account/account_id)"
[ -n "$ACCOUNT_ID" ] || fail "create account: empty account_id"

code="$(http POST "$SUB_BASE/api/admin/accounts/$ACCOUNT_ID/links" "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d "$(printf '{"token":"%s","password":"%s","limit_bytes":104857600,"label":"e2e-primary"}' "$SUB_TOKEN" "$SUB_PASSWORD")")"
assert_status 201 "$code" "create account link"
LINKED_TOKEN="$(json_get "$(cat "$RESP_BODY")" json-get /link/token)"
[ "$LINKED_TOKEN" = "$SUB_TOKEN" ] || fail "create account link: token mismatch"

code="$(http POST "$SUB_BASE/api/admin/tokens/$SUB_TOKEN/link" "${AUTH[@]}")"
assert_status 200 "$code" "create subscription access token"
SUB_ACCESS="$(json_get "$(cat "$RESP_BODY")" json-get /access_token)"
[ -n "$SUB_ACCESS" ] || fail "subscription access token is empty"

code="$(http GET "$SUB_BASE/api/admin/accounts/$ACCOUNT_ID/links" "${AUTH[@]}")"
assert_status 200 "$code" "list account links"
[ "$(json_get "$(cat "$RESP_BODY")" json-length /links)" -ge 1 ] || fail "account has no links"
step_end

# --- every subscription format ------------------------------------------
ctype_for() {
  case "$1" in
    clash|stash) printf 'text/yaml' ;;
    singbox) printf 'application/json' ;;
    *) printf 'text/plain' ;;
  esac
}
ext_for() {
  case "$1" in
    clash|stash) printf 'yaml' ;;
    singbox) printf 'json' ;;
    surge|quantumultx|loon|surfboard) printf 'conf' ;;
    *) printf 'txt' ;;
  esac
}
signature_for() {
  case "$1" in
    clash|stash) printf 'proxies:' ;;
    singbox) printf '"outbounds"' ;;
    v2ray|links) printf '://' ;;
    surge|surfboard) printf '[Proxy]' ;;
    quantumultx) printf '[server_local]' ;;
    loon) printf '[Proxy]' ;;
    *) printf 'vpn233' ;;
  esac
}
protocol_marker_for() {
  case "$1" in
    clash|stash) printf 'type: vless' ;;
    singbox) printf '"type": "vless"' ;;
    surge|surfboard) printf '= vmess,' ;;
    quantumultx) printf 'vless=' ;;
    loon) printf '= VLESS,' ;;
    *) printf 'vless://' ;;
  esac
}

step_begin "subscription formats"
declare -a FORMATS=(clash singbox v2ray links surge quantumultx loon surfboard stash)
for format in "${FORMATS[@]}"; do
  expected_type="$(ctype_for "$format")"
  extension="$(ext_for "$format")"
  signature="$(signature_for "$format")"
  marker="$(protocol_marker_for "$format")"
  url="$SUB_BASE/sub/$format?token=$SUB_TOKEN&access_token=$SUB_ACCESS&filename=vpn233-$format.$extension"
  code="$(http GET "$url")"
  assert_status 200 "$code" "format $format"
  assert_header_contains "content-type" "$expected_type" "format $format"
  assert_header_contains "x-vpn233-format" "$format" "format $format"
  assert_header_contains "content-disposition" ".$extension" "format $format filename"
  assert_header_contains "subscription-userinfo" "total=" "format $format userinfo"
  assert_header_contains "cache-control" "no-store" "format $format cache"
  node_count="$(header_value "x-vpn233-nodes")"
  case "$node_count" in
    ''|*[!0-9]*) fail "format $format: X-VPN233-Nodes is not an integer ('$node_count')" ;;
  esac
  [ "$node_count" -ge 1 ] || fail "format $format: rendered zero nodes"
  skipped="$(header_value "x-vpn233-skipped")"
  case "$skipped" in
    ''|*[!0-9]*) fail "format $format: X-VPN233-Skipped is not an integer ('$skipped')" ;;
  esac

  case "$format" in
    singbox)
      "$XTASK" network e2e validate-json < "$RESP_BODY" >/dev/null 2>&1 || fail "format singbox: invalid JSON"
      assert_body_contains '"outbounds"' "format singbox"
      ;;
    v2ray)
      decoded="$WORK_DIR/v2ray.decoded"
      "$XTASK" network e2e base64-decode < "$RESP_BODY" > "$decoded" 2>/dev/null || fail "format v2ray: body is not valid base64"
      grep -qF '://' "$decoded" || fail "format v2ray: decoded body has no proxy links"
      ;;
    clash|stash)
      assert_body_contains 'proxies:' "format $format"
      "$XTASK" network e2e validate-yaml < "$RESP_BODY" >/dev/null 2>&1 || fail "format $format: invalid YAML"
      ;;
  esac
  if [ "$format" = v2ray ]; then
    grep -qF -- "$signature" "$decoded" || fail 'format v2ray: decoded syntax signature missing'
    grep -qF -- "$marker" "$decoded" || fail 'format v2ray: decoded protocol syntax marker missing'
    "$XTASK" network e2e validate-links "$node_count" < "$decoded" 2>/dev/null || fail 'format v2ray: decoded link count/scheme does not match metadata'
  else
    assert_body_contains "$signature" "format $format"
    assert_body_contains "$marker" "format $format protocol marker"
  fi
  note "format $format: $(header_value 'content-type'), nodes=$node_count, $(wc -c < "$RESP_BODY" | tr -d ' ') bytes"
done

code="$(http GET "$SUB_BASE/sub?token=$SUB_TOKEN&access_token=$SUB_ACCESS")"
assert_status 200 "$code" "default subscription endpoint"
assert_header_contains "x-vpn233-format" "clash" "default subscription format"
code="$(http GET "$SUB_BASE/sub/c?token=$SUB_TOKEN&access_token=$SUB_ACCESS&format=singbox")"
assert_status 200 "$code" "format override on /sub/c"
assert_header_contains "x-vpn233-format" "singbox" "format override on /sub/c"
code="$(http GET "$SUB_BASE/sub/clash?token=$SUB_TOKEN&access_token=$SUB_ACCESS&format=not-a-format")"
assert_status 400 "$code" "invalid format override"
code="$(http GET "$SUB_BASE/sub/clash")"
assert_status 401 "$code" "subscription without credentials"
code="$(http GET "$SUB_BASE/sub/clash?token=$SUB_TOKEN&password=wrong-password")"
assert_status 401 "$code" "subscription with wrong password"
code="$(http GET "$SUB_BASE/sub/links?token=$SUB_TOKEN&password=$SUB_PASSWORD")"
assert_status 200 "$code" "subscription with token + password"
step_end

# --- quota ---------------------------------------------------------------
step_begin "quota enforcement"
code="$(http POST "$SUB_BASE/api/admin/accounts/$ACCOUNT_ID/links" "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d "$(printf '{"token":"%s","password":"%s","limit_bytes":1,"label":"e2e-quota"}' "$QUOTA_TOKEN" "$QUOTA_PASSWORD")")"
assert_status 201 "$code" "create quota link"
code="$(http POST "$SUB_BASE/api/admin/tokens/$QUOTA_TOKEN/link" "${AUTH[@]}")"
assert_status 200 "$code" "quota access token"
QUOTA_ACCESS="$(json_get "$(cat "$RESP_BODY")" json-get /access_token)"
quota_hit=0
attempt=1
while [ "$attempt" -le 5 ]; do
  code="$(http GET "$SUB_BASE/sub/links?token=$QUOTA_TOKEN&access_token=$QUOTA_ACCESS")"
  if [ "$code" = "429" ]; then
    quota_hit=1
    break
  fi
  attempt=$((attempt + 1))
done
[ "$quota_hit" -eq 1 ] || fail "quota link never returned HTTP 429 (last status '$code')"
note "quota exceeded after $attempt request(s)"
step_end

step_begin "expired token"
past="$(( $(date +%s) - 3600 ))"
code="$(http POST "$SUB_BASE/api/admin/accounts/$ACCOUNT_ID/links" "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d "$(printf '{"token":"%s","password":"%s","limit_bytes":1048576,"expires_at":%s,"label":"e2e-expired"}' "$EXPIRED_TOKEN" "$EXPIRED_PASSWORD" "$past")")"
assert_status 201 "$code" "create expired link"
code="$(http GET "$SUB_BASE/sub/links?token=$EXPIRED_TOKEN&password=$EXPIRED_PASSWORD")"
assert_status 401 "$code" "expired token"
step_end

# --- provider registry ---------------------------------------------------
step_begin "provider registry"
registry_online="false"
deadline=$(( $(date +%s) + 40 ))
while [ "$(date +%s)" -lt "$deadline" ]; do
  code="$(http GET "$SUB_BASE/api/admin/providers" "${AUTH[@]}")"
  if [ "$code" = "200" ]; then
    registry_online="$(json_get "$(cat "$RESP_BODY")" provider-online e2e-provider)"
    if [ "$registry_online" = "true" ]; then
      break
    fi
  fi
  sleep 1
done
[ "$registry_online" = "true" ] || fail "provider never appeared online in /api/admin/providers"
step_end

step_begin "registered nodes appear in subscriptions"
found="false"
deadline=$(( $(date +%s) + 20 ))
while [ "$(date +%s)" -lt "$deadline" ]; do
  code="$(http GET "$SUB_BASE/sub/links?token=$SUB_TOKEN&access_token=$SUB_ACCESS")"
  if [ "$code" = "200" ] && grep -qF 'provider.example.invalid' "$RESP_BODY"; then
    found="true"
    break
  fi
  sleep 1
done
[ "$found" = "true" ] || fail "registered provider node did not appear in the subscription"
note "provider node is visible in /sub/links"
step_end

# --- client proxy --------------------------------------------------------
step_begin "client proxy (SOCKS5 and HTTP)"
ECHO_READY="$WORK_DIR/echo.ready"
"$XTASK" network echo --port 0 --ready-file "$ECHO_READY" --marker "vpn233-e2e-echo" \
  >"$LOG_DIR/echo.log" 2>&1 &
ECHO_PID=$!
PIDS+=("$ECHO_PID")
deadline=$(( $(date +%s) + 15 ))
ECHO_PORT=""
while [ "$(date +%s)" -lt "$deadline" ]; do
  if ! kill -0 "$ECHO_PID" 2>/dev/null; then
    echo_status=0
    wait "$ECHO_PID" || echo_status=$?
    fail "echo fixture exited before readiness (exit=$echo_status; log contents remain local)"
  fi
  if [ -s "$ECHO_READY" ] && [ ! -L "$ECHO_READY" ]; then
    if IFS= read -r ECHO_PORT < "$ECHO_READY"; then
      [[ "$ECHO_PORT" =~ ^[1-9][0-9]{0,4}$ ]] && [ "$ECHO_PORT" -le 65535 ] || fail "echo fixture readiness has an invalid port (value redacted)"
      break
    fi
    ECHO_PORT=""
  fi
  sleep 0.1
done
[ -n "$ECHO_PORT" ] || fail "echo fixture did not publish readiness within 15 seconds"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) ;;
  *) [ "$(stat -c '%a' "$ECHO_READY")" = 600 ] || fail "echo fixture readiness is not private" ;;
esac
ECHO_BASE="$(http_base "$ECHO_PORT")"
remaining=$(( deadline - $(date +%s) ))
[ "$remaining" -gt 0 ] && wait_for_http "$ECHO_BASE/" "$remaining" || fail "echo fixture did not serve HTTP before the readiness deadline"

socks_body="$(curl -sS --max-time 20 --socks5-hostname "127.0.0.1:$CLIENT_SOCKS_PORT" "$ECHO_BASE/echo/socks")" ||
  fail "SOCKS5 proxy request failed"
case "$socks_body" in
  *vpn233-e2e-echo*'/echo/socks'*) ;;
  *) fail "SOCKS5 proxy returned unexpected body (response redacted)" ;;
esac
note "SOCKS5 proxy request reached the echo server"

http_body="$(curl -sS --max-time 20 -x "http://127.0.0.1:$CLIENT_HTTP_PORT" "$ECHO_BASE/echo/http")" ||
  fail "HTTP proxy request failed"
case "$http_body" in
  *vpn233-e2e-echo*'/echo/http'*) ;;
  *) fail "HTTP proxy returned unexpected body (response redacted)" ;;
esac
note "HTTP proxy request reached the echo server"
step_end

step_begin "client stats and privacy endpoints"
code="$(http GET "$CLIENT_WEB_BASE/api/v1/stats" -u "root:$CLIENT_PASSWORD")"
assert_status 200 "$code" "client stats"
stats_body="$(cat "$RESP_BODY")"
total_connections="$(json_get "$stats_body" json-get /total_connections)"
[ "$total_connections" -ge 2 ] || fail "client stats: total_connections=$total_connections, expected at least 2"
domains="$(json_get "$stats_body" json-length /domains)"
[ "$domains" -eq 0 ] || fail "client stats: record_domains=false but $domains domain(s) were recorded"

code="$(http GET "$CLIENT_WEB_BASE/api/v1/stats")"
assert_status 401 "$code" "client stats without credentials"
code="$(http GET "$CLIENT_WEB_BASE/api/v1/config" -u "root:$CLIENT_PASSWORD")"
assert_status 200 "$code" "client config endpoint"
config_body="$(cat "$RESP_BODY")"
case "$config_body" in
  *"$CLIENT_PASSWORD"*) fail "client config endpoint leaked the admin password" ;;
esac
code="$(http GET "$CLIENT_WEB_BASE/proxy.pac")"
assert_status 200 "$code" "client PAC endpoint"
assert_header_contains "content-type" "application/x-ns-proxy-autoconfig" "client PAC endpoint"
step_end

# --- privacy: logs and data file permissions -----------------------------
check_log_privacy() {
  local secret log
  for secret in "$SUB_ADMIN_PASSWORD" "$PROVIDER_ADMIN_PASSWORD" "$CLIENT_PASSWORD" \
    "$SUB_ACCESS" "$REGISTRY_TOKEN" "$PROVIDER_FEED_TOKEN" "$SUB_TOKEN" "$QUOTA_TOKEN" "$ADMIN_JWT" "$PROVIDER_JWT" "$QUOTA_ACCESS" "${RESTART_ADMIN_JWT:-}"; do
    [ -n "$secret" ] || continue
    for log in "$LOG_DIR"/*; do
    [ -f "$log" ] || continue
    if grep -aF -- "$secret" "$log" >/dev/null 2>&1; then
      fail "privacy: a secret leaked into $(basename "$log")"
    fi
    done
  done
}
step_begin "privacy checks (logs and data files)"
check_log_privacy
note "no credentials, access tokens or feed tokens in service logs"

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    note "skipping 0600 data-file assertion on Windows (POSIX permissions are not applied)"
    ;;
  *)
    data_files="$(find "$DATA_DIR" -type f -name '*.json' | LC_ALL=C sort)"
    [ -n "$data_files" ] || fail "no persisted JSON data files were found under $DATA_DIR"
    while IFS= read -r data_file; do
      [ -n "$data_file" ] || continue
      mode="$(stat -c '%a' "$data_file")"
      [ "$mode" = "600" ] || fail "data file $(basename "$data_file") has mode $mode, expected 600"
    done <<< "$data_files"
    note "all $(printf '%s\n' "$data_files" | wc -l | tr -d ' ') persisted data files are mode 600"
    ;;
esac
step_end

# --- persistence across restart -----------------------------------------
step_begin "persistence across subscribe-server restart"
stop_pid "$SUB_PID"
sleep 0.5
start_service "$SUBSCRIBE_BIN" "$CONFIG_DIR/subscribe.yaml" "$LOG_DIR/subscribe-restart.log"
SUB_PID="$LAST_PID"
wait_for_http "$SUB_BASE/api/v1/formats" 30 || fail "subscribe-server did not restart"
code="$(http GET "$SUB_BASE/sub/links?token=$SUB_TOKEN&access_token=$SUB_ACCESS")"
assert_status 200 "$code" "token after subscribe restart"
assert_body_contains "example.invalid" "subscription after restart"
code="$(http POST "$SUB_BASE/api/auth/login" -H 'Content-Type: application/json' \
  -d "$(printf '{"username":"admin","password":"%s"}' "$SUB_ADMIN_PASSWORD")")"
assert_status 200 "$code" "admin login after restart"
RESTART_ADMIN_JWT="$(json_get "$(cat "$RESP_BODY")" json-get /token)"
note "token and admin credentials survived the restart"
step_end
step_begin 'privacy across restart (stdout and stderr)'
check_log_privacy
step_end

# --- summary -------------------------------------------------------------
printf '\n============================================\n'
printf 'E2E PASSED\n'
printf '============================================\n'
total_ms=0
index=0
while [ "$index" -lt "${#STEP_NAMES[@]}" ]; do
  printf '  %-44s %8s ms\n' "${STEP_NAMES[$index]}" "${STEP_MS[$index]}"
  total_ms=$((total_ms + STEP_MS[index]))
  index=$((index + 1))
done
printf '  %-44s %8s ms\n' "total" "$total_ms"
printf '============================================\n'
