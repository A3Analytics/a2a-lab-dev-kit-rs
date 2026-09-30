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
wiki_check
bash "$root/.mise/scripts/wiki-mermaid-fixture.sh"
