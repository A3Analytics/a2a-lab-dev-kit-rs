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

wiki_cmd=wiki-publish
export GIT_TERMINAL_PROMPT=0

wiki_check

clone="$root/target/wiki-remote"
branch=$(wiki_fetch_clone "$clone")

git -C "$clone" reset --hard
git -C "$clone" clean -fd
git -C "$clone" checkout -B "$branch" "origin/$branch"

for existing in "$clone"/*.md; do
  [ -e "$existing" ] || continue
  base=$(basename "$existing")
  if [ ! -f "$WIKI_STAGE/$base" ]; then
    rm -f -- "$existing"
  fi
done
cp "$WIKI_STAGE"/*.md "$clone"/

git -C "$clone" add -A -- .
if git -C "$clone" diff --cached --quiet; then
  printf 'wiki-publish: remote Wiki already matches target/wiki-stage\n'
  exit 0
fi

if ! git -C "$clone" commit -m "$(cat <<'EOF'
Publish the Wiki from public backlog docs.

EOF
)"; then
  printf 'wiki-publish: could not commit the Wiki checkout. git must have a user name and email configured.\n' >&2
  exit 1
fi

if ! err=$(git -C "$clone" push origin "$branch" 2>&1); then
  printf '%s\n' "$err" >&2
  case "$err" in
    *non-fast-forward* | *\[rejected\]* | *failed\ to\ push*)
      printf 'wiki-publish: push was rejected. Refusing to force-push.\n' >&2
      exit 1
      ;;
  esac
  wiki_access_fail
fi

printf 'wiki-publish: pushed %s\n' "$branch"
