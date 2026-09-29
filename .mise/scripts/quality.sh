#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

mode="${1:-all}"

step() {
  local name="$1"
  shift
  printf 'processing: %s\n' "$name"
  SECONDS=0
  "$@"
  printf 'processing: %s - finished in: (%ds)\n' "$name" "$SECONDS"
}

kiss_check() {
  local out filtered
  if ! out=$(kiss check . 2>&1); then
    filtered=$(printf '%s\n' "$out" | awk '!/^VIOLATION:(duplication|orphan_module):/')
    if printf '%s\n' "$filtered" | grep -q '^VIOLATION:'; then
      printf '%s\n' "$out" >&2
      return 1
    fi
  fi
}

check_warnings() {
  CARGO_TARGET_DIR="$root/target/quality" RUSTFLAGS="-D warnings" cargo check --workspace --quiet
}

clippy_warnings() {
  CARGO_TARGET_DIR="$root/target/quality" cargo clippy --all-targets --all-features --quiet -- -D warnings
}

case "$mode" in
  all)
    step 'cargo fmt' cargo fmt --check
    step kiss kiss_check
    step duplication bash .mise/scripts/duplication.sh
    step check check_warnings
    step clippy clippy_warnings
    step nextest cargo nextest run --workspace
    ;;
  kiss)
    kiss_check
    ;;
  *)
    echo "usage: quality.sh [all|kiss]" >&2
    exit 2
    ;;
esac
