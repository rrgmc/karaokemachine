#!/usr/bin/env bash
#
# Assembles the landing page into one folder, ready to publish.
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
# ** The screenshots are staged, never committed twice. ** `site/index.html` asks for `images/*.png`
# and this is what puts them there, out of `docs/images/` -- which `tools/dev/screenshots.sh`
# regenerates and which the README links directly. A second tracked copy would be 1.3 MB of PNG that
# goes stale the first time the pictures are retaken, silently, in the one place nobody looks. The
# cost is that opening `site/index.html` straight from the checkout shows broken images; `site/README.md`
# says so in its first paragraph, and this script is the answer.
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

# -- arguments -------------------------------------------------------------------------------------

while [ $# -gt 0 ]; do
  case "$1" in
    -v|--verbose)  DIST_VERBOSE=1 ;;
    --open)        OPEN=1 ;;
    -h|--help)     sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)             echo "$DIST_SCRIPT: unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

# -- what has to be here ---------------------------------------------------------------------------

for f in site/index.html site/style.css icon/icon-32.png icon/icon-512.png; do
  [ -f "$f" ] || { echo "$DIST_SCRIPT: missing $f" >&2; exit 1; }
done

# The one mistake this arrangement exists to prevent, caught where it is made rather than months
# later when the two copies have drifted.
if [ -e site/images ]; then
  echo "$DIST_SCRIPT: site/images exists, and the screenshots are staged rather than committed." >&2
  echo "  They live in docs/images/ because README.md shows them too. Remove site/images." >&2
  exit 1
fi

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

cp site/index.html site/style.css "$OUT/"
dist_detail "page      site/index.html + style.css"

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
if grep -Eqn '(src|href)="/' site/index.html site/style.css; then
  echo "$DIST_SCRIPT: an absolute path -- this is served under /karaokemachine/, so it would 404:" >&2
  grep -En '(src|href)="/' site/index.html site/style.css >&2
  exit 1
fi

# ** No external request, ever. ** No web font, no CDN, no analytics. This is the check rather than
# the promise: a product whose whole claim is that it works on a network with no internet on it
# should not have a home page that fetches a stylesheet from somebody else, and that is exactly the
# sort of thing that arrives inside a copy-pasted snippet. Ordinary `<a href="https://…">` links are
# navigation rather than a request and are left alone -- this looks for things the browser *fetches*.
if grep -Eqn 'src="https?:|<link[^>]+href="https?:|url\(\s*["'"'"']?https?:' site/index.html site/style.css; then
  echo "$DIST_SCRIPT: the page would fetch something from another server:" >&2
  grep -En 'src="https?:|<link[^>]+href="https?:|url\(\s*["'"'"']?https?:' site/index.html site/style.css >&2
  exit 1
fi

# -- every link has to land ------------------------------------------------------------------------

# The one failure this arrangement can actually produce, and it is invisible until somebody loads the
# page: a relative path in `index.html` naming a file the staging does not put there. Renaming a
# screenshot is all it takes. Absolute URLs are somebody else's business and are skipped.
dist_step "checking relative links"
MISSING=0
while IFS= read -r ref; do
  [ -n "$ref" ] || continue
  case "$ref" in
    http:*|https:*|//*|"#"*|mailto:*|data:*) continue ;;
  esac
  # Anchors and query strings are not part of the path on disk.
  path="${ref%%#*}"; path="${path%%\?*}"
  [ -n "$path" ] || continue
  if [ ! -e "$OUT/$path" ]; then
    echo "$DIST_SCRIPT: index.html asks for '$path', which is not in $OUT" >&2
    MISSING=$((MISSING + 1))
  else
    dist_detail "link      $path"
  fi
done < <(grep -o -E '(src|href)="[^"]*"' "$OUT/index.html" | sed -E 's/^[a-z]+="//; s/"$//' | sort -u)

if [ "$MISSING" -gt 0 ]; then
  echo "$DIST_SCRIPT: $MISSING broken relative link(s); the page would publish with holes in it" >&2
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
