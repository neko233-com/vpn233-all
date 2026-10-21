#!/usr/bin/env bash
# Documentation gate used by the CI `docs` job and by verify-all.
#
# Checks every Markdown file that ends up on GitHub (README files and docs/):
#   1. relative links point at files that exist in the repository,
#   2. fenced code blocks are balanced and consistently typed,
#   3. no unresolved TODO/FIXME/TBD placeholders are left behind,
#   4. no credential-like strings or non-example IPv4 addresses are published,
#   5. Jekyll front matter exists for docs pages and docs/_config.yml is valid,
#   6. the GitHub mirror list in the docs matches scripts/lib/mirrors.sh, and
#      the public repository URL is consistent.
#
# Exit code 0 means the documentation is publishable.
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

errors=0
warnings=0

fail() { printf 'ERROR %s\n' "$*" >&2; errors=$((errors + 1)); }
warn() { printf 'WARN  %s\n' "$*" >&2; warnings=$((warnings + 1)); }
info() { printf 'ok    %s\n' "$*"; }

# --- collect files -------------------------------------------------------
files=()
while IFS= read -r file; do
  files+=("$file")
done < <(find docs -type f -name '*.md' 2>/dev/null | LC_ALL=C sort)
for readme in README.md README.zh-CN.md; do
  [ -f "$readme" ] && files+=("$readme")
done
if [ "${#files[@]}" -eq 0 ]; then
  fail "no Markdown files found (docs/ or README.md missing)"
  exit 1
fi

# --- 1. relative links ---------------------------------------------------
check_links() {
  local file="$1" dir target clean resolved
  dir="$(dirname "$file")"
  while IFS= read -r target; do
    [ -n "$target" ] || continue
    case "$target" in
      http://*|https://*|mailto:*|tel:*|'#'*) continue ;;
    esac
    # Strip a fragment and query string, keep the file path.
    clean="${target%%#*}"
    clean="${clean%%\?*}"
    [ -n "$clean" ] || continue
    case "$clean" in
      /*) resolved="$ROOT_DIR$clean" ;;
      *) resolved="$dir/$clean" ;;
    esac
    if [ ! -e "$resolved" ]; then
      fail "$file: broken relative link (target redacted)"
    fi
  done < <(grep -oE '\]\([^()[:space:]]+\)' "$file" 2>/dev/null | sed -e 's/^](//' -e 's/)$//' || true)
}

# --- 2. code fences ------------------------------------------------------
check_fences() {
  local file="$1" fences mixed
  fences="$(grep -cE '^[[:space:]]*(```|~~~)' "$file" || true)"
  if [ $((fences % 2)) -ne 0 ]; then
    fail "$file: unbalanced code fences (${fences} fence markers)"
  fi
  mixed="$(awk '
    function fence_kind(line) {
      if (match(line, /^[[:space:]]*```/)) return "backtick"
      if (match(line, /^[[:space:]]*~~~/)) return "tilde"
      return ""
    }
    {
      kind = fence_kind($0)
      if (kind != "") {
        if (open_kind == "") {
          open_kind = kind
        } else {
          if (kind != open_kind) { print NR; exit }
          open_kind = ""
        }
      }
    }
  ' "$file")"
  [ -z "$mixed" ] || fail "$file: mixed code fence types near line $mixed"
}

# --- 3. placeholders -----------------------------------------------------
check_placeholders() {
  local file="$1" match
  match="$(grep -nE '\b(TODO|FIXME|TBD)\b' "$file" | head -n 5 || true)"
  if [ -n "$match" ]; then
    fail "$file: unresolved TODO/FIXME/TBD placeholder(s):"
  fi
}

# --- 4. secrets and real IPs --------------------------------------------
is_example_ip() {
  local ip="$1"
  case "$ip" in
    127.*|10.*|192.168.*|0.0.0.0|1.1.1.1|8.8.8.8|255.255.255.0) return 0 ;;
    192.0.2.*|198.51.100.*|203.0.113.*) return 0 ;;
  esac
  printf '%s' "$ip" | grep -qE '^172\.(1[6-9]|2[0-9]|3[01])\.' && return 0
  return 1
}

check_secrets() {
  local file="$1" pattern ip hits
  for pattern in \
    'ghp_[A-Za-z0-9]{20,}' \
    'gho_[A-Za-z0-9]{20,}' \
    'ghu_[A-Za-z0-9]{20,}' \
    'ghs_[A-Za-z0-9]{20,}' \
    'github_pat_[A-Za-z0-9_]{20,}' \
    'AKIA[0-9A-Z]{16}' \
    'xox[baprs]-[A-Za-z0-9-]{10,}' \
    '-----BEGIN [A-Z ]*PRIVATE KEY-----' \
    'ssh-rsa AAAA[0-9A-Za-z+/]{20,}'; do
    if grep -qE -- "$pattern" "$file"; then
      fail "$file: matches forbidden secret pattern '$pattern'"
    fi
  done
  hits=0
  while IFS= read -r ip; do
    [ -n "$ip" ] || continue
    if ! is_example_ip "$ip"; then
      fail "$file: contains non-example IPv4 address (value redacted)"
      hits=$((hits + 1))
      [ "$hits" -ge 5 ] && break
    fi
  done < <(grep -oE '\b([0-9]{1,3}\.){3}[0-9]{1,3}\b' "$file" | LC_ALL=C sort -u || true)
}

# --- 5. Jekyll front matter ---------------------------------------------
check_front_matter() {
  local file="$1" first
  first="$(head -n 1 "$file")"
  if [ "$first" != "---" ]; then
    fail "$file: missing Jekyll front matter (first line must be '---')"
    return
  fi
  if ! head -n 10 "$file" | grep -qE '^(title|layout):'; then
    fail "$file: front matter has neither title nor layout"
  fi
}

# --- run the per-file checks --------------------------------------------
for file in "${files[@]}"; do
  check_links "$file"
  check_fences "$file"
  check_placeholders "$file"
  check_secrets "$file"
done
info "checked ${#files[@]} Markdown file(s): links, fences, placeholders, secrets"

# --- docs site metadata --------------------------------------------------
if [ ! -f docs/_config.yml ]; then
  fail "docs/_config.yml is missing (GitHub Pages needs it)"
else
  grep -qE '^theme:' docs/_config.yml || fail "docs/_config.yml: no theme configured"
  grep -qE '^title:' docs/_config.yml || fail "docs/_config.yml: no title configured"
fi
doc_pages=()
while IFS= read -r page; do
  doc_pages+=("$page")
done < <(find docs -maxdepth 2 -type f -name '*.md' ! -name '_*' | LC_ALL=C sort)
for page in "${doc_pages[@]}"; do
  check_front_matter "$page"
done
info "checked Jekyll front matter for ${#doc_pages[@]} doc page(s)"

# --- 6. mirror contract + repository URL --------------------------------
REPO_URL="neko233-com/vpn233-all"
for entry in README.md README.zh-CN.md; do
  [ -f "$entry" ] || continue
  grep -q "$REPO_URL" "$entry" || fail "$entry: does not mention the repository $REPO_URL"
done

if [ -f scripts/lib/mirrors.sh ]; then
  script_mirrors=()
  while IFS= read -r mirror; do
    script_mirrors+=("$mirror")
  done < <(grep -oE 'https://(ghproxy\.net|gh\.llkk\.cc|ghfast\.top|gh-proxy\.com|cdn\.jsdelivr\.net)/' scripts/lib/mirrors.sh |
    LC_ALL=C sort -u)
  if [ "${#script_mirrors[@]}" -eq 0 ]; then
    fail "scripts/lib/mirrors.sh: no known mirror prefixes found (contract drift)"
  fi
  for mirror in "${script_mirrors[@]}"; do
    if ! grep -qF "$mirror" docs/mirrors.md; then
      fail "docs/mirrors.md: missing mirror prefix '$mirror' used by scripts/lib/mirrors.sh"
    fi
  done
  info "mirror list matches scripts/lib/mirrors.sh (${#script_mirrors[@]} prefixes)"
else
  warn "scripts/lib/mirrors.sh not found; mirror consistency check skipped"
fi

if [ -f docs/mirrors.md ]; then
  grep -q 'https://ghproxy.net/' docs/mirrors.md ||
    fail "docs/mirrors.md: primary mirror https://ghproxy.net/ is not documented"
else
  fail "docs/mirrors.md is missing"
fi

# --- summary -------------------------------------------------------------
echo
if [ "$errors" -ne 0 ]; then
  printf 'documentation check FAILED: %s error(s), %s warning(s)\n' "$errors" "$warnings" >&2
  exit 1
fi
printf 'documentation check PASSED: %s file(s), %s warning(s)\n' "${#files[@]}" "$warnings"
