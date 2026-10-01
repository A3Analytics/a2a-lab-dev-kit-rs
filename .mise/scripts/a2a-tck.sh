#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# Pinned a2aproject/a2a-tck commit (main as of 2026-09-01).
tck_rev="263b9cfaf16a554bdfb166a7ba5b67716e946349"
tck_dir="${A2A_TCK_DIR:-$root/target/a2a-tck}"

if [ ! -d "$tck_dir/.git" ]; then
  git clone https://github.com/a2aproject/a2a-tck.git "$tck_dir"
fi
git -C "$tck_dir" fetch origin
git -C "$tck_dir" checkout --detach "$tck_rev"

cargo build --example a2a_tck --quiet

server_log="$root/target/a2a-tck-sut.log"
mkdir -p "$root/target"
: >"$server_log"
"$root/target/debug/examples/a2a_tck" >"$server_log" 2>&1 &
server_pid=$!
cleanup() {
  kill "$server_pid" >/dev/null 2>&1 || true
  wait "$server_pid" >/dev/null 2>&1 || true
}
trap cleanup EXIT

sut_url=""
for _ in $(seq 1 50); do
  if sut_url=$(sed -n 's/^A2A_TCK_SUT=//p' "$server_log" | tail -n 1) && [ -n "$sut_url" ]; then
    break
  fi
  sleep 0.2
done
if [ -z "$sut_url" ]; then
  echo "SUT did not print A2A_TCK_SUT" >&2
  cat "$server_log" >&2 || true
  exit 1
fi

python - "$sut_url/.well-known/agent-card.json" <<'PY'
import sys
import time
import urllib.request

url = sys.argv[1]
for _ in range(50):
    try:
        urllib.request.urlopen(url, timeout=1)
        raise SystemExit(0)
    except Exception:
        time.sleep(0.2)
raise SystemExit("SUT did not become ready: " + url)
PY

(
  cd "$tck_dir"
  uv sync
  # HTTP_JSON-SVC-001 still asserts application/json; A2A 1.0 and the official
  # Rust SDK emit application/a2a+json. Keep the schema half of that requirement.
  uv run python run_tck.py --sut-host "$sut_url" --transport http_json --level must -- \
    -k "not test_response_content_type"
)
