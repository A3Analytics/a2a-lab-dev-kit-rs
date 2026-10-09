#!/usr/bin/env bash
set -uo pipefail

fail() {
  printf 'A2A-LAB compliance action: %s\n' "$1" >&2
  exit 2
}

require_nonblank() {
  local name="$1"
  local value="$2"
  [[ -n "${value//[[:space:]]/}" ]] || fail "$name must be nonblank"
}

require_url() {
  local name="$1"
  local value="$2"
  [[ "$value" =~ ^https?://[^/[:space:]]+ ]] ||
    fail "$name must be an absolute HTTP(S) URL"
}

require_nonblank "fixtures" "${INPUT_FIXTURES:-}"
require_nonblank "report-path" "${INPUT_REPORT_PATH:-}"
require_nonblank "implementation-name" "${INPUT_IMPLEMENTATION_NAME:-}"
require_nonblank "implementation-version" "${INPUT_IMPLEMENTATION_VERSION:-}"
require_url "a2a-url" "${INPUT_A2A_URL:-}"
require_url "mcp-url" "${INPUT_MCP_URL:-}"
[[ "${INPUT_PROFILE:-}" == "1.1.0" ]] ||
  fail "profile must be 1.1.0"
[[ "${INPUT_SUITE:-}" == "basic" || "${INPUT_SUITE:-}" == "full" ]] ||
  fail "suite must be basic or full"
[[ "${INPUT_LLM_CHECK:-}" == "true" || "${INPUT_LLM_CHECK:-}" == "false" ]] ||
  fail "llm-check must be true or false"
[[ "${INPUT_TIMEOUT_MILLISECONDS:-}" =~ ^[1-9][0-9]*$ ]] ||
  fail "timeout-milliseconds must be a positive integer"
[[ -f "$INPUT_FIXTURES" ]] || fail "fixtures file does not exist: $INPUT_FIXTURES"
require_nonblank "GITHUB_OUTPUT" "${GITHUB_OUTPUT:-}"

report_path="$INPUT_REPORT_PATH"
report_directory="$(dirname "$report_path")"
mkdir -p "$report_directory" || fail "could not create report directory: $report_directory"

arguments=(
  --a2a-url "$INPUT_A2A_URL"
  --mcp-url "$INPUT_MCP_URL"
  --fixtures "$INPUT_FIXTURES"
  --profile "$INPUT_PROFILE"
  --suite "$INPUT_SUITE"
  --report "$report_path"
  --timeout-milliseconds "$INPUT_TIMEOUT_MILLISECONDS"
  --implementation-name "$INPUT_IMPLEMENTATION_NAME"
  --implementation-version "$INPUT_IMPLEMENTATION_VERSION"
)
if [[ "$INPUT_LLM_CHECK" == "false" ]]; then
  arguments+=(--no-llm-check)
fi

set +e
if [[ -n "${A2A_LAB_COMPLIANCE_BIN:-}" ]]; then
  "$A2A_LAB_COMPLIANCE_BIN" "${arguments[@]}"
  status=$?
else
  cargo run --quiet --locked \
    --manifest-path "$GITHUB_ACTION_PATH/Cargo.toml" \
    --bin a2a-lab-compliance -- "${arguments[@]}"
  status=$?
fi
set -e

compliant=false
if [[ -f "$report_path" ]]; then
  compliant="$(
    python3 - "$report_path" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as report:
    value = json.load(report).get("compliant")
if not isinstance(value, bool):
    raise SystemExit("report does not contain a boolean compliant field")
print(str(value).lower())
PY
  )" || fail "could not read compliance from report: $report_path"
fi

{
  printf 'profile=%s\n' "$INPUT_PROFILE"
  printf 'suite=%s\n' "$INPUT_SUITE"
  printf 'llm-check=%s\n' "$INPUT_LLM_CHECK"
  printf 'compliant=%s\n' "$compliant"
  printf 'report-path=%s\n' "$report_path"
} >>"$GITHUB_OUTPUT"

exit "$status"
