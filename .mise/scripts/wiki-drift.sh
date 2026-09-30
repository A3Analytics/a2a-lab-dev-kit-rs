#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

wiki_tmp=""
cleanup() {
  if [ -n "${wiki_tmp:-}" ]; then
    rm -rf "$wiki_tmp"
  fi
}
trap cleanup EXIT

# shellcheck disable=SC1091
source "$root/.mise/scripts/wiki-lib.sh"

wiki_cmd=wiki-drift
export GIT_TERMINAL_PROMPT=0

wiki_check

clone="$root/target/wiki-drift-cache"
tree="$root/target/wiki-drift-remote"
branch=$(wiki_fetch_clone "$clone")

rm -rf "$tree"
mkdir -p "$tree"
git -C "$clone" archive --format=tar "origin/$branch" | tar -x -C "$tree"

diff_out="$root/target/wiki-drift.diff"
set +e
diff -ru "$tree" "$WIKI_STAGE" >"$diff_out"
status=$?
set -e
if [ "$status" -eq 0 ]; then
  printf 'wiki-drift: target/wiki-stage matches the remote Wiki\n'
  exit 0
fi
if [ "$status" -eq 1 ]; then
  cat "$diff_out" >&2
  printf 'wiki-drift: target/wiki-stage differs from the remote Wiki\n' >&2
  exit 1
fi
printf 'wiki-drift: diff failed\n' >&2
exit "$status"
