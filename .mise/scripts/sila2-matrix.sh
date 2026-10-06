#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
schema="$root/.mise/sila-conformance/schema/FeatureDefinition.xsd"
normative="$root/.mise/sila-conformance/normative"

for feature in src/sila/standard/*.sila.xml; do
  xmllint --noout --schema "$schema" "$feature"
done

cmp -s src/sila/standard/SiLAService.sila.xml "$normative/SiLAService-v1_0.sila.xml"
cmp -s src/sila/standard/CancelController.sila.xml "$normative/CancelController-v1_0.sila.xml"
cmp -s src/sila/standard/ConnectionConfigurationService.sila.xml \
  "$normative/ConnectionConfigurationService-v1_1.sila.xml"

python3 - "$root" <<'PY'
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
rows = json.loads((root / ".mise/sila-conformance/requirements.json").read_text())
sources = []
for folder in ("src", "tests"):
    for path in (root / folder).rglob("*.rs"):
        sources.append(path.read_text())
blob = "\n".join(sources)
failed = False
for row in rows:
    status = row.get("status")
    if status == "pass":
        test = row.get("test", "")
        if not test or f"fn {test}" not in blob:
            print(f"missing test for {row['id']}: {test}", file=sys.stderr)
            failed = True
    elif status == "na":
        if not str(row.get("rationale", "")).strip():
            print(f"missing N/A rationale for {row['id']}", file=sys.stderr)
            failed = True
    else:
        print(f"unclassified requirement {row['id']}", file=sys.stderr)
        failed = True
if failed:
    raise SystemExit(1)
print(f"sila2 matrix: {sum(row['status'] == 'pass' for row in rows)} pass, {sum(row['status'] == 'na' for row in rows)} n/a")
PY
