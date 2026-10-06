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
source = "guide/plain"
wiki = "Plain.md"

[[page]]
source = "reference/a2a"
wiki = "A2A.md"
EOF
}

write_closed_docs() {
  local dest="$1"
  mkdir -p "$dest/backlog/docs/overview" "$dest/backlog/docs/guide/plain" \
    "$dest/backlog/docs/reference/a2a" "$dest/.mise"
  cat >"$dest/README.md" <<'EOF'
# fixture

- [Home](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/Home)
- [Plain](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/Plain)
- [A2A](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/A2A)
EOF
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

## Role in this dev kit

Caption for the fixture diagram.

```mermaid
flowchart TD
  accTitle: Fixture path
  accDescr: The fixture moves from the first node to the next node.
  # this heading is inside the fence
  nodeA["See [Lab dev kit overview](<../overview/doc-1 - Fixture-Home.md>)"]
  nodeA --> nodeB[Next]
```

## Related

- [README](<../../../README.md>)
- [A2A](<../reference/a2a/doc-2 - Fixture-A2A.md>)
- GET /tasks/{id}
EOF
  cat >"$dest/backlog/docs/guide/plain/doc-3 - Fixture-Plain.md" <<'EOF'
---
id: doc-3
title: Fixture Plain
type: guide
audience: public
created_date: "2026-09-30"
---

# Fixture Plain

## Role in this dev kit

This page has no diagram.
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

## Role in this dev kit

```mermaid
flowchart LR
  accTitle: Fixture reference
  accDescr: Node A connects to node B.
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

## Role in this dev kit

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

write_inaccessible_docs() {
  local dest="$1"
  mkdir -p "$dest/backlog/docs/overview" "$dest/.mise"
  cat >"$dest/README.md" <<'EOF'
# fixture

- [Home](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/Home)
EOF
  cat >"$dest/.mise/wiki-map.toml" <<'EOF'
[[page]]
source = "overview"
wiki = "Home.md"
EOF
  cat >"$dest/backlog/docs/overview/doc-1 - Missing-Title.md" <<'EOF'
---
id: doc-1
title: Missing Title
type: overview
audience: public
created_date: "2026-09-30"
---

# Missing Title

```mermaid
flowchart LR
  a[A] --> b[B]
```
EOF
}

closed=$(mktemp -d "${TMPDIR:-/tmp}/a2a-wiki-mermaid.XXXXXX")
unclosed=$(mktemp -d "${TMPDIR:-/tmp}/a2a-wiki-mermaid-open.XXXXXX")
inaccessible=$(mktemp -d "${TMPDIR:-/tmp}/a2a-wiki-mermaid-a11y.XXXXXX")
cleanup() {
  rm -rf "$closed" "$unclosed" "$inaccessible"
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
plain="$closed/target/wiki-stage/Plain.md"
a2a="$closed/target/wiki-stage/A2A.md"
sidebar="$closed/target/wiki-stage/_Sidebar.md"
[ -f "$home" ] || fail "missing staged Home.md"
[ -f "$plain" ] || fail "missing staged Plain.md"
[ -f "$a2a" ] || fail "missing staged A2A.md"

source_mermaid=$(extract_mermaid_body "$closed/backlog/docs/overview/doc-1 - Fixture-Home.md")
staged_mermaid=$(extract_mermaid_body "$home")
[ -n "$source_mermaid" ] || fail "source mermaid was empty"
[ "$source_mermaid" = "$staged_mermaid" ] \
  || fail "staged mermaid fence did not keep the source diagram"

printf '%s\n' "$staged_mermaid" | grep -q '# this heading is inside the fence' \
  || fail "heading inside mermaid was dropped"
printf '%s\n' "$staged_mermaid" | grep -q 'accTitle: Fixture path' \
  || fail "staged mermaid dropped accTitle"
printf '%s\n' "$staged_mermaid" | grep -q 'accDescr: The fixture moves from the first node to the next node.' \
  || fail "staged mermaid dropped accDescr"
printf '%s\n' "$staged_mermaid" | grep -Fq '](<../overview/doc-1 - Fixture-Home.md>)' \
  || fail "markdown link inside mermaid was rewritten"

grep -q '```mermaid' "$home" || fail "staged Wiki dropped the mermaid fence"
if grep -q 'https://mermaid.ink/' "$home"; then
  fail "staged Wiki converted mermaid to mermaid.ink"
fi
if grep -q '```mermaid' "$plain"; then
  fail "diagram-free page gained a mermaid fence"
fi

grep -q '\[README\](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/blob/main/README.md)' "$home" \
  || fail "README link after mermaid was not rewritten"
grep -q '\[A2A\](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/A2A)' "$home" \
  || fail "sibling Wiki link after mermaid was not rewritten"
grep -q 'GET /tasks/{id}' "$home" \
  || fail "curly braces after mermaid were rewritten"
grep -q '## Start' "$sidebar" || fail "sidebar is missing the Start group"
grep -q '## Guides' "$sidebar" || fail "sidebar is missing the Guides group"
grep -q '## Reference' "$sidebar" || fail "sidebar is missing the Reference group"
grep -q '\[Fixture Home\](https://github.com/A3Analytics/a2a-lab-dev-kit-rs/wiki/Home)' "$sidebar" \
  || fail "sidebar is not an absolute Markdown link"

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

write_inaccessible_docs "$inaccessible"
set +e
err=$(
  wiki_cmd=wiki-mermaid-fixture
  root="$inaccessible"
  # shellcheck disable=SC1090
  source "$lib"
  wiki_check 2>&1
)
status=$?
set -e
[ "$status" -ne 0 ] || fail "mermaid without accessibility text was accepted"
printf '%s\n' "$err" | grep -q 'accTitle and accDescr' \
  || fail "inaccessible mermaid error did not require accTitle and accDescr"

printf 'wiki-mermaid-fixture: native mermaid stays fenced, sidebar links are markdown, and bad fences fail\n'
