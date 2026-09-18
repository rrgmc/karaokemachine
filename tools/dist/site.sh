#!/usr/bin/env bash
#
# Assembles the landing pages into one folder, ready to publish.
#
#   tools/dist/site.sh                 # stage into dist/site
#   tools/dist/site.sh --open          # ...and open it in a browser
#   tools/dist/site.sh -v              # say what went where
#
# Published to https://rrgmc.github.io/karaokemachine/ by .github/workflows/pages.yml, which runs
# exactly this script and uploads what it produces. **That is the point of the script existing rather
# than the workflow spelling out a few `cp` lines**: there is one code path, so what you preview
# locally is what gets published, and a page that works here cannot break there.
#
# ** One page per language, and one of everything else. ** English is served at the root and every
# other language one segment down under its own tag, so `site/pt-BR/index.html` reaches the shared
# stylesheet and the shared pictures as `../style.css` and `../images/`. The pages are checked
# against each other as well as on their own: the same sections, the same pictures and the same
# links out, which is the drift a grep can see. Prose is not checked, and a page that fell behind in
# words is found by reading it.
#
# ** The screenshots are staged, never committed twice. ** The English page asks for `images/*.png`
# and the Portuguese one for `../images/*.png`, and this is what puts them there, out of
# `docs/images/` -- which `tools/dev/screenshots.sh` regenerates and which the README links directly.
# A second tracked copy would be 1.3 MB of PNG that goes stale the first time the pictures are
# retaken, silently, in the one place nobody looks. The cost is that opening either page straight
# from the checkout shows broken images; `site/README.md` says so in its first paragraph, and this
# script is the answer.
#
# ** Nothing here needs a toolchain. ** No cargo, no rustc, not even for the platform check below --
# `dist_platform` asks `rustc -vV` and is therefore deliberately not used. A checkout, a copy and a
# browser is the whole dependency list, which is what lets the workflow be the cheapest in the
# repository: no Rust setup step, no cache, well under a minute.
#
# ** The output is `dist/site/`, and that is a deliberate exception ** to the `dist/<app>/<platform>/`
# layout `dist_dir` enforces and that "Where releases go" states as a rule. A web page has no
# platform, and inventing one -- `dist/site/web/` -- would be a worse lie than the exception is a
# breach. It is still under `dist/`, which is what the rule is really protecting: gitignored, and
# swept by the same `clean` as every other carrier.

set -euo pipefail

cd "$(dirname "$0")/../.."

. tools/dist/common.sh
DIST_SCRIPT=dist-site
dist_assert_root

OUT=dist/site
OPEN=0

# The page, once per language, and the tag each one declares. A path here is relative to `site/` and
# to the staged folder alike, so one name serves the source, the copy and the address a reader sees.
# English is first because it is the source language and what `x-default` names.
PAGES=(index.html pt-BR/index.html)
LANGS=(en pt-BR)

# -- arguments -------------------------------------------------------------------------------------

while [ $# -gt 0 ]; do
  case "$1" in
    -v|--verbose)  DIST_VERBOSE=1 ;;
    --open)        OPEN=1 ;;
    # The header down to the first line that is not a comment. A hand-counted range truncates the
    # help the moment a paragraph is added to the header, and says nothing when it does.
    -h|--help)     awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"; exit 0 ;;
    *)             echo "$DIST_SCRIPT: unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

# -- what has to be here ---------------------------------------------------------------------------

for f in "${PAGES[@]/#/site/}" site/style.css icon/icon-32.png icon/icon-512.png; do
  [ -f "$f" ] || { echo "$DIST_SCRIPT: missing $f" >&2; exit 1; }
done

# The one mistake this arrangement exists to prevent, caught where it is made rather than months
# later when the two copies have drifted. Once per language folder as well as at the root: a
# screenshot committed beside a translated page is the same mistake one directory down, where the
# pictures are a step further away and copying them in looks like the obvious fix.
for page in "${PAGES[@]}"; do
  d="site/$(dirname "$page")/images"
  d="${d#site/./}"; case "$d" in images) d="site/images" ;; esac
  [ -e "$d" ] || continue
  echo "$DIST_SCRIPT: $d exists, and the screenshots are staged rather than committed." >&2
  echo "  They live in docs/images/ because README.md shows them too. Remove $d." >&2
  exit 1
done

# A page folder nothing here stages is a language that publishes nowhere, and no other check sees it:
# the page is never copied, so it never names a missing file and never drifts from anything.
while IFS= read -r d; do
  [ -n "$d" ] || continue
  case " ${PAGES[*]} " in *" $d/index.html "*) continue ;; esac
  echo "$DIST_SCRIPT: site/$d holds a page nothing here stages." >&2
  echo "  Name it in PAGES and LANGS at the top of this script, or it reaches no reader." >&2
  exit 1
done < <(find site -mindepth 1 -maxdepth 1 -type d | sed 's|^site/||' | LC_ALL=C sort)

# Not a formality: the pictures are the page. An empty `docs/images` stages a page of broken frames
# and says nothing, which is the failure this whole arrangement is otherwise vulnerable to.
SHOTS=$(find docs/images -maxdepth 1 -name '*.png' | wc -l | tr -d ' ')
if [ "$SHOTS" -eq 0 ]; then
  echo "$DIST_SCRIPT: no screenshots in docs/images -- run tools/dev/screenshots.sh" >&2
  exit 1
fi

# -- stage -----------------------------------------------------------------------------------------

dist_step "staging the site into $OUT"
dist_clear "$OUT"

for page in "${PAGES[@]}"; do
  mkdir -p "$OUT/$(dirname "$page")"
  cp "site/$page" "$OUT/$page"
  dist_detail "page      site/$page"
done
cp site/style.css "$OUT/"
dist_detail "style     site/style.css, shared by every page"

# `site/README.md` is for somebody reading the repository, not for the web.
mkdir -p "$OUT/images"
cp docs/images/*.png "$OUT/images/"
dist_detail "images    $SHOTS from docs/images"

# The favicon and the Open Graph / apple-touch image, both out of the generated icon set. 32 is the
# size `km-api` and `km-remote-pages` already serve as their own favicon, so a tab from the machine
# and a tab from the website carry the same mark. The 512 is also the mark the hero shows, and these
# two are the only icons that land here -- so naming any other one fails the link check below.
cp icon/icon-32.png "$OUT/favicon.png"
cp icon/icon-512.png "$OUT/icon-512.png"
dist_detail "icons     favicon.png + icon-512.png from icon/"

# Not required by the artifact-upload path Actions uses, and free. It costs one empty file and it
# removes the whole class of surprise where somebody switches the repository to "deploy from a
# branch" later and Jekyll starts eating files whose names begin with an underscore.
: > "$OUT/.nojekyll"

# -- two things that fail only after publishing ------------------------------------------------------

# ** An absolute path 404s here and nowhere else. ** The page is served at
# https://rrgmc.github.io/karaokemachine/ -- a *project* page, one path segment down -- so
# `/images/x.png` resolves to `rrgmc.github.io/images/x.png` and is not this site at all. `//cdn…` is
# caught by the same pattern and is worse, being somebody else's server. Every path in the page is
# relative, which is also why nothing here reads `configure-pages`' `base_url`: the whole class of
# problem a Jekyll `baseurl` exists to solve is avoided by not having absolute paths.
if grep -Eqn '(src|href)="/' "${PAGES[@]/#/site/}" site/style.css; then
  echo "$DIST_SCRIPT: an absolute path -- this is served under /karaokemachine/, so it would 404:" >&2
  grep -En '(src|href)="/' "${PAGES[@]/#/site/}" site/style.css >&2
  exit 1
fi

# ** No external request, ever. ** No web font, no CDN, no analytics. This is the check rather than
# the promise: a product whose whole claim is that it works on a network with no internet on it
# should not have a home page that fetches a stylesheet from somebody else, and that is exactly the
# sort of thing that arrives inside a copy-pasted snippet. Ordinary `<a href="https://…">` links are
# navigation rather than a request and are left alone -- this looks for things the browser *fetches*.
#
# `<link rel="alternate">` names a sibling page relatively and is skipped by the pattern already:
# what it looks for is a `<link>` whose href is `http`, and every one of those is somebody else's.
if grep -Eqn 'src="https?:|<link[^>]+href="https?:|url\(\s*["'"'"']?https?:' "${PAGES[@]/#/site/}" site/style.css; then
  echo "$DIST_SCRIPT: the page would fetch something from another server:" >&2
  grep -En 'src="https?:|<link[^>]+href="https?:|url\(\s*["'"'"']?https?:' "${PAGES[@]/#/site/}" site/style.css >&2
  exit 1
fi

# -- every link has to land ------------------------------------------------------------------------

# The one failure this arrangement can actually produce, and it is invisible until somebody loads the
# page: a relative path in a page naming a file the staging does not put there. Renaming a screenshot
# is all it takes. Absolute URLs are somebody else's business and are skipped.
#
# **Each reference is resolved against its own page's directory**, so `../style.css` from a language
# one segment down lands on the one staged stylesheet. Resolving everything against `$OUT` instead
# would pass a page that names a file above the artifact root -- which is the fault that makes a
# `../docs/images/` path 404 in production only.
dist_step "checking relative links"
MISSING=0

check_links() {
  local page="$1" ref path base target
  base="$(dirname "$OUT/$page")"
  while IFS= read -r ref; do
    [ -n "$ref" ] || continue
    case "$ref" in
      http:*|https:*|//*|"#"*|mailto:*|data:*) continue ;;
    esac
    # Anchors and query strings are not part of the path on disk.
    path="${ref%%#*}"; path="${path%%\?*}"
    [ -n "$path" ] || continue
    # What a static host serves from a directory is its `index.html`, so that is what a reference
    # ending in `/` has to reach. `[ -e ]` on the directory alone is satisfied by an empty one, which
    # is exactly the hole a language link would fall through.
    target="$base/$path"
    [ -d "$target" ] && target="${target%/}/index.html"
    if [ ! -e "$target" ]; then
      echo "$DIST_SCRIPT: $page asks for '$path', which is not in $OUT" >&2
      MISSING=$((MISSING + 1))
    else
      dist_detail "link      $page -> $path"
    fi
  done < <(grep -o -E '(src|href)="[^"]*"' "$OUT/$page" | sed -E 's/^[a-z]+="//; s/"$//' | sort -u)
}

for page in "${PAGES[@]}"; do
  check_links "$page"
done

if [ "$MISSING" -gt 0 ]; then
  echo "$DIST_SCRIPT: $MISSING broken relative link(s); the page would publish with holes in it" >&2
  exit 1
fi

# -- the pages have to say the same thing ------------------------------------------------------------

# **Structure is compared and prose is not.** A page per language is the same page twice, and what
# reaches one and not the other -- a section, a picture, a link out -- has a shape a grep can see. A
# paragraph that fell behind in words does not, and is found by reading it. This is what stands in
# for the catalog parity tests every translated crate owes, in a script that may not compile anything.
dist_step "checking the pages against each other"

# `../images/x.png` and `images/x.png` name one staged file, so the step up comes off before the two
# sets meet.
page_set() {
  case "$2" in
    id)    grep -o -E 'id="[^"]*"' "$OUT/$1" ;;
    image) grep -o -E 'src="[^"]*"' "$OUT/$1" | sed -E 's|"\.\./|"|' ;;
    link)  grep -o -E 'href="https://[^"]*"' "$OUT/$1" ;;
  esac | sort -u
}

# Every language declares its own tag, names every language including itself, and says which one a
# reader who asked for neither is served. A page in Portuguese declaring `lang="en"` lies to every
# reader that believes it.
page_langs() {
  grep -o -E '<link rel="alternate" hreflang="[^"]*"' "$OUT/$1" |  sed -E 's/.*hreflang="//; s/"$//' | sort -u
}

WANT_LANGS=$(printf '%s\n' "${LANGS[@]}" x-default | sort -u)
DRIFT=0

for i in "${!PAGES[@]}"; do
  page="${PAGES[$i]}"
  lang="${LANGS[$i]}"

  if ! grep -q "^<html lang=\"$lang\">" "$OUT/$page"; then
    echo "$DIST_SCRIPT: $page has to declare <html lang=\"$lang\">" >&2
    DRIFT=$((DRIFT + 1))
  fi

  if ! diff_out=$(diff <(echo "$WANT_LANGS") <(page_langs "$page")); then
    echo "$DIST_SCRIPT: $page does not name every language in a <link rel=\"alternate\">:" >&2
    echo "$diff_out" | sed -e 's|^<|  wanted |' -e "s|^>|  $page has |" >&2
    DRIFT=$((DRIFT + 1))
  fi

  # The link a reader clicks, which is not the same thing as the `<link>` a search engine reads. It
  # names the other language in that language, so the one person who needs it can read it.
  for other in "${LANGS[@]}"; do
    [ "$other" = "$lang" ] && continue
    if ! grep -q "hreflang=\"$other\" lang=\"$other\"" "$OUT/$page"; then
      echo "$DIST_SCRIPT: $page offers the reader no way to reach $other" >&2
      DRIFT=$((DRIFT + 1))
    fi
  done

  # Against the source language, which is where a section or a picture is added first.
  [ "$i" -eq 0 ] && continue
  for what in id image link; do
    if ! diff_out=$(diff <(page_set "${PAGES[0]}" "$what") <(page_set "$page" "$what")); then
      echo "$DIST_SCRIPT: ${PAGES[0]} and $page do not carry the same ${what}s:" >&2
      echo "$diff_out" | sed -e "s|^<|  ${PAGES[0]} |" -e "s|^>|  $page |" >&2
      DRIFT=$((DRIFT + 1))
    fi
  done
done

if [ "$DRIFT" -gt 0 ]; then
  echo "$DIST_SCRIPT: the pages have drifted apart in $DRIFT way(s)" >&2
  exit 1
fi

# -- report ----------------------------------------------------------------------------------------

FILES=$(find "$OUT" -type f | wc -l | tr -d ' ')
printf '\n%s\n' "staged $FILES files ($(( $(dist_bytes "$OUT") / 1024 )) KiB) in $OUT"

# `uname` rather than `dist_platform`, which shells out to rustc -- see the header. A machine with no
# Rust on it can still preview the page, and the CI runner has none by design.
open_it() {
  case "$(uname -s)" in
    Darwin)             open "$OUT/index.html" ;;
    MINGW*|MSYS*|CYGWIN*) start "" "$(cygpath -w "$OUT/index.html")" ;;
    *)                  xdg-open "$OUT/index.html" ;;
  esac
}

if [ "$OPEN" -eq 1 ]; then
  open_it >/dev/null 2>&1 || echo "$DIST_SCRIPT: could not open a browser; the page is at $OUT/index.html"
else
  echo "now:  open $OUT/index.html"
fi
