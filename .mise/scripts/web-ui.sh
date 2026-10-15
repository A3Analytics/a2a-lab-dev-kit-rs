#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

mode="${1:-build}"

web_ui_packages() {
  cargo tree --prefix none --format '{p}' --edges normal,build "$@"
}

reject_web_ui_deps() {
  local tree
  tree="$(web_ui_packages "$@")"
  if printf '%s\n' "$tree" | grep -E '^(wasm-bindgen|wasm-bindgen-cli-support|web-sys) v'; then
    echo "web UI dependencies are present without the web-ui feature" >&2
    exit 1
  fi
}

probe_absent_api() {
  local default_features="$1"
  local dir
  dir="$(mktemp -d)"
  cat >"$dir/Cargo.toml" <<EOF
[package]
name = "web-ui-api-probe"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
a2a-lab-dev-kit = { path = "$root", default-features = $default_features }
EOF
  mkdir -p "$dir/src"
  cat >"$dir/src/main.rs" <<'EOF'
fn main() {
    let _opt_in = a2a_lab_dev_kit::A2aServer::with_web_ui;
}
EOF
  local log status
  set +e
  log="$(cargo check --manifest-path "$dir/Cargo.toml" --offline 2>&1)"
  status=$?
  set -e
  rm -rf "$dir"
  if [ "$status" -eq 0 ]; then
    echo "with_web_ui is available without the web-ui feature" >&2
    exit 1
  fi
  if ! printf '%s\n' "$log" | grep -q 'with_web_ui'; then
    printf '%s\n' "$log" >&2
    echo "feature-off probe failed before it could see with_web_ui" >&2
    exit 1
  fi
}

feature_off() {
  cargo check --no-default-features --locked --quiet
  cargo check --locked --quiet
  reject_web_ui_deps --no-default-features
  reject_web_ui_deps
  probe_absent_api false
  probe_absent_api true
}

build() {
  cargo fmt --manifest-path web-ui/Cargo.toml --check
  cargo clippy --manifest-path web-ui/Cargo.toml --all-targets --locked -- -D warnings
  cargo clippy --manifest-path web-ui/Cargo.toml --target wasm32-unknown-unknown --locked -- -D warnings
  cargo test --manifest-path web-ui/Cargo.toml --locked --quiet
  cargo check --all-features --locked
}

freshness() {
  export CARGO_TARGET_DIR="$root/target/quality"
  cargo check --all-features --locked
  local stamp="$root/target/web-ui-wasm/source.fingerprint"
  if [ ! -s "$stamp" ]; then
    echo "web UI source fingerprint was not written" >&2
    exit 1
  fi
  cp "$stamp" "$stamp.bak"
  if ! WEB_UI_CHECK_ONLY=1 WEB_UI_FRESHNESS_NONCE=fresh cargo check --all-features --locked; then
    echo "fresh web UI assets were rejected" >&2
    mv "$stamp.bak" "$stamp"
    exit 1
  fi
  printf 'stale' >"$stamp"
  if WEB_UI_CHECK_ONLY=1 WEB_UI_FRESHNESS_NONCE=stale cargo check --all-features --locked; then
    echo "stale generated web UI assets were accepted" >&2
    mv "$stamp.bak" "$stamp"
    exit 1
  fi
  mv "$stamp.bak" "$stamp"
  WEB_UI_FRESHNESS_NONCE=repair cargo check --all-features --locked
}

serve() {
  cargo nextest run --all-features --test web_ui
}

watch() {
  export WEB_UI_BIND="${WEB_UI_BIND:-127.0.0.1:3847}"
  exec watchexec \
    --watch web-ui \
    --watch examples/web_ui.rs \
    --watch src \
    --watch build.rs \
    --watch Cargo.toml \
    --ignore 'target/**' \
    --ignore 'web-ui/target/**' \
    --exts rs,css,js,html,toml \
    --debounce 500ms \
    --restart \
    --stop-signal SIGTERM \
    --stop-timeout 15s \
    -- cargo run --example web_ui --features web-ui --locked
}

case "$mode" in
  build|feature-off|freshness|serve|watch)
    ;;
  *)
    echo "usage: web-ui.sh [build|feature-off|freshness|serve|watch]" >&2
    exit 2
    ;;
esac

case "$mode" in
  feature-off) feature_off ;;
  build) build ;;
  freshness) freshness ;;
  serve) serve ;;
  watch) watch ;;
esac
