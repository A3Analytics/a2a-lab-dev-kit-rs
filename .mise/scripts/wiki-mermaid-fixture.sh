#!/usr/bin/env bash
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
lib="$repo/.mise/scripts/wiki-lib.sh"

fail() {
  printf 'wiki-mermaid-fixture: %s\n' "$*" >&2
  exit 1
}

write_map() {
  local dest="$1"
  cat >"$dest/.mise/wiki-map.toml" <<'EOF'
[[page]]
source = "overview"
wiki = "Home.md"

[[page]]
source = "reference/a2a"
wiki = "A2A.md"
EOF
}

write_closed_docs() {
  local dest="$1"
  mkdir -p "$dest/backlog/docs/overview" "$dest/backlog/docs/reference/a2a" "$dest/.mise"
  printf '# fixture\n' >"$dest/README.md"
  write_map "$dest"
  cat >"$dest/backlog/docs/overview/doc-1 - Fixture-Home.md" <<'EOF'
---
id: doc-1
title: Fixture Home
type: overview
audience: public
created_date: "2026-09-30"
---

# Fixture Home

## Role in this SDK

Caption for the fixture diagram.

```mermaid
flowchart TD
  # this heading is inside the fence
  nodeA["See [Lab SDK overview](<../overview/doc-1 - Fixture-Home.md>)"]
  nodeA --> nodeB[Next]
```

## Related

- [README](<../../../README.md>)
- [A2A](<../reference/a2a/doc-2 - Fixture-A2A.md>)
- GET /tasks/{id}
EOF
  cat >"$dest/backlog/docs/reference/a2a/doc-2 - Fixture-A2A.md" <<'EOF'
---
id: doc-2
title: Fixture A2A
type: reference
audience: public
created_date: "2026-09-30"
---

# Fixture A2A

## Role in this SDK

```mermaid
flowchart LR
  a[A] --> b[B]
```

## Related

- [Home](<../../overview/doc-1 - Fixture-Home.md>)
EOF
}

write_unclosed_docs() {
  local dest="$1"
  mkdir -p "$dest/backlog/docs/overview" "$dest/.mise"
  printf '# fixture\n' >"$dest/README.md"
  cat >"$dest/.mise/wiki-map.toml" <<'EOF'
[[page]]
source = "overview"
wiki = "Home.md"
EOF
  cat >"$dest/backlog/docs/overview/doc-1 - Open-Fence.md" <<'EOF'
---
id: doc-1
title: Open Fence
type: overview
audience: public
created_date: "2026-09-30"
---

# Open Fence

## Role in this SDK

```mermaid
flowchart TD
  a[A] --> b[B]

## Related

- [README](<../../../README.md>)
EOF
}

extract_mermaid_body() {
  awk '
    $0 ~ /^```mermaid([[:space:]]|$)/ { start = 1; next }
    start && $0 == "```" { exit }
    start { print }
  ' "$1"
}

decode_staged_diagram() {
  python3 -c '
import base64, pathlib, re, sys, urllib.parse
text = pathlib.Path(sys.argv[1]).read_text()
match = re.search(r"https://mermaid.ink/svg/([^)\s]+)", text)
if not match:
    sys.exit(1)
sys.stdout.write(base64.b64decode(urllib.parse.unquote(match.group(1))).decode())
' "$1"
}

closed=$(mktemp -d "${TMPDIR:-/tmp}/a2a-wiki-mermaid.XXXXXX")
unclosed=$(mktemp -d "${TMPDIR:-/tmp}/a2a-wiki-mermaid-open.XXXXXX")
cleanup() {
  rm -rf "$closed" "$unclosed"
}
trap cleanup EXIT

write_closed_docs "$closed"
wiki_cmd=wiki-mermaid-fixture
# shellcheck disable=SC2034
root="$closed"
# shellcheck disable=SC1090
source "$lib"
wiki_check

home="$closed/target/wiki-stage/Home.md"
a2a="$closed/target/wiki-stage/A2A.md"
[ -f "$home" ] || fail "missing staged Home.md"
[ -f "$a2a" ] || fail "missing staged A2A.md"

source_mermaid=$(extract_mermaid_body "$closed/backlog/docs/overview/doc-1 - Fixture-Home.md")
staged_mermaid=$(decode_staged_diagram "$home")
[ -n "$source_mermaid" ] || fail "source mermaid was empty"
[ "$source_mermaid"$'\n' = "$staged_mermaid" ] || [ "$source_mermaid" = "$staged_mermaid" ] \
  || fail "staged mermaid image did not round-trip the source diagram"

printf '%s\n' "$staged_mermaid" | grep -q '# this heading is inside the fence' \
  || fail "heading inside mermaid was dropped"
printf '%s\n' "$staged_mermaid" | grep -Fq '](<../overview/doc-1 - Fixture-Home.md>)' \
  || fail "markdown link inside mermaid was rewritten"

if grep -q '```mermaid' "$home"; then
  fail "staged Wiki kept a mermaid fence"
fi
grep -q 'https://mermaid.ink/svg/' "$home" \
  || fail "staged Wiki is missing a mermaid.ink image"

grep -q '\[README\](https://github.com/A3Analytics/a2a-lab-sdk-rs/blob/main/README.md)' "$home" \
  || fail "README link after mermaid was not rewritten"
grep -q '\[A2A\](https://github.com/A3Analytics/a2a-lab-sdk-rs/wiki/A2A)' "$home" \
  || fail "sibling Wiki link after mermaid was not rewritten"
grep -q 'GET /tasks/{id}' "$home" \
  || fail "curly braces after mermaid were rewritten"
grep -q '\* \[\[Home|Fixture Home\]\]' "$closed/target/wiki-stage/_Sidebar.md" \
  || fail "sidebar is not Wiki link syntax"

write_unclosed_docs "$unclosed"
set +e
err=$(
  wiki_cmd=wiki-mermaid-fixture
  root="$unclosed"
  # shellcheck disable=SC1090
  source "$lib"
  wiki_check 2>&1
)
status=$?
set -e
[ "$status" -ne 0 ] || fail "unclosed mermaid fence was accepted"
printf '%s\n' "$err" | grep -q 'unclosed mermaid fence' \
  || fail "unclosed mermaid error did not mention a mermaid fence"
printf '%s\n' "$err" | grep -q 'Open-Fence.md' \
  || fail "unclosed mermaid error did not name the page"

printf 'wiki-mermaid-fixture: mermaid becomes a mermaid.ink image, Wiki links stay markdown, and unclosed fences fail\n'
