#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# Pinned a2aproject/a2a-tck commit (main as of 2026-09-01).
tck_rev="263b9cfaf16a554bdfb166a7ba5b67716e946349"
tck_dir="${A2A_TCK_DIR:-$root/target/a2a-tck}"
image="a2a-tck:${tck_rev}"

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required to run the A2A TCK image" >&2
  exit 1
fi

if [ ! -d "$tck_dir/.git" ]; then
  git clone https://github.com/a2aproject/a2a-tck.git "$tck_dir"
fi
git -C "$tck_dir" fetch origin
git -C "$tck_dir" checkout --detach "$tck_rev"
printf '.git\n.venv\nreports\n' >"$tck_dir/.dockerignore"
docker build -f "$root/.mise/a2a-tck.Dockerfile" -t "$image" "$tck_dir"

# Linux containers share the host network, so 127.0.0.1 reaches the server.
# Elsewhere the server advertises host.docker.internal and listens on 0.0.0.0.
docker_args=(--rm)
if [ "$(uname -s)" = "Linux" ]; then
  docker_args+=(--network host)
else
  # Docker Desktop already resolves host.docker.internal to the host.
  export A2A_TCK_ADVERTISE=host.docker.internal
fi

cargo build --example a2a_interface --quiet

server_log="$root/target/a2a-tck-sut.log"
mkdir -p "$root/target"
: >"$server_log"
"$root/target/debug/examples/a2a_interface" >"$server_log" 2>&1 &
server_pid=$!
cleanup() {
  kill "$server_pid" >/dev/null 2>&1 || true
  wait "$server_pid" >/dev/null 2>&1 || true
}
trap cleanup EXIT

sut_url=""
docker_sut=""
for _ in $(seq 1 50); do
  sut_url="$(sed -n 's/^A2A_TCK_SUT=//p' "$server_log" | tail -n 1)"
  docker_sut="$(sed -n 's/^A2A_TCK_DOCKER_SUT=//p' "$server_log" | tail -n 1)"
  if [ -n "$sut_url" ]; then
    break
  fi
  sleep 0.2
done
if [ -z "$sut_url" ]; then
  echo "SUT did not print A2A_TCK_SUT" >&2
  cat "$server_log" >&2 || true
  exit 1
fi
if [ -z "$docker_sut" ]; then
  docker_sut="$sut_url"
fi

ready=0
for _ in $(seq 1 50); do
  if curl -fsS "$sut_url/.well-known/agent-card.json" >/dev/null; then
    ready=1
    break
  fi
  sleep 0.2
done
if [ "$ready" -ne 1 ]; then
  echo "SUT did not become ready: $sut_url/.well-known/agent-card.json" >&2
  cat "$server_log" >&2 || true
  exit 1
fi

mkdir -p "$root/target/a2a-tck-reports"
# HTTP_JSON-SVC-001 still asserts application/json; A2A 1.0 and the official
# Rust SDK emit application/a2a+json. Keep the schema half of that requirement.
# CORE-SEND-003 describes ContentTypeNotSupportedError but its runner expects
# a successful task. This server follows the spec.
docker run "${docker_args[@]}" \
  -v "$root/target/a2a-tck-reports:/tck/reports" \
  "$image" \
  --sut-host "$docker_sut" \
  --transport http_json,jsonrpc,grpc \
  -- \
  -k "not test_response_content_type and not CORE-SEND-003"
