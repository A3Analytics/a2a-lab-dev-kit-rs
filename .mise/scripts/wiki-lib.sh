# Shared Wiki generation, validation, and remote helpers.
# Sourced by wiki-check.sh, wiki-publish.sh, and wiki-drift.sh.

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  printf 'wiki-lib.sh is sourced by the wiki scripts\n' >&2
  exit 2
fi

: "${root:?repo root is not set}"

WIKI_REPO="A3Analytics/a2a-lab-dev-kit-rs"
WIKI_GIT_URL="https://github.com/${WIKI_REPO}.wiki.git"
WIKI_PAGE_BASE="https://github.com/${WIKI_REPO}/wiki"
WIKI_BLOB_BASE="https://github.com/${WIKI_REPO}/blob/main"
WIKI_MAP="$root/.mise/wiki-map.toml"
WIKI_STAGE="$root/target/wiki-stage"
DOCS_ROOT="$root/backlog/docs"

page_sources=()
page_wikis=()
page_files=()
page_titles=()
page_sections=()
page_count=0

die() {
  printf '%s: %s\n' "${wiki_cmd:-wiki}" "$*" >&2
  exit 1
}

normalize_posix() {
  local path="$1" result="" part rest
  rest="$path"
  while [ -n "$rest" ]; do
    case "$rest" in
      */*)
        part=${rest%%/*}
        rest=${rest#*/}
        ;;
      *)
        part=$rest
        rest=""
        ;;
    esac
    case "$part" in
      "" | .) ;;
      ..)
        case "$result" in
          */*) result=${result%/*} ;;
          *) result="" ;;
        esac
        ;;
      *)
        if [ -z "$result" ]; then
          result=$part
        else
          result="$result/$part"
        fi
        ;;
    esac
  done
  printf '%s' "$result"
}

urlencode_component() {
  local s="$1" out="" i=0 c hex len
  len=${#s}
  while [ "$i" -lt "$len" ]; do
    c=${s:$i:1}
    case "$c" in
      [A-Za-z0-9._~-]) out="$out$c" ;;
      *)
        printf -v hex '%%%02X' "'$c"
        out="$out$hex"
        ;;
    esac
    i=$((i + 1))
  done
  printf '%s' "$out"
}

urlencode_path() {
  local path="$1" out="" part encoded rest first=1
  rest="$path"
  while [ -n "$rest" ] || [ "$first" -eq 1 ]; do
    case "$rest" in
      */*)
        part=${rest%%/*}
        rest=${rest#*/}
        ;;
      *)
        part=$rest
        rest=""
        ;;
    esac
    encoded=$(urlencode_component "$part")
    if [ "$first" -eq 1 ]; then
      out=$encoded
      first=0
    else
      out="$out/$encoded"
    fi
    [ -z "$rest" ] && break
  done
  printf '%s' "$out"
}

urldecode() {
  local s="$1" out="" hex ch
  while true; do
    case "$s" in
      *%*) ;;
      *)
        printf '%s' "$out$s"
        return 0
        ;;
    esac
    out="$out${s%%\%*}"
    s=${s#*%}
    hex=${s:0:2}
    s=${s:2}
    case "$hex" in
      [0-9A-Fa-f][0-9A-Fa-f]) ;;
      *) return 1 ;;
    esac
    printf -v ch '%b' "\\x$hex"
    out="$out$ch"
  done
}

frontmatter_value() {
  local file="$1" key="$2"
  awk -v key="$key" '
    BEGIN { state = 0 }
    state == 0 && $0 ~ /^---\r?$/ { state = 1; next }
    state == 1 && $0 ~ /^---\r?$/ { exit }
    state == 1 {
      sub(/\r$/, "")
      if (index($0, key ":") == 1) {
        val = substr($0, length(key) + 2)
        sub(/^[[:space:]]+/, "", val)
        sub(/[[:space:]]+$/, "", val)
        if (substr(val, 1, 1) == "\"" && substr(val, length(val), 1) == "\"") {
          val = substr(val, 2, length(val) - 2)
        }
        q = sprintf("%c", 39)
        if (substr(val, 1, 1) == q && substr(val, length(val), 1) == q) {
          val = substr(val, 2, length(val) - 2)
        }
        print val
        exit
      }
    }
  ' "$file"
}

extract_body() {
  local file="$1" dest="$2"
  awk '
    BEGIN { state = 0 }
    state == 0 {
      sub(/\r$/, "")
      if ($0 == "---") { state = 1; next }
      print "missing YAML frontmatter" > "/dev/stderr"
      exit 2
    }
    state == 1 {
      sub(/\r$/, "")
      if ($0 == "---") { state = 2; next }
      next
    }
    {
      sub(/\r$/, "")
      print
    }
    END {
      if (state != 2) {
        print "unterminated YAML frontmatter" > "/dev/stderr"
        exit 2
      }
    }
  ' "$file" >"$dest"
}

wiki_section_index() {
  case "$1" in
    Start) printf '0' ;;
    Guides) printf '1' ;;
    A2A-LAB) printf '2' ;;
    Interfaces) printf '3' ;;
    Providers) printf '4' ;;
    *) return 1 ;;
  esac
}

store_mapped_page() {
  local source="$1" wiki="$2" section="$3" i=0
  [ -n "$source" ] || die "wiki map page is missing source"
  [ -n "$wiki" ] || die "wiki map page is missing wiki"
  [ -n "$section" ] || die "wiki map page is missing section"
  case "$source" in
    /* | *..*) die "wiki map source must be a relative docs directory: $source" ;;
  esac
  case "$wiki" in
    *.md) ;;
    *) die "wiki output must be a .md filename: $wiki" ;;
  esac
  case "$wiki" in
    */* | _Sidebar.md) die "wiki output is not a page filename: $wiki" ;;
  esac
  wiki_section_index "$section" >/dev/null || die "unknown wiki section: $section"
  while [ "$i" -lt "$page_count" ]; do
    if [ "$source" = "${page_sources[$i]}" ] || [ "$wiki" = "${page_wikis[$i]}" ]; then
      die "duplicate wiki map entry for $source -> $wiki"
    fi
    i=$((i + 1))
  done
  page_sources[$page_count]=$source
  page_wikis[$page_count]=$wiki
  page_sections[$page_count]=$section
  page_count=$((page_count + 1))
}

load_map() {
  local line source="" wiki="" section="" open=0
  page_count=0
  page_sources=()
  page_wikis=()
  page_sections=()
  [ -f "$WIKI_MAP" ] || die "missing $WIKI_MAP"
  while IFS= read -r line || [ -n "$line" ]; do
    line=${line%$'\r'}
    case "$line" in
      "" | \#*) continue ;;
      "[[page]]")
        if [ "$open" -eq 1 ]; then
          store_mapped_page "$source" "$wiki" "$section"
        fi
        source=""
        wiki=""
        section=""
        open=1
        ;;
      source\ =\ \"*\")
        [ "$open" -eq 1 ] || die "wiki map field is outside a page: $line"
        source=${line#source = \"}
        source=${source%\"}
        ;;
      wiki\ =\ \"*\")
        [ "$open" -eq 1 ] || die "wiki map field is outside a page: $line"
        wiki=${line#wiki = \"}
        wiki=${wiki%\"}
        ;;
      section\ =\ \"*\")
        [ "$open" -eq 1 ] || die "wiki map field is outside a page: $line"
        section=${line#section = \"}
        section=${section%\"}
        ;;
      *)
        die "cannot parse wiki map line: $line"
        ;;
    esac
  done <"$WIKI_MAP"
  if [ "$open" -eq 1 ]; then
    store_mapped_page "$source" "$wiki" "$section"
  fi
  [ "$page_count" -gt 0 ] || die "wiki map has no pages"
}

validate_map_groups() {
  local i=0 section="" index="" last=-1 saw_home=0
  while [ "$i" -lt "$page_count" ]; do
    section=${page_sections[$i]}
    index=$(wiki_section_index "$section") || die "unknown wiki section: $section"
    if [ "$last" -ge 0 ] && [ "$index" -lt "$last" ]; then
      die "wiki section $section is out of order"
    fi
    if [ "$last" -ge 0 ] && [ "$index" -gt $((last + 1)) ]; then
      die "wiki map skips a section before $section"
    fi
    last=$index
    if [ "${page_wikis[$i]}" = "Home.md" ]; then
      saw_home=1
    fi
    i=$((i + 1))
  done
  [ "$saw_home" -eq 1 ] || die "wiki map must include Home.md"
}

expected_type_for() {
  local source="$1" top
  top=${source%%/*}
  case "$top" in
    overview | reference | guide) printf '%s' "$top" ;;
    *) die "Wiki source must sit under overview, reference, or guide: $source" ;;
  esac
}

validate_mermaid_ids() {
  local body="$1" label="$2" line in_mermaid=0
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      '```mermaid' | '```mermaid'*)
        in_mermaid=1
        continue
        ;;
      '```'* | '~~~'*)
        in_mermaid=0
        continue
        ;;
    esac
    [ "$in_mermaid" -eq 1 ] || continue
    if printf '%s\n' "$line" | grep -Eq '(^|[[:space:]])(graph|end|subgraph|flowchart)(\[|\{|\(|[[:space:]]*-->)'; then
      die "reserved Mermaid node id in ${label}: ${line}"
    fi
  done <"$body"
}

validate_mermaid_a11y() {
  local body="$1" label="$2" line in_mermaid=0 has_title=0 has_descr=0 trimmed text
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      '```mermaid' | '```mermaid'*)
        in_mermaid=1
        has_title=0
        has_descr=0
        continue
        ;;
      '```'* | '~~~'*)
        if [ "$in_mermaid" -eq 1 ]; then
          [ "$has_title" -eq 1 ] && [ "$has_descr" -eq 1 ] \
            || die "mermaid diagram in ${label} needs accTitle and accDescr"
          in_mermaid=0
        fi
        continue
        ;;
    esac
    [ "$in_mermaid" -eq 1 ] || continue
    trimmed=$(printf '%s' "$line" | sed 's/^[[:space:]]*//')
    case "$trimmed" in
      accTitle:*)
        text=${trimmed#accTitle:}
        text=$(printf '%s' "$text" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
        [ -n "$text" ] && has_title=1
        ;;
      accDescr:*)
        text=${trimmed#accDescr:}
        text=$(printf '%s' "$text" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
        [ -n "$text" ] && has_descr=1
        ;;
    esac
  done <"$body"
  [ "$in_mermaid" -eq 0 ] || die "unclosed mermaid fence in ${label}"
}

validate_h1() {
  local body="$1" title="$2" line in_fence=0 count=0 found=""
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      '```'* | '~~~'*)
        if [ "$in_fence" -eq 0 ]; then
          in_fence=1
        else
          in_fence=0
        fi
        continue
        ;;
    esac
    [ "$in_fence" -eq 1 ] && continue
    case "$line" in
      '# '[!\#]*)
        count=$((count + 1))
        found=${line#'# '}
        found=$(printf '%s' "$found" | sed 's/[[:space:]]*$//')
        ;;
    esac
  done <"$body"
  [ "$in_fence" -eq 0 ] || die "unclosed fence while checking H1"
  [ "$count" -eq 1 ] || die "expected one H1, found $count"
  [ "$found" = "$title" ] || die "H1 '$found' does not match title '$title'"
}

validate_fences() {
  local body="$1" label="$2" line in_fence=0 opener=""
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      '```'* | '~~~'*)
        if [ "$in_fence" -eq 0 ]; then
          in_fence=1
          opener=$line
        else
          in_fence=0
          opener=""
        fi
        ;;
    esac
  done <"$body"
  if [ "$in_fence" -ne 0 ]; then
    case "$opener" in
      '```mermaid' | '```mermaid'*)
        die "unclosed mermaid fence in ${label}"
        ;;
      *)
        die "unclosed fence in ${label}: ${opener}"
        ;;
    esac
  fi
}

prepare_pages() {
  local i=0 source dir found count abs title type audience expected body mermaid_count
  wiki_tmp=$(mktemp -d "${TMPDIR:-/tmp}/a2a-wiki.XXXXXX")
  while [ "$i" -lt "$page_count" ]; do
    source=${page_sources[$i]}
    dir="$DOCS_ROOT/$source"
    [ -d "$dir" ] || die "missing docs directory backlog/docs/$source"
    found=$(find "$dir" -maxdepth 1 -type f -name '*.md' | sort)
    count=0
    abs=""
    while IFS= read -r line || [ -n "$line" ]; do
      [ -z "$line" ] && continue
      count=$((count + 1))
      abs=$line
    done <<EOF
$found
EOF
    [ "$count" -eq 1 ] || die "backlog/docs/$source must contain exactly one markdown file (found $count)"
    page_files[$i]=${abs#"$root/"}
    if grep -q '<!--' "$abs"; then
      die "HTML comment in ${page_files[$i]}"
    fi
    title=$(frontmatter_value "$abs" title)
    type=$(frontmatter_value "$abs" type)
    audience=$(frontmatter_value "$abs" audience)
    [ -n "$title" ] || die "missing title in ${page_files[$i]}"
    [ -n "$type" ] || die "missing type in ${page_files[$i]}"
    [ "$audience" = "public" ] || die "${page_files[$i]} audience must be public"
    case "$type" in
      overview | reference | guide) ;;
      *) die "${page_files[$i]} type '$type' is not allowed for a public Wiki page" ;;
    esac
    expected=$(expected_type_for "$source")
    [ "$type" = "$expected" ] || die "${page_files[$i]} type '$type' does not match directory '$source'"
    body="$wiki_tmp/$i.body"
    extract_body "$abs" "$body" || die "could not read frontmatter in ${page_files[$i]}"
    validate_fences "$body" "${page_files[$i]}" || die "fence check failed for ${page_files[$i]}"
    validate_h1 "$body" "$title" || die "H1 check failed for ${page_files[$i]}"
    validate_mermaid_ids "$body" "${page_files[$i]}"
    validate_mermaid_a11y "$body" "${page_files[$i]}"
    page_titles[$i]=$title
    i=$((i + 1))
  done
}

ensure_public_docs_are_mapped() {
  local found abs rel dir audience i mapped line
  found=$(find "$DOCS_ROOT" -type f -name '*.md' | sort)
  while IFS= read -r abs || [ -n "$abs" ]; do
    [ -z "$abs" ] && continue
    rel=${abs#"$root/"}
    dir=$(dirname "$rel")
    dir=${dir#backlog/docs/}
    audience=$(frontmatter_value "$abs" audience)
    mapped=0
    i=0
    while [ "$i" -lt "$page_count" ]; do
      if [ "$dir" = "${page_sources[$i]}" ]; then
        mapped=1
        break
      fi
      i=$((i + 1))
    done
    if [ "$audience" = "public" ] && [ "$mapped" -eq 0 ]; then
      die "$rel is public and is not listed in .mise/wiki-map.toml"
    fi
  done <<EOF
$found
EOF
}

rewrite_url() {
  local url="$1" src_dir="$2" fragment="" path query="" combined normalized i name encoded
  case "$url" in
    \#*)
      REWRITTEN_URL=$url
      return 0
      ;;
  esac
  case "$url" in
    *\#*)
      fragment="#${url#*#}"
      path=${url%%#*}
      ;;
    *) path=$url ;;
  esac
  case "$path" in
    http://* | https://*)
      REWRITTEN_URL=$url
      return 0
      ;;
    "")
      printf 'empty link in %s\n' "$src_dir" >&2
      return 1
      ;;
  esac
  case "$path" in
    *\?*)
      query="?${path#*\?}"
      path=${path%%\?*}
      ;;
  esac
  case "$path" in
    /*) normalized=${path#/} ;;
    *)
      combined=$(normalize_posix "$src_dir/$path")
      normalized=$combined
      ;;
  esac
  i=0
  while [ "$i" -lt "$page_count" ]; do
    if [ "$normalized" = "${page_files[$i]}" ]; then
      name=${page_wikis[$i]%.md}
      REWRITTEN_URL="${WIKI_PAGE_BASE}/${name}${fragment}"
      return 0
    fi
    i=$((i + 1))
  done
  if [ ! -f "$root/$normalized" ]; then
    printf 'link target does not exist: %s\n' "$normalized" >&2
    return 1
  fi
  encoded=$(urlencode_path "$normalized")
  REWRITTEN_URL="${WIKI_BLOB_BASE}/${encoded}${query}${fragment}"
}

rewrite_line() {
  local src_dir="$1" cursor="$2" out="" prefix text url
  while true; do
    case "$cursor" in
      *'['*) ;;
      *)
        REWRITTEN_LINE="$out$cursor"
        return 0
        ;;
    esac
    prefix=${cursor%%\[*}
    cursor=${cursor#*\[}
    case "$cursor" in
      *']('*) ;;
      *)
        out="$out$prefix["
        continue
        ;;
    esac
    text=${cursor%%\]\(*}
    cursor=${cursor#*\]\(}
    case "$cursor" in
      '<'*)
        url=${cursor%%>*}
        url=${url#<}
        cursor=${cursor#*>}
        case "$cursor" in
          ')'*) cursor=${cursor#\)} ;;
          *)
            printf 'unclosed link target <%s>\n' "$url" >&2
            return 1
            ;;
        esac
        ;;
      *)
        url=${cursor%%\)*}
        cursor=${cursor#*\)}
        ;;
    esac
    rewrite_url "$url" "$src_dir" || return 1
    out="$out$prefix[$text]($REWRITTEN_URL)"
  done
}

render_pages() {
  local i=0 src_dir line started dest in_fence
  case "$WIKI_STAGE" in
    "$root"/target/wiki-stage) ;;
    *) die "refusing to replace unexpected stage path $WIKI_STAGE" ;;
  esac
  rm -rf "$WIKI_STAGE"
  mkdir -p "$WIKI_STAGE"
  while [ "$i" -lt "$page_count" ]; do
    src_dir=$(dirname "${page_files[$i]}")
    dest="$WIKI_STAGE/${page_wikis[$i]}"
    started=0
    in_fence=0
    : >"$dest"
    while IFS= read -r line || [ -n "$line" ]; do
      if [ "$started" -eq 0 ] && [ -z "$line" ]; then
        continue
      fi
      started=1
      case "$line" in
        '```'* | '~~~'*)
          if [ "$in_fence" -eq 0 ]; then
            in_fence=1
          else
            in_fence=0
          fi
          printf '%s\n' "$line" >>"$dest"
          continue
          ;;
      esac
      if [ "$in_fence" -eq 1 ]; then
        printf '%s\n' "$line" >>"$dest"
        continue
      fi
      rewrite_line "$src_dir" "$line" || die "could not rewrite links in ${page_files[$i]}"
      printf '%s\n' "$REWRITTEN_LINE" >>"$dest"
    done <"$wiki_tmp/$i.body"
    if [ "$in_fence" -ne 0 ]; then
      die "unclosed fence in ${page_files[$i]}"
    fi
    i=$((i + 1))
  done
  write_sidebar
}

write_sidebar() {
  local i=0 heading="" current="" name
  {
    while [ "$i" -lt "$page_count" ]; do
      heading=${page_sections[$i]}
      if [ "$heading" != "$current" ]; then
        if [ -n "$current" ]; then
          printf '\n'
        fi
        printf '## %s\n\n' "$heading"
        current=$heading
      fi
      name=${page_wikis[$i]%.md}
      printf -- '- [%s](%s/%s)\n' "${page_titles[$i]}" "$WIKI_PAGE_BASE" "$name"
      i=$((i + 1))
    done
  } >"$WIKI_STAGE/_Sidebar.md"
}

verify_absolute_link() {
  local url="$1" prefix rest repo_path decoded
  prefix="${WIKI_BLOB_BASE%main}"
  case "$url" in
    "$prefix"*)
      rest=${url#"$prefix"}
      rest=${rest%%#*}
      rest=${rest%%\?*}
      case "$rest" in
        */*) repo_path=${rest#*/} ;;
        *) die "blob link is missing a path: $url" ;;
      esac
      decoded=$(urldecode "$repo_path") || die "blob link has invalid percent-encoding: $url"
      [ -f "$root/$decoded" ] || die "blob link does not match a repository file: $url"
      ;;
    http://* | https://*) ;;
    *) die "unsupported link: $url" ;;
  esac
}

resolve_staged_link() {
  local url="$1" path normalized
  case "$url" in
    \#*) return 0 ;;
  esac
  path=$url
  case "$path" in
    *\#*) path=${path%%#*} ;;
  esac
  case "$path" in
    http://* | https://*)
      verify_absolute_link "$path"
      return 0
      ;;
    "") return 0 ;;
  esac
  case "$path" in
    /* | *://*) die "unresolved Wiki link: $url" ;;
  esac
  normalized=$(normalize_posix "$path")
  case "$normalized" in
    .. | ../* | */.. | */../*) die "Wiki link escapes the stage: $url" ;;
  esac
  if [ -f "$WIKI_STAGE/$normalized" ] || [ -f "$WIKI_STAGE/$normalized.md" ]; then
    return 0
  fi
  die "unresolved Wiki link: $url"
}

check_line_links() {
  local cursor="$1" prefix url
  while true; do
    case "$cursor" in
      *'['*) ;;
      *) return 0 ;;
    esac
    prefix=${cursor%%\[*}
    cursor=${cursor#*\[}
    case "$cursor" in
      *']('*) ;;
      *) continue ;;
    esac
    cursor=${cursor#*\]\(}
    case "$cursor" in
      '<'*)
        url=${cursor%%>*}
        url=${url#<}
        cursor=${cursor#*>}
        case "$cursor" in
          ')'*) cursor=${cursor#\)} ;;
          *) die "unclosed link in staged Wiki" ;;
        esac
        ;;
      *)
        url=${cursor%%\)*}
        cursor=${cursor#*\)}
        ;;
    esac
    resolve_staged_link "$url" || return 1
    prefix=$prefix
  done
}

check_wiki_bracket_links() {
  local cursor="$1" inner page
  while true; do
    case "$cursor" in
      *'[['*) ;;
      *) return 0 ;;
    esac
    cursor=${cursor#*\[\[}
    case "$cursor" in
      *']]'*) ;;
      *) die "unclosed Wiki link" ;;
    esac
    inner=${cursor%%\]\]*}
    cursor=${cursor#*\]\]}
    page=${inner%%|*}
    page=${page%%\#*}
    [ -n "$page" ] || die "empty Wiki link"
    resolve_staged_link "$page" || return 1
  done
}

check_mermaid_diagrams() {
  local file base
  for file in "$WIKI_STAGE"/*.md; do
    [ -f "$file" ] || continue
    base=$(basename "$file")
    [ "$base" = "_Sidebar.md" ] && continue
    if grep -q 'https://mermaid.ink/' "$file"; then
      die "$base must keep native Mermaid fences"
    fi
    validate_mermaid_a11y "$file" "$base"
  done
}

check_readme_wiki_parity() {
  local readme="$root/README.md"
  [ -f "$readme" ] || die "missing README.md"
  grep -q "${WIKI_PAGE_BASE}/Home" "$readme" \
    || die "README must link to the Wiki overview"
}

check_staged_links() {
  local file line in_fence lineno
  for file in "$WIKI_STAGE"/*.md; do
    [ -f "$file" ] || continue
    if grep -q '<!--' "$file"; then
      die "HTML comment in staged $(basename "$file")"
    fi
    in_fence=0
    lineno=0
    while IFS= read -r line || [ -n "$line" ]; do
      lineno=$((lineno + 1))
      case "$line" in
        '```'* | '~~~'*)
          if [ "$in_fence" -eq 0 ]; then
            in_fence=1
          else
            in_fence=0
          fi
          continue
          ;;
      esac
      [ "$in_fence" -eq 1 ] && continue
      check_line_links "$line" || die "unresolved link in $(basename "$file"):$lineno"
      check_wiki_bracket_links "$line" || die "unresolved Wiki link in $(basename "$file"):$lineno"
    done <"$file"
  done
}

wiki_check() {
  local saved_cmd=${wiki_cmd:-wiki-check}
  wiki_cmd=wiki-check
  load_map
  validate_map_groups
  prepare_pages
  ensure_public_docs_are_mapped
  render_pages
  [ -f "$WIKI_STAGE/Home.md" ] || die "staged Home.md is missing"
  check_staged_links
  check_mermaid_diagrams
  check_readme_wiki_parity
  if [ -n "${wiki_tmp:-}" ]; then
    rm -rf "$wiki_tmp"
    wiki_tmp=""
  fi
  printf 'wiki-check: wrote %s Wiki pages to target/wiki-stage\n' "$page_count"
  wiki_cmd=$saved_cmd
}

wiki_access_fail() {
  printf '%s: GitHub Wiki access is unavailable for %s. Enable Wikis, create the first page in the GitHub Wiki tab so .wiki.git exists, and publish from GitHub Actions. This command does not force-push.\n' \
    "${wiki_cmd:-wiki}" "$WIKI_GIT_URL" >&2
  exit 1
}

wiki_disable_interactive_auth() {
  unset GIT_ASKPASS
  unset SSH_ASKPASS
  unset VSCODE_GIT_ASKPASS_MAIN
  unset VSCODE_GIT_ASKPASS_NODE
  unset VSCODE_GIT_ASKPASS_EXTRA_ARGS
  unset VSCODE_GIT_IPC_HANDLE
  export GIT_TERMINAL_PROMPT=0
}

wiki_token() {
  if [ -n "${WIKI_TOKEN:-}" ]; then
    printf '%s' "$WIKI_TOKEN"
    return 0
  fi
  return 1
}

wiki_redact() {
  local text="$1" token=""
  if token=$(wiki_token); then
    text=${text//$token/********}
  fi
  printf '%s' "$text"
}

wiki_git() {
  local token="" header
  wiki_disable_interactive_auth
  if token=$(wiki_token); then
    header=$(printf 'x-access-token:%s' "$token" | base64 | tr -d '\n')
    git -c credential.helper= \
      -c "http.https://github.com/.extraheader=AUTHORIZATION: basic ${header}" \
      "$@"
    return
  fi
  git -c credential.helper= "$@"
}

wiki_default_branch() {
  local line
  line=$(wiki_git ls-remote --symref "$WIKI_GIT_URL" HEAD 2>"$wiki_tmp/git.err") || return 1
  line=$(printf '%s\n' "$line" | awk 'NR == 1 { print $2 }')
  case "$line" in
    refs/heads/*) printf '%s\n' "${line#refs/heads/}" ;;
    *) return 1 ;;
  esac
}

wiki_fetch_clone() {
  local dest="$1" branch
  mkdir -p "$(dirname "$dest")"
  wiki_tmp=${wiki_tmp:-$(mktemp -d "${TMPDIR:-/tmp}/a2a-wiki.XXXXXX")}
  mkdir -p "$wiki_tmp"
  if ! branch=$(wiki_default_branch); then
    if [ -f "$wiki_tmp/git.err" ]; then
      wiki_redact "$(cat "$wiki_tmp/git.err")" >&2
      printf '\n' >&2
    fi
    wiki_access_fail
  fi
  if [ ! -d "$dest/.git" ]; then
    if ! wiki_git clone --quiet --depth 1 --branch "$branch" "$WIKI_GIT_URL" "$dest"; then
      wiki_access_fail
    fi
  else
    if ! wiki_git -C "$dest" fetch --quiet --depth 1 origin "+refs/heads/${branch}:refs/remotes/origin/${branch}"; then
      wiki_access_fail
    fi
  fi
  printf '%s\n' "$branch"
}
