#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$root/target/sila2-interop-reports"
source_dir="$root/target/sila-csharp"
commit="2625cce6541c501cb951f2eea95d490a2efd12c0"
server_name="a2a-lab-sila-provider"
client_name="a2a-lab-sila-official-client"
server_uuid="11111111-1111-1111-1111-111111111111"
port=50052
mkdir -p "$out/certs"
cd "$root"

if [ ! -d "$source_dir/.git" ]; then
  git clone --depth 1 --recurse-submodules --shallow-submodules --branch 'v.10.3.2' \
    https://gitlab.com/SiLA2/sila_csharp.git "$source_dir"
fi
if [ "$(git -C "$source_dir" rev-parse HEAD)" != "$commit" ]; then
  echo "sila_csharp checkout is not $commit" >&2
  exit 1
fi
git -C "$source_dir" submodule update --init --recursive --depth 1
if [ ! -f "$source_dir/src/sila_base/protobuf/SiLAFramework.proto" ]; then
  echo "sila_csharp is missing the sila_base protobufs" >&2
  exit 1
fi

docker build -f .mise/sila-provider-server.Dockerfile -t "$server_name" "$root"
stage="$(mktemp -d)"
cp -R "$source_dir" "$stage/sila-csharp"
cp -R .mise/sila-provider-driver "$stage/sila-provider-driver"
docker build -f "$root/.mise/sila-provider-client.Dockerfile" -t "$client_name" "$stage"
rm -rf "$stage"

network="sila-interop"
cleanup() {
  docker rm -f "$server_name" "$client_name" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
}
trap cleanup EXIT
docker rm -f "$server_name" >/dev/null 2>&1 || true
docker network create "$network" >/dev/null 2>&1 || true
docker run -d --name "$server_name" --network "$network" \
  -e SILA_HOST=0.0.0.0 \
  -e SILA_PORT="$port" \
  -e SILA_UUID="$server_uuid" \
  -e SILA_WRITE_CERTS=/certs \
  -v "$out/certs:/certs" \
  "$server_name"

ready=0
for _ in $(seq 1 90); do
  if docker logs "$server_name" 2>&1 | grep -q "^sila "; then
    ready=1
    break
  fi
  sleep 1
done
if [ "$ready" -ne 1 ]; then
  docker logs "$server_name" >&2 || true
  echo "SiLA provider did not start" >&2
  exit 1
fi

docker run --rm --network "container:$server_name" \
  -e SILA_HOST=127.0.0.1 \
  -e SILA_PORT="$port" \
  -e SILA_UUID="$server_uuid" \
  -e SILA_REPORT=/reports/csharp.json \
  -v "$out:/reports" \
  "$client_name"

server_image="$(docker image inspect --format '{{.Id}}' "$server_name")"
client_image="$(docker image inspect --format '{{.Id}}' "$client_name")"
python3 - "$out" "$server_image" "$client_image" "$root/.mise/sila-conformance/pins.json" <<'PY'
import json
from pathlib import Path
import sys
out = Path(sys.argv[1])
peer = json.loads((out / "csharp.json").read_text())
pins = json.loads(Path(sys.argv[4]).read_text())
peer["server_image"] = sys.argv[2]
peer["client_image"] = sys.argv[3]
peer["sila_csharp"] = pins["sila_csharp"]
unclassified = 0
for item in peer["capabilities"]:
    if item["Status"] not in {"pass", "fail", "unsupported"}:
        unclassified += 1
    if item["Status"] == "unsupported" and not str(item.get("Rationale", "")).strip():
        unclassified += 1
report = {
    "profile": {
        "role": "feature_provider",
        "specification": pins["specification"],
        "sila_base": pins["sila_base"],
        "peers": [peer],
    },
    "summary": peer["summary"],
    "unclassified": unclassified,
    "capabilities": peer["capabilities"],
}
(out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
cases = []
rows = []
for item in peer["capabilities"]:
    detail = str(item["Detail"]).replace("&", "&amp;").replace("<", "&lt;").replace('"', "&quot;")
    body = ""
    if item["Status"] == "fail":
        body = f'<failure message="{detail}"/>'
    elif item["Status"] == "unsupported":
        body = f'<skipped message="{detail}"/>'
    cases.append(f'<testcase name="{item["Id"]}" classname="sila2.provider">{body}</testcase>')
    rows.append(f'<tr><td>{item["Id"]}</td><td>{item["Status"]}</td><td>{detail}</td></tr>')
failed = report["summary"]["fail"]
skipped = report["summary"]["unsupported"]
(out / "junit.xml").write_text(
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    f'<testsuite name="sila2-interop" tests="{len(cases)}" failures="{failed}" skipped="{skipped}">\n'
    + "\n".join(cases)
    + "\n</testsuite>\n"
)
(out / "report.html").write_text(
    "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>SiLA interop</title></head><body>"
    f"<p>{failed} failed, {skipped} classified n/a, {unclassified} unclassified</p><table>"
    + "".join(rows)
    + "</table></body></html>\n"
)
if failed or unclassified:
    raise SystemExit(1)
PY
