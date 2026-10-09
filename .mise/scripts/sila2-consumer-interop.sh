#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$root/target/sila2-consumer-interop-reports"
source_dir="$root/target/sila-csharp"
commit="2625cce6541c501cb951f2eea95d490a2efd12c0"
server_name="a2a-lab-sila-consumer-csharp"
server_uuid="a82121b1-22de-4b81-a450-86fa2e5344ee"
port=50052
case "$(uname -m)" in
  arm64 | aarch64) targetarch=arm64 ;;
  x86_64) targetarch=amd64 ;;
  *)
    echo "unsupported architecture $(uname -m)" >&2
    exit 1
    ;;
esac
mkdir -p "$out/csharp"
cd "$root"

if [ ! -d "$source_dir/.git" ]; then
  git clone --depth 1 --branch 'v.10.3.2' https://gitlab.com/SiLA2/sila_csharp.git "$source_dir"
fi
if [ "$(git -C "$source_dir" rev-parse HEAD)" != "$commit" ]; then
  echo "sila_csharp checkout is not $commit" >&2
  exit 1
fi
git -C "$source_dir" submodule update --init --depth 1

openssl req -x509 -newkey rsa:2048 -keyout "$out/csharp/ca.key" -out "$out/csharp/ca.crt" -days 3 -nodes -subj "/CN=SiLA2 CA" >/dev/null 2>&1
openssl req -newkey rsa:2048 -keyout "$out/csharp/server.key" -out "$out/csharp/server.csr" -nodes -subj "/CN=SiLA2" >/dev/null 2>&1
openssl x509 -req -in "$out/csharp/server.csr" -CA "$out/csharp/ca.crt" -CAkey "$out/csharp/ca.key" -CAcreateserial -out "$out/csharp/server.crt" -days 3 \
  -extfile <(printf 'subjectAltName=DNS:SiLA2,IP:127.0.0.1\n') >/dev/null 2>&1
chmod a+r "$out/csharp/server.key" "$out/csharp/server.crt" "$out/csharp/ca.crt" "$out/csharp/ca.key"

docker build \
  --build-arg TARGETARCH="$targetarch" \
  -f "$source_dir/src/Tests/SiLA2.IntegrationTests.ServerApp/Dockerfile" \
  -t a2a-lab-sila-consumer-csharp \
  "$source_dir"
docker build -f .mise/sila-consumer-client.Dockerfile -t a2a-lab-sila-consumer-client "$root"

docker rm -f "$server_name" >/dev/null 2>&1 || true
docker run -d --name "$server_name" --network host \
  -v "$out/csharp:/data" \
  -e SILA_HOST=0.0.0.0 \
  -e SILA_PORT="$port" \
  -e SILA_SERVER_UUID="$server_uuid" \
  -e SILA_SERVER_VERSION=1.1 \
  -e SILA_TLS=true \
  -e SILA_DATA_DIR=/data \
  -e ServerConfig__DiscoveryServiceName=_sila._tcp \
  a2a-lab-sila-consumer-csharp

ready=0
for _ in $(seq 1 90); do
  if docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{end}}' "$server_name" | grep -q healthy; then
    ready=1
    break
  fi
  sleep 1
done
if [ "$ready" -ne 1 ]; then
  docker logs "$server_name" >&2 || true
  echo "C# SiLA server did not announce" >&2
  exit 1
fi

docker rm -f a2a-lab-sila-consumer-caddy >/dev/null 2>&1 || true
cleanup() {
  docker rm -f a2a-lab-sila-consumer-caddy a2a-lab-sila-consumer-initiated "$server_name" >/dev/null 2>&1 || true
}
trap cleanup EXIT
# The client and official servers share the Docker host network. Caddy has to
# listen there too; a process on the Mac cannot see that 127.0.0.1.
docker run -d --name a2a-lab-sila-consumer-caddy --network host \
  -e SILA_CA=/certs/ca.crt \
  -e SILA_CERT=/certs/server.crt \
  -e SILA_KEY=/certs/server.key \
  -v "$root/.mise/sila-consumer.Caddyfile:/etc/caddy/Caddyfile:ro" \
  -v "$out/csharp:/certs:ro" \
  caddy:2.10.2 caddy run --config /etc/caddy/Caddyfile --adapter caddyfile

initiated_name="a2a-lab-sila-consumer-initiated"
initiated_uuid="11111111-1111-1111-1111-111111111111"
docker build \
  --build-arg TARGETARCH="$targetarch" \
  -f "$root/.mise/sila-consumer-initiated.Dockerfile" \
  -t "$initiated_name" \
  "$source_dir"
docker rm -f "$initiated_name" >/dev/null 2>&1 || true
docker run -d --name "$initiated_name" --network host \
  -v "$out/csharp:/data" \
  -e SILA_HOST=0.0.0.0 \
  -e SILA_PORT=60051 \
  -e SILA_SERVER_UUID="$initiated_uuid" \
  -e SILA_SERVER_VERSION=1.1 \
  -e SILA_TLS=true \
  -e SILA_DATA_DIR=/data \
  "$initiated_name"
for _ in $(seq 1 90); do
  if docker logs "$initiated_name" 2>&1 | grep -q "Starting Server Announcement"; then
    break
  fi
  sleep 1
done

docker run --rm --network host \
  -e SILA_PEER=sila_csharp \
  -e SILA_HOST=127.0.0.1 \
  -e SILA_PORT="$port" \
  -e SILA_UUID="$server_uuid" \
  -e SILA_CA=/certs/ca.crt \
  -e SILA_REPORT=/reports/csharp.json \
  -e SILA_PLAIN_PORT=50053 \
  -e SILA_CLOUD_PORT=50055 \
  -e SILA_INITIATED_PORT=60051 \
  -e SILA_INITIATED_UUID="$initiated_uuid" \
  -v "$out/csharp/ca.crt:/certs/ca.crt:ro" \
  -v "$out:/reports" \
  a2a-lab-sila-consumer-client

server_image="$(docker image inspect --format '{{.Id}}' a2a-lab-sila-consumer-csharp)"
client_image="$(docker image inspect --format '{{.Id}}' a2a-lab-sila-consumer-client)"

python3 - "$out" "$server_image" "$client_image" <<'PY'
import json
from pathlib import Path
import sys
out = Path(sys.argv[1])
peer = json.loads((out / "csharp.json").read_text())
peer["server_image"] = sys.argv[2]
peer["client_image"] = sys.argv[3]
peer["sila_csharp"] = "v.10.3.2 2625cce6541c501cb951f2eea95d490a2efd12c0"
peer["sila_interoperability"] = "v0.10.3 ebaa33759fec74b39b413e86361922b8159b1833"
report = {
    "profile": {
        "role": "feature_consumer",
        "specification": "SiLA 2 Part A and Part B v1.1",
        "sila_base": {"tag": "v1.2", "commit": "37c400faa5036af2da6244529283d3017f34d4cd"},
        "sila_interoperability": peer["sila_interoperability"],
        "peers": [peer],
    },
    "scenarios": [
        {"id": "encrypted-direct-client-initiated", "capability": "direct-connection"},
        {"id": "unencrypted-connection", "capability": "unencrypted-connection"},
        {"id": "server-discovery", "capability": "mdns-discovery"},
        {"id": "server-initiated-connection", "capability": "server-initiated-connection"},
        {"id": "sila-service", "capability": "sila-service"},
        {"id": "basic-data-types", "capability": "basic-data-types"},
        {"id": "structure-data-types", "capability": "structured-data"},
        {"id": "list-data-types", "capability": "list-data"},
        {"id": "any-type", "capability": "any-type"},
        {"id": "unobservable-command", "capability": "unobservable-command"},
        {"id": "unobservable-property", "capability": "unobservable-property"},
        {"id": "metadata", "capability": "metadata"},
        {"id": "metadata-affected-calls", "capability": "metadata-affected-calls"},
        {"id": "error-handling", "capability": "error-classes"},
        {"id": "observable-command", "capability": "observable-command"},
        {"id": "observable-property", "capability": "observable-property-subscription"},
        {"id": "intermediate-responses", "capability": "intermediate-responses"},
        {"id": "binary-transfer", "capability": "binary-transfer"},
        {"id": "authorization", "capability": "authorization"},
        {"id": "cancellation", "capability": "cancellation"},
    ],
    "summary": {
        "pass": sum(item["status"] == "pass" for item in peer["capabilities"]),
        "fail": sum(item["status"] == "fail" for item in peer["capabilities"]),
        "unsupported": sum(item["status"] == "unsupported" for item in peer["capabilities"]),
    },
    "capabilities": peer["capabilities"],
}
(out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
cases = []
for item in peer["capabilities"]:
    body = ""
    detail = str(item["detail"]).replace("&", "&amp;").replace('"', "&quot;")
    if item["status"] == "fail":
        body = f'<failure message="{detail}"/>'
    elif item["status"] == "unsupported":
        body = f'<skipped message="{detail}"/>'
    cases.append(f'<testcase name="{item["id"]}" classname="sila2.csharp">{body}</testcase>')
failed = report["summary"]["fail"]
skipped = report["summary"]["unsupported"]
(out / "junit.xml").write_text(
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    f'<testsuite name="sila2-consumer-interop" tests="{len(cases)}" failures="{failed}" skipped="{skipped}">\n'
    + "\n".join(cases)
    + "\n</testsuite>\n"
)
if failed:
    raise SystemExit(1)
PY
