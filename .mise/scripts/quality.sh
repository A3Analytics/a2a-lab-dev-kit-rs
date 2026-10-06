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
    step 'sila2-matrix' bash .mise/scripts/sila2-matrix.sh
    step 'wiki-check' bash .mise/scripts/wiki-check.sh
    step kiss kiss_check
    step duplication bash .mise/scripts/duplication.sh
    step check check_warnings
    step clippy clippy_warnings
    step nextest cargo nextest run --workspace --all-features
    step tck bash .mise/scripts/a2a-tck.sh
    step sila2-interop bash .mise/scripts/sila2-interop.sh
    step sila2-communication bash .mise/scripts/sila2-communication.sh
    step sila2-assess bash .mise/scripts/sila2-assess.sh
    ;;
  kiss)
    kiss_check
    ;;
  *)
    echo "usage: quality.sh [all|kiss]" >&2
    exit 2
    ;;
esac
