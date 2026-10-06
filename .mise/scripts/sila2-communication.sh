#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$root/target/sila2-conformance"
port=50053
mkdir -p "$out"
cd "$root"

cargo build -p sila2-conformance --offline
server="$root/target/debug/sila2-conformance"
"$server" "127.0.0.1:$port" >"$out/server.log" 2>&1 &
server_pid=$!
cleanup() {
  kill "$server_pid" >/dev/null 2>&1 || true
}
trap cleanup EXIT

ready=0
for _ in $(seq 1 50); do
  if python3 -c 'import socket,sys; s=socket.create_connection((sys.argv[1], int(sys.argv[2])), 0.2)' 127.0.0.1 "$port" >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.2
done
if [ "$ready" -ne 1 ]; then
  cat "$out/server.log" >&2 || true
  echo "communication-tester server did not listen" >&2
  exit 1
fi

version="$(sed -n 's/.*sila2-interop-communication-tester==\([0-9.]*\).*/\1/p' .mise/sila-communication-tester.Dockerfile)"
venv="$out/venv"
if [ ! -x "$venv/bin/python" ]; then
  python3 -m venv "$venv"
fi
"$venv/bin/pip" install --disable-pip-version-check "sila2-interop-communication-tester==$version"
set +e
"$venv/bin/python" -m sila2_interop_communication_tester.test_client \
  --server-address "127.0.0.1:$port" \
  --report-file "$out/communication-junit.xml" \
  --html-file "$out/communication.html" \
  --testsuite-name sila2-communication
status=$?
set -e

python3 - "$out" "$status" <<'PY'
import sys
import xml.etree.ElementTree as ET
from pathlib import Path
import json
out = Path(sys.argv[1])
status = int(sys.argv[2])
path = out / "communication-junit.xml"
if not path.exists():
    report = {"passed": 0, "failed": 1, "skipped": 0, "unclassified": 1, "exit": status}
else:
    root = ET.parse(path).getroot()
    suites = [root] if root.tag == "testsuite" else list(root.findall("testsuite"))
    tests = failures = skipped = errors = 0
    for suite in suites:
        tests += int(suite.attrib.get("tests", 0))
        failures += int(suite.attrib.get("failures", 0))
        skipped += int(suite.attrib.get("skipped", 0))
        errors += int(suite.attrib.get("errors", 0))
    failed = failures + errors
    report = {
        "passed": max(tests - failed - skipped, 0),
        "failed": failed,
        "skipped": skipped,
        "unclassified": 0 if status == 0 or failed else 1,
        "exit": status,
    }
(out / "communication.json").write_text(json.dumps(report, indent=2) + "\n")
if report["failed"] or report["unclassified"] or status:
    raise SystemExit(1)
PY
