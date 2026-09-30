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
wiki_disable_interactive_auth

wiki_check

clone="$root/target/wiki-remote"
branch=$(wiki_fetch_clone "$clone")

if [ "${GITHUB_ACTIONS:-}" = true ]; then
  wiki_git -C "$clone" config user.name "github-actions[bot]"
  wiki_git -C "$clone" config user.email "41898282+github-actions[bot]@users.noreply.github.com"
fi

wiki_git -C "$clone" reset --hard
wiki_git -C "$clone" clean -fd
wiki_git -C "$clone" checkout -B "$branch" "origin/$branch"

for existing in "$clone"/*.md; do
  [ -e "$existing" ] || continue
  base=$(basename "$existing")
  if [ ! -f "$WIKI_STAGE/$base" ]; then
    rm -f -- "$existing"
  fi
done
cp "$WIKI_STAGE"/*.md "$clone"/

wiki_git -C "$clone" add -A -- .
if wiki_git -C "$clone" diff --cached --quiet; then
  printf 'wiki-publish: remote Wiki already matches target/wiki-stage\n'
  exit 0
fi

if ! wiki_git -C "$clone" commit -m "$(cat <<'EOF'
Publish the Wiki from public backlog docs.

EOF
)"; then
  printf 'wiki-publish: could not commit the Wiki checkout. git must have a user name and email configured.\n' >&2
  exit 1
fi

if ! err=$(wiki_git -C "$clone" push origin "$branch" 2>&1); then
  printf '%s\n' "$(wiki_redact "$err")" >&2
  case "$err" in
    *non-fast-forward* | *\[rejected\]* | *failed\ to\ push*)
      printf 'wiki-publish: push was rejected. Refusing to force-push.\n' >&2
      exit 1
      ;;
  esac
  wiki_access_fail
fi

printf 'wiki-publish: pushed %s\n' "$branch"
