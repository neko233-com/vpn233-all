#!/usr/bin/env bash
# Default only prepares the shared version metadata. --publish requires a clean
# main commit already verified by CI, runs full local verification, then pushes
# a new tag without force. The tag workflow validates archives before publishing.
set -Eeuo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
version=''; publish=0; remote=origin
while [ "$#" -gt 0 ]; do
  case "$1" in
    --version) version="${2:?version required}"; shift ;;
    --remote) remote="${2:?remote required}"; shift ;;
    --publish) publish=1 ;;
    -h|--help) echo 'release.sh [--version MAJOR.MINOR.PATCH] [--publish] [--remote origin]'; exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done
if [ "$publish" -eq 0 ]; then
  [ -n "$version" ] || { echo '--version is required to prepare a release' >&2; exit 2; }
  python scripts/release-version.py --prepare "$version"
  exit 0
fi
python scripts/release-version.py
current="$(tr -d '\r\n' < version.txt)"
[ -z "$version" ] || [ "$version" = "$current" ] || { echo 'Prepare and commit the requested version first' >&2; exit 1; }
[ "$(git branch --show-current)" = main ] || { echo 'Releases must run on main' >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo 'Review and commit all candidate changes before publishing' >&2; exit 1; }
python scripts/check-privacy.py --history
bash scripts/verify-all.sh
head_sha="$(git rev-parse HEAD)"
remote_sha="$(git ls-remote --exit-code "$remote" refs/heads/main | cut -f1)"
[ "$head_sha" = "$remote_sha" ] || { echo 'Push the verified main commit before publishing a tag' >&2; exit 1; }
GH_PROMPT_DISABLED=1 gh run list --workflow ci.yml --branch main --commit "$head_sha" --limit 1 --json status,conclusion |
  python -c 'import json,sys; runs=json.load(sys.stdin); sys.exit(0 if runs and runs[0]["status"]=="completed" and runs[0]["conclusion"]=="success" else "The main commit needs a successful completed CI run")'
tag="v$current"
if git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null || [ -n "$(git ls-remote "$remote" "refs/tags/$tag")" ]; then
  echo 'Release tag already exists; refusing to replace it' >&2; exit 1
fi
git tag -a "$tag" -m "Release $tag"
git push "$remote" "refs/tags/$tag:refs/tags/$tag"
printf 'Tag %s pushed; the release workflow will verify every archive before publishing.\n' "$tag"
