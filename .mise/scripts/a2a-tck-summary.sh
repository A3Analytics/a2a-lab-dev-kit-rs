#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
report="$root/target/a2a-tck-reports/compatibility.json"
summary="${GITHUB_STEP_SUMMARY:-}"

if [ -z "$summary" ]; then
  exit 0
fi

if [ ! -f "$report" ]; then
  echo "The A2A TCK did not write a compatibility report." >>"$summary"
  exit 0
fi

python3 - "$report" >>"$summary" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1]))
summary = data["summary"]
levels = ("MUST", "SHOULD", "MAY")
transports = ("agent_card", "grpc", "http_json", "jsonrpc")


def cell(value):
    return str(value).replace("|", "\\|").replace("\n", " ")


print("# A2A TCK")
print()
card = data.get("agent_card") or {}
version = card.get("version", "") if isinstance(card, dict) else ""
name = card.get("name", "") if isinstance(card, dict) else ""
print(f"**{cell(summary['overall_compatibility'])}** compatible")
print()
if name or version:
    print(f"{cell(name)} {cell(version)}".strip())
    print()
print("| Level | Compatibility |")
print("| --- | --- |")
for level, key in (
    ("MUST", "must_compatibility"),
    ("SHOULD", "should_compatibility"),
    ("MAY", "may_compatibility"),
):
    print(f"| {level} | {cell(summary[key])} |")
print()
print("| Transport | Passed | Failed | Skipped | Total |")
print("| --- | ---: | ---: | ---: | ---: |")
for name, counts in data["per_transport"].items():
    print(
        f"| {cell(name)} | {counts['passed']} | {counts['failed']} | {counts['skipped']} | {counts['total']} |"
    )
print()
print("| Requirement | Level | Status | agent_card | grpc | http_json | jsonrpc | Errors |")
print("| --- | --- | --- | --- | --- | --- | --- | --- |")
requirements = sorted(
    data["per_requirement"].items(),
    key=lambda item: (levels.index(item[1]["level"]), item[0]),
)
for requirement_id, result in requirements:
    by_transport = result.get("transports") or {}
    columns = [cell(by_transport.get(name, "")) for name in transports]
    errors = "; ".join(result.get("errors") or [])
    print(
        "| "
        + " | ".join(
            [
                cell(requirement_id),
                cell(result["level"]),
                cell(result["status"]),
                *columns,
                cell(errors),
            ]
        )
        + " |"
    )
PY
