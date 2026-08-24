#!/usr/bin/env bash
set -euo pipefail

# Read-only backend audit. Credentials must come from the environment and are
# never printed, persisted, or included in a request URL.
: "${VPN233_SUBSCRIBE_URL:?set VPN233_SUBSCRIBE_URL, for example http://127.0.0.1:23444}"
: "${VPN233_ADMIN_USER:?set VPN233_ADMIN_USER}"
: "${VPN233_ADMIN_PASSWORD:?set VPN233_ADMIN_PASSWORD}"

base_url="${VPN233_SUBSCRIBE_URL%/}"
basic_auth="$(printf '%s:%s' "$VPN233_ADMIN_USER" "$VPN233_ADMIN_PASSWORD" | base64 -w0)"
auth_header="Authorization: Basic $basic_auth"

read_tokens() {
  curl -fsS -H "$auth_header" "$base_url/api/admin/tokens"
}

active_count() {
  local payload="$1"
  local now
  now="$(date +%s)"
  jq --argjson now "$now" '[.tokens[] | select(.enabled == true and (.expires_at == null or .expires_at > $now))] | length' <<<"$payload"
}

body_hash() {
  printf '%s' "$1" | sha256sum | cut -d' ' -f1
}

tokens_before="$(read_tokens)"
token_count_before="$(jq '.tokens | length' <<<"$tokens_before")"
active_before="$(active_count "$tokens_before")"
test "$token_count_before" = 1
test "$active_before" = 1

token="$(jq -r '.tokens[0].token // empty' <<<"$tokens_before")"
test -n "$token"
token_encoded="$(jq -rn --arg value "$token" '$value | @uri')"

link_one="$(curl -fsS -X POST -H "$auth_header" "$base_url/api/admin/tokens/$token_encoded/link")"
link_two="$(curl -fsS -X POST -H "$auth_header" "$base_url/api/admin/tokens/$token_encoded/link")"
access_one="$(jq -r '.access_token // empty' <<<"$link_one")"
access_two="$(jq -r '.access_token // empty' <<<"$link_two")"
test -n "$access_one"
test "$access_one" = "$access_two"

access_encoded="$(jq -rn --arg value "$access_one" '$value | @uri')"
subscription_url="$base_url/sub/c?token=$token_encoded&access_token=$access_encoded&filename=clash"
subscription_http_one="$(curl -sS -o /dev/null -w '%{http_code}' "$subscription_url")"
subscription_http_two="$(curl -sS -o /dev/null -w '%{http_code}' "$subscription_url")"
test "$subscription_http_one" = 200
test "$subscription_http_two" = 200

# Keep responses in memory only. This intentionally does not write a
# subscription body, token list, or link to disk.
body_one="$(curl -fsS "$subscription_url")"
body_two="$(curl -fsS "$subscription_url")"
hash_one="$(body_hash "$body_one")"
hash_two="$(body_hash "$body_two")"
test "$hash_one" = "$hash_two"
geodata_rules="$(printf '%s' "$body_one" | grep -E -c 'GEOSITE,|GEOIP,' || true)"
proxy_sections="$(printf '%s' "$body_one" | grep -c '^proxies:' || true)"
test "$geodata_rules" = 0
test "$proxy_sections" -ge 1

tokens_after="$(read_tokens)"
token_count_after="$(jq '.tokens | length' <<<"$tokens_after")"
active_after="$(active_count "$tokens_after")"
cleanup_candidates="$(jq --arg keep "$token" --argjson now "$(date +%s)" '[.tokens[] | select(.token != $keep and .enabled == true and (.expires_at == null or .expires_at > $now))] | length' <<<"$tokens_after")"
test "$token_count_after" = 1
test "$active_after" = 1
test "$cleanup_candidates" = 0

printf 'backend_idempotency=pass token_count_before=%s active_before=%s link_same=true subscription_http_one=%s subscription_http_two=%s body_hash_same=true geodata_rules=%s token_count_after=%s active_after=%s cleanup_candidates=%s\n' \
  "$token_count_before" "$active_before" "$subscription_http_one" "$subscription_http_two" \
  "$geodata_rules" "$token_count_after" "$active_after" "$cleanup_candidates"
