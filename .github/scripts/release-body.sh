#!/usr/bin/env bash
# Build the GitHub Release body for the tag that triggered the release
# workflow. Prints Markdown to stdout.
#
# Sources:
#   * the matching "## [vX.Y.Z]" section from CHANGELOG.md (fallback: the first
#     unreleased section, then a generic note),
#   * install commands for Windows and Linux/macOS,
#   * the GitHub mirror acceleration table shared with the installers.
set -Eeuo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

VERSION="${VERSION:-${GITHUB_REF_NAME:-}}"
VERSION="${VERSION#v}"
[ -n "$VERSION" ] || { echo "VERSION or GITHUB_REF_NAME is required" >&2; exit 1; }
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'VERSION must be a semantic release version' >&2; exit 1; }

REPO="neko233-com/vpn233-all"
if [ -n "${GITHUB_REPOSITORY:-}" ]; then
  REPO="$GITHUB_REPOSITORY"
fi
[[ "$REPO" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || { echo 'Invalid GitHub repository' >&2; exit 1; }

extract_changelog() {
  local version="$1"
  [ -f CHANGELOG.md ] || return 0
  awk -v version="$version" '
    BEGIN { found = 0 }
    /^## \[v?[0-9]+\.[0-9]+\.[0-9]+\]/ {
      line = $0
      match(line, /v?[0-9]+\.[0-9]+\.[0-9]+/)
      heading_version = substr(line, RSTART, RLENGTH)
      sub(/^v/, "", heading_version)
      if (found == 1) { exit }
      if (heading_version == version) { found = 1; next }
    }
    found == 1 { print }
  ' CHANGELOG.md | sed -e '${/^$/d}' -e '/^## \[Unreleased\]/d'
}

BASE_URL="https://github.com/${REPO}/releases/download/v${VERSION}"

cat <<EOF
## What's changed

EOF
changelog_section="$(extract_changelog "$VERSION" || true)"
if [ -n "$changelog_section" ]; then
  printf '%s\n\n' "$changelog_section"
else
  printf 'See the commit history and CHANGELOG.md for the changes in v%s.\n\n' "$VERSION"
fi

cat <<EOF
## Download

Every platform ships the same bundle twice: \`.tar.gz\` for Linux/macOS and
\`.zip\` for Windows. Verify the download against \`SHA256SUMS\` before running
anything.

| File | Platform |
| --- | --- |
| \`vpn233-rust-linux-x86_64.tar.gz\` / \`.zip\` | Linux x86_64 |
| \`vpn233-rust-windows-x86_64.zip\` / \`.tar.gz\` | Windows x86_64 |
| \`vpn233-rust-macos-x86_64.tar.gz\` / \`.zip\` | macOS Intel |
| \`vpn233-rust-macos-aarch64.tar.gz\` / \`.zip\` | macOS Apple Silicon |

## Install

Windows (Windows PowerShell 5.1 or later):

\`\`\`powershell
\$setup = Join-Path ([IO.Path]::GetTempPath()) ('vpn233-install-' + [guid]::NewGuid().ToString('N') + '.ps1')
\$previousTls = [Net.ServicePointManager]::SecurityProtocol
try {
  [Net.ServicePointManager]::SecurityProtocol = \$previousTls -bor [Net.SecurityProtocolType]::Tls12
  try { Invoke-WebRequest -UseBasicParsing 'https://ghproxy.net/https://raw.githubusercontent.com/${REPO}/v${VERSION}/install.ps1' -OutFile \$setup -TimeoutSec 30 -ErrorAction Stop }
  catch { Invoke-WebRequest -UseBasicParsing 'https://raw.githubusercontent.com/${REPO}/v${VERSION}/install.ps1' -OutFile \$setup -TimeoutSec 30 -ErrorAction Stop }
  powershell -NoProfile -ExecutionPolicy Bypass -File \$setup -Release -Version '${VERSION}' -Mirror auto
  if (\$LASTEXITCODE -ne 0) { throw 'VPN233 installation failed.' }
}
finally {
  if (Test-Path -LiteralPath \$setup) { Remove-Item -LiteralPath \$setup -Force }
  [Net.ServicePointManager]::SecurityProtocol = \$previousTls
}
\`\`\`

Linux / macOS:

\`\`\`bash
(
  set -eu
  setup="\$(mktemp "\${TMPDIR:-/tmp}/vpn233-install.XXXXXXXX")"
  trap 'rm -f -- "\$setup"' EXIT
  curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fL --connect-timeout 5 --max-time 30 'https://ghproxy.net/https://raw.githubusercontent.com/${REPO}/v${VERSION}/install.sh' -o "\$setup" ||
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fL --connect-timeout 5 --max-time 30 'https://raw.githubusercontent.com/${REPO}/v${VERSION}/install.sh' -o "\$setup"
  bash "\$setup" --release --version '${VERSION}' --mirror auto
)
\`\`\`

Direct links (no mirror):

- \`${BASE_URL}/vpn233-rust-linux-x86_64.tar.gz\`
- \`${BASE_URL}/vpn233-rust-windows-x86_64.zip\`
- \`${BASE_URL}/SHA256SUMS\`

## GitHub mirror acceleration

If raw.githubusercontent.com or github.com is slow or blocked, prefix a
supported mirror to any GitHub URL. The installers try the mirrors in order and
fall back to a direct connection:

| Mirror prefix | Notes |
| --- | --- |
| \`https://ghproxy.net/\` | Default; availability depends on the network |
| \`https://gh.llkk.cc/\` | Fallback |
| \`https://ghfast.top/\` | Fallback |
| \`https://gh-proxy.com/\` | Fallback |
| (direct) | Last resort |

Set \`VPN233_GH_MIRROR\` to pin one prefix, \`VPN233_GH_MIRRORS\` (\`|\` separated)
to override the list, or \`VPN233_NO_MIRROR=1\` to force direct downloads. See
\`docs/mirrors.md\` for the full matrix and troubleshooting.

## Checksums

\`SHA256SUMS\` in this release covers every archive:

\`\`\`bash
sha256sum -c SHA256SUMS
# PowerShell: Get-FileHash -Algorithm SHA256 .\\vpn233-rust-*.zip
\`\`\`

Every release artifact is built by GitHub Actions from the tagged commit after
the full CI pipeline (Rust tests, admin web build, shell/PowerShell script tests,
secret scan, dependency audit and the end-to-end suite) has passed.
EOF
