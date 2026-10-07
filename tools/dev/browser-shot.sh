# shellcheck shell=bash
#
# Finding a browser and taking a picture of a page with it, headless.
#
#   . tools/dev/browser-shot.sh     # defines find_chrome and shot
#
# Sourced by tools/dev/screenshots.sh and tools/dev/sync-pictures.sh. No shebang and no
# `set -euo pipefail`, for the reason tools/dist/common.sh gives: a sourced file must not change the
# caller's shell.
#
# The caller sets REPO to the repository root and PROFILE to a browser profile folder of the run's
# own, defines `note` and `warn`, and sets CHROME from `find_chrome` before the first `shot`.
#
#   KM_CHROME   the browser binary   (default: Chrome, then Edge)

find_chrome() {
  if [ -n "${KM_CHROME:-}" ]; then
    printf '%s' "$KM_CHROME"
    return 0
  fi
  for candidate in \
    "/c/Program Files/Google/Chrome/Application/chrome.exe" \
    "/c/Program Files (x86)/Google/Chrome/Application/chrome.exe" \
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
    "/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe" \
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"; do
    [ -x "$candidate" ] && {
      printf '%s' "$candidate"
      return 0
    }
  done
  for name in google-chrome chromium chromium-browser microsoft-edge; do
    command -v "$name" >/dev/null 2>&1 && {
      command -v "$name"
      return 0
    }
  done
  return 1
}

# shot <out.png> <css-width> <css-height> <scale> <url>
#
# Two of these flags each fix a specific way this silently produces nothing:
#
#   --user-data-dir  without it, an already-running Chrome adopts the invocation, and the headless
#                    process exits 0 having written no file at all. The commonest failure on Windows.
#   --run-all-compositor-stages-before-draw
#                    or the tab bar and star transitions are caught mid-animation.
#
# There is deliberately **no --blink-settings=preferredColorScheme flag**. One pinned to 0 answers
# a single surface: headless defaults to light, so a page leading with dark comes out in a palette
# the product does not lead with.
#
# The two browser surfaces disagree, and that is precisely why no flag belongs here. The remote is
# pinned light (see the head of km-remote-pages/static/app.css) and km-package-builder is pinned dark (the
# head of its own style.css), each for its own reason, so there is no one value this flag could take
# that would be right for both -- which was always the objection to it: setting a color scheme for
# every page from one place asserts they agree, and each page already knows its own. Nothing is lost
# by leaving it out, because the flag drives `prefers-color-scheme` and neither stylesheet has such a
# branch. Each declares `color-scheme` itself, which is what the UA-drawn scrollbars and `<select>`
# follow, so headless's own default reaches nothing either page cares about.
#
# Note the width floor: headless Chrome will not give a viewport narrower than 500 CSS px, so asking
# for 390 silently lays the page out at 500 and crops it, which looks like a broken stylesheet. The
# phone pictures are therefore taken at 500. That is not a meaningful infidelity here -- the only
# thing below km-remote-pages's 27rem breakpoint is the result count.
#
# `out` is made absolute before it is handed over: Chrome resolves a relative --screenshot path
# against something other than this shell's working directory and then writes nothing, silently,
# exiting 0. Every capture failed exactly once for this reason.
shot() {
  out="$1" w="$2" h="$3" scale="$4" url="$5"
  case "$out" in
    /* | ?:*) ;;
    *) out="$REPO/$out" ;;
  esac
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars \
    --window-size="$w,$h" --force-device-scale-factor="$scale" \
    --virtual-time-budget=4000 --run-all-compositor-stages-before-draw \
    --user-data-dir="$PROFILE" --no-first-run --no-default-browser-check \
    --screenshot="$out" "$url" >/dev/null 2>&1 || true
  if [ -s "$out" ]; then
    note "wrote $out"
  else
    warn "could not capture $url -> $out (capture it by hand, ${w}x${h})"
  fi
}
