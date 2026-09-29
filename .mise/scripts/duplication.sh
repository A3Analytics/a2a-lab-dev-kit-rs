#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

mode="${1:-production}"
case "$mode" in
  test)
    default_advisory=1
    threshold=12.0
    ;;
  *)
    mode=production
    default_advisory=0
    threshold=0.5
    ;;
esac
advisory="${DUPLICATION_ADVISORY:-$default_advisory}"
large_block=40

printf 'processing: duplication (%s)\n' "$mode"
SECONDS=0

is_test_file() {
  case "$1" in
    *_tests.rs|*/tests.rs|*/tests/*|*_tests/*|*/test_harness/*|*/integration_test_support/*|*test_support*|*testutil*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

list_files() {
  git ls-files --cached --others --exclude-standard -- '*.rs' | while IFS= read -r f; do
    case "$f" in
      target/*) continue ;;
    esac
    if [ "$mode" = test ]; then
      if is_test_file "$f"; then
        printf '%s\n' "$f"
      fi
    else
      if ! is_test_file "$f"; then
        printf '%s\n' "$f"
      fi
    fi
  done
}

sources=0
total_lines=0
while IFS= read -r f; do
  [ -n "$f" ] || continue
  [ -f "$f" ] || continue
  sources=$((sources + 1))
  n=$(wc -l <"$f")
  total_lines=$((total_lines + n))
done < <(list_files)

set +e
kiss dry --lang rust . --ignore target >/dev/null 2>&1
kiss_out=$(kiss check --lang rust . 2>&1)
set -e

stats=$(printf '%s\n' "$kiss_out" | awk -v mode="$mode" -v total="$total_lines" -v th="$threshold" -v large="$large_block" '
function is_test(p) {
  return p ~ /_tests\.rs$/ || p ~ /\/tests\.rs$/ || p ~ /\/tests\// || p ~ /_tests\// \
    || p ~ /test_harness\// || p ~ /integration_test_support\// || p ~ /test_support/ || p ~ /testutil/
}
function range_lines(s,    rest, j, start, end, i) {
  i = index(s, ":")
  if (i == 0) return 0
  rest = substr(s, i + 1)
  j = index(rest, "-")
  if (j == 0) return 0
  start = substr(rest, 1, j - 1) + 0
  end = substr(rest, j + 1) + 0
  return end - start + 1
}
function keep_path(p) {
  if (mode == "test") return is_test(p)
  return !is_test(p)
}
/^VIOLATION:duplication:/ {
  line = $0
  sub(/^VIOLATION:duplication:/, "", line)
  split(line, halves, " copies: ")
  copies = halves[2]
  sub(/^\[/, "", copies)
  sub(/\].*$/, "", copies)
  nc = split(copies, parts, ", ")
  block = 0
  kept = 0
  for (i = 1; i <= nc; i++) {
    cp = parts[i]
    sub(/^ /, "", cp)
    p = cp
    sub(/:[0-9]+-[0-9]+$/, "", p)
    if (!keep_path(p)) continue
    kept++
    rl = range_lines(cp)
    if (rl > block) block = rl
  }
  if (kept < 2) next
  clones++
  dup += block
  if (block > maxb) maxb = block
}
END {
  pct = 0
  if (total > 0) pct = dup * 100.0 / total
  breach = 0
  if (pct >= th) breach = 1
  if (maxb >= large) breach = 1
  printf "%.6f %d %d %d", pct, clones + 0, dup + 0, breach
}
')

pct=$(printf '%s' "$stats" | awk '{print $1}')
clones=$(printf '%s' "$stats" | awk '{print $2}')
dup=$(printf '%s' "$stats" | awk '{print $3}')
breach=$(printf '%s' "$stats" | awk '{print $4}')

printf 'duplication: %.2f%% (%s clones, %s duplicated lines, %s sources)\n' "$pct" "$clones" "$dup" "$sources"
printf 'processing: duplication (%s) - finished in: (%ds)\n' "$mode" "$SECONDS"

rc=0
if [ "$breach" = 1 ]; then
  rc=1
fi

if [ "$advisory" = "1" ]; then
  if [ "$rc" -ne 0 ]; then
    printf 'duplication: advisory (threshold breach suppressed)\n' >&2
  fi
  exit 0
fi

exit "$rc"
