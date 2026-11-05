#!/usr/bin/env bash
# Confirms optional media crates stay out of protocol and browser graphs.
# Linux GNU/musl and Windows are reported as not executed on other hosts.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

absent() {
  local package="$1"
  shift
  if cargo tree -i "$package" "$@" >/dev/null 2>&1; then
    printf 'expected %s to be absent (%s)\n' "$package" "$*" >&2
    exit 1
  fi
  printf 'absent: %s %s\n' "$package" "$*"
}

present() {
  local package="$1"
  shift
  cargo tree -i "$package" "$@" >/dev/null
  printf 'present: %s %s\n' "$package" "$*"
}

printf 'host: %s\n' "$(uname -sm)"
printf 'rustc: %s\n' "$(rustc --version)"
absent openh264
absent nokhwa
absent openh264 --no-default-features
absent nokhwa --no-default-features
present openh264 --features openh264
present nokhwa --features nokhwa
present yuv --features yuv

if rustup target list --installed | grep -q '^wasm32-unknown-unknown$'; then
  absent openh264 --target wasm32-unknown-unknown
  absent nokhwa --target wasm32-unknown-unknown
else
  printf 'wasm32-unknown-unknown is not installed; browser isolation stays on the web-ui feature-off check\n'
fi

printf 'not executed here: x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, x86_64-unknown-linux-musl, aarch64-unknown-linux-musl, x86_64-pc-windows-msvc\n'
printf 'apple smoke: cargo test --test media_validate --features yuv,openh264\n'
cargo test --test media_validate --features yuv,openh264 -- --test-threads=8
