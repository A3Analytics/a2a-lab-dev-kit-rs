#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$root/target/sila2-conformance"
mkdir -p "$out"

python3 - "$root" "$out" <<'PY'
import json
from datetime import date
from pathlib import Path
import sys

root = Path(sys.argv[1])
out = Path(sys.argv[2])
pins = json.loads((root / ".mise/sila-conformance/pins.json").read_text())
requirements = json.loads((root / ".mise/sila-conformance/requirements.json").read_text())
interop_path = root / "target/sila2-interop-reports/report.json"
communication_path = out / "communication.json"
missing = [str(path.relative_to(root)) for path in (interop_path, communication_path) if not path.exists()]
if missing:
    print("missing local evidence: " + ", ".join(missing), file=sys.stderr)
    raise SystemExit(1)
interop = json.loads(interop_path.read_text())
communication = json.loads(communication_path.read_text())
if interop["summary"]["fail"] or interop.get("unclassified", 0):
    print("direct interop has applicable failures", file=sys.stderr)
    raise SystemExit(1)
if communication.get("failed", 0) or communication.get("unclassified", 0):
    print("official communication tester has applicable failures", file=sys.stderr)
    raise SystemExit(1)
manifest = {
    "generated": date.today().isoformat(),
    "claim": "local self-assessment of applicable SiLA 2 Part A and Part B v1.1 requirements",
    "published": False,
    "pins": pins,
    "requirements": requirements,
    "direct_interop": interop["summary"],
    "communication_tester": {
        "version": pins["communication_tester"]["version"],
        "passed": communication.get("passed", 0),
        "failed": communication.get("failed", 0),
        "skipped": communication.get("skipped", 0),
    },
    "not_applicable": [row for row in requirements if row["status"] == "na"],
}
(out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(f"local self-assessment written to {out / 'manifest.json'}")
PY
