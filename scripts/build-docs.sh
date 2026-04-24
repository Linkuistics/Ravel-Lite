#!/usr/bin/env bash
# Build the .adoc chapters under docs/ into HTML pages wrapped in the
# Linkuistics site chrome. Project-specific values live in
# docs/build-config.sh; chapter order comes from docs/manifest.txt; the
# HTML shell is docs/templates/page-shell.html. Requires `asciidoctor`
# on PATH — `brew install asciidoctor` on macOS.

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DOCS_DIR="$ROOT/docs"
CONFIG="$DOCS_DIR/build-config.sh"
MANIFEST="$DOCS_DIR/manifest.txt"
SHELL_TPL="$DOCS_DIR/templates/page-shell.html"

for f in "$CONFIG" "$MANIFEST" "$SHELL_TPL"; do
  [ -f "$f" ] || { echo "missing $f" >&2; exit 1; }
done

# shellcheck disable=SC1090
source "$CONFIG"

command -v asciidoctor >/dev/null || {
  echo "asciidoctor not found — try 'brew install asciidoctor'" >&2
  exit 1
}

# Resolve OUTPUT_DIR relative to the repo root so configs can be written
# as path expressions (e.g. '../www.linkuistics.com/projects/ravel-lite')
# without depending on the caller's cwd.
if [ -z "${OUTPUT_DIR:-}" ]; then
  echo "build-config.sh must set OUTPUT_DIR" >&2
  exit 1
fi
case "$OUTPUT_DIR" in
  /*) OUT_DIR_ABS="$OUTPUT_DIR" ;;
  *)  OUT_DIR_ABS="$ROOT/$OUTPUT_DIR" ;;
esac
# Require the site checkout's parent to exist (the sibling repo), but
# create the per-project subdir on demand — matches how other projects'
# subdirs under linkuistics/projects/ come and go.
OUT_PARENT=$(dirname "$OUT_DIR_ABS")
if [ ! -d "$OUT_PARENT" ]; then
  echo "site checkout not found: $OUT_PARENT" >&2
  echo "(expected a sibling site checkout at that path)" >&2
  exit 1
fi
mkdir -p "$OUT_DIR_ABS"
OUT_DIR_ABS=$(cd "$OUT_DIR_ABS" && pwd)

MANIFEST_ENTRIES=()
while IFS= read -r line; do
  case "$line" in
    '' | \#*) continue ;;
    *) MANIFEST_ENTRIES+=("$line") ;;
  esac
done < "$MANIFEST"

if [ "${#MANIFEST_ENTRIES[@]}" -eq 0 ]; then
  echo "manifest is empty: $MANIFEST" >&2
  exit 1
fi

# Title is the first `= Heading` line; asciidoctor requires it to be on
# the first line of the document anyway.
extract_title() {
  awk 'NR==1 && /^= / { sub(/^= /,""); print; exit }' "$1"
}

# Flatten 'tutorial/00-overview.adoc' -> 'tutorial-00-overview' so the
# output site stays one directory deep under projects/<slug>/.
flatten_name() {
  local rel="$1"
  echo "${rel%.adoc}" | tr '/' '-'
}

chapter_nav_html() {
  local idx="$1"
  local prev_html=""
  local next_html=""

  if [ "$idx" -gt 0 ]; then
    local prev_src="${MANIFEST_ENTRIES[$((idx-1))]}"
    local prev_title prev_name
    prev_title=$(extract_title "$DOCS_DIR/$prev_src")
    prev_name=$(flatten_name "$prev_src")
    prev_html="<a class=\"detail-link\" href=\"${prev_name}.html\">&#8592; ${prev_title}</a>"
  fi
  if [ "$idx" -lt "$((${#MANIFEST_ENTRIES[@]} - 1))" ]; then
    local next_src="${MANIFEST_ENTRIES[$((idx+1))]}"
    local next_title next_name
    next_title=$(extract_title "$DOCS_DIR/$next_src")
    next_name=$(flatten_name "$next_src")
    next_html="<a class=\"detail-link\" href=\"${next_name}.html\">${next_title} &#8594;</a>"
  fi

  if [ -z "$prev_html" ] && [ -z "$next_html" ]; then
    return
  fi

  cat <<HTML
        <hr>
        <div style="display: flex; justify-content: space-between; font-size: 0.9rem;">
          <span>${prev_html}</span>
          <span>${next_html}</span>
        </div>
HTML
}

template=$(cat "$SHELL_TPL")

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

for idx in "${!MANIFEST_ENTRIES[@]}"; do
  rel="${MANIFEST_ENTRIES[$idx]}"
  src="$DOCS_DIR/$rel"
  [ -f "$src" ] || { echo "missing source: $src" >&2; exit 1; }

  page_title=$(extract_title "$src")
  [ -n "$page_title" ] || { echo "no '= Title' line at top of $src" >&2; exit 1; }

  name=$(flatten_name "$rel")
  body_file="$tmpdir/$name.body.html"
  asciidoctor --no-header-footer -o "$body_file" "$src"
  body=$(cat "$body_file")
  nav=$(chapter_nav_html "$idx")

  # Bash 5.2+ treats a leading `&` in the replacement half of
  # ${var//pat/repl} as a back-reference to the matched pattern, so any
  # replacement text that can contain `&` (HTML entities like &lt;,
  # numeric references like &#8592;, or stray `&` in titles) must be
  # escaped to \& before being substituted in. The templated fields
  # below (body, chapter-nav, and free-text strings from the config)
  # all qualify, so escape them uniformly.
  esc() { printf '%s' "${1//&/\\&}"; }

  out="$template"
  out="${out//\{\{PAGE_TITLE\}\}/$(esc "$page_title")}"
  out="${out//\{\{SITE_NAME\}\}/$(esc "$SITE_NAME")}"
  out="${out//\{\{SITE_TAGLINE\}\}/$(esc "$SITE_TAGLINE")}"
  out="${out//\{\{PROJECT_NAME\}\}/$(esc "$PROJECT_NAME")}"
  out="${out//\{\{PROJECT_SLUG\}\}/$(esc "$PROJECT_SLUG")}"
  out="${out//\{\{HOME_HREF\}\}/$(esc "$HOME_HREF")}"
  out="${out//\{\{LOGO_HREF\}\}/$(esc "$LOGO_HREF")}"
  out="${out//\{\{CSS_HREF\}\}/$(esc "$CSS_HREF")}"
  out="${out//\{\{BACK_LINK_HREF\}\}/$(esc "$BACK_LINK_HREF")}"
  out="${out//\{\{BACK_LINK_TEXT\}\}/$(esc "$BACK_LINK_TEXT")}"
  out="${out//\{\{BODY\}\}/$(esc "$body")}"
  out="${out//\{\{CHAPTER_NAV\}\}/$(esc "$nav")}"

  out_file="$OUT_DIR_ABS/$name.html"
  printf '%s\n' "$out" > "$out_file"
  echo "built $name.html"
done

echo "wrote ${#MANIFEST_ENTRIES[@]} page(s) to $OUT_DIR_ABS"
