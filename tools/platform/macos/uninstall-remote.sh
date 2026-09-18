#!/bin/sh
#
# Removes what the KM Remote setup package put on this Mac, and nothing else.
#
#   sudo /usr/local/km-remote/uninstall.sh
#   sudo /usr/local/km-remote/uninstall.sh --dry-run    # say what would go, remove nothing
#
# **It never touches your copy of the song list or your favorites.** Those live under
# ~/Library/Application Support and are yours to delete or not; the last thing this prints is where
# they are. The words come from tools/platform/macos/pkg-remote/data-locations.txt, which is also
# what the installer's closing pane says, so the two cannot drift apart.
#
# This file is a template: tools/platform/macos/installer-remote.sh substitutes the version and that
# block of text, and stages the result into the `docs` component. It is installed unconditionally,
# because the one way to take a thing off again must not be something you could decline.
#
# **The two placeholders are deliberately not spelled out in this comment.** The substitution is a
# plain string replacement over the whole file, so naming a marker here drops the replacement into
# the middle of a `#` comment, and the result is still valid shell -- the fault shows only when
# somebody runs the installed script. The installer counts the mentions for that reason.
#
# **It touches /usr/local/bin not at all.** The package that installed this put no command there:
# it carries the application alone. A sweep of that directory would therefore be a sweep looking for
# something this package never wrote.
#
# `sh` rather than `bash`, and no `set -o pipefail`: this runs on somebody else's Mac, as root, and
# the smallest shell that can do the job is the right one.

set -eu

VERSION="@VERSION@"
PREFIX=/usr/local/km-remote
APP="/Applications/KM Remote.app"
APP_ID=com.karaokemachine.remote
RECEIPT_PREFIX='^com\.rrgmc\.km-remote\.'
OTHER_RECEIPT=com.rrgmc.karaokemachine.remote

DRY=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY=1 ;;
    -h|--help) echo "usage: sudo $0 [--dry-run]"; exit 0 ;;
    *) echo "uninstall: unknown option $arg" >&2; exit 2 ;;
  esac
done

if [ "$DRY" -eq 0 ] && [ "$(id -u)" != 0 ]; then
  echo "uninstall: this removes files from /Applications and /usr/local, so it needs sudo." >&2
  echo "           sudo $0" >&2
  exit 1
fi

REMOVED=0

say() { printf '%s\n' "$*"; }

# Only ever removes a path this installer created. `--dry-run` prints and does nothing: the first
# thing anybody wants from a remover is to see the list first.
take() { # <path> <what it is, for the line printed>
  [ -e "$1" ] || [ -L "$1" ] || return 0
  if [ "$DRY" -eq 1 ]; then
    say "  would remove  $1  ($2)"
  else
    rm -rf "$1"
    say "  removed       $1  ($2)"
  fi
  REMOVED=$((REMOVED + 1))
}

say "KM Remote $VERSION -- removing what the setup package installed."
say ""

# -- the application ---------------------------------------------------------------------------------
#
# **Matched by the identifier inside the bundle, never by the name on it.** A name is what a release
# is free to change; `CFBundleIdentifier` is what macOS means by "this application", and it survives
# a rename because changing it would make the Finder, Launch Services and every receipt treat the
# result as a different program. The literal path is checked as well, because a bundle whose
# Info.plist will not parse is invisible to the scan and is still ours.
#
# **And it asks whose it is, which the all-in-one's uninstaller does not have to.** Both carriers
# place this same bundle at this same path, because the identifier names the product rather than the
# carrier and two bundles may not share one. The receipts stay separate, so each package knows what
# it archived -- but an uninstaller that took the application while the other carrier's receipt still
# claims it would leave that install pointing at nothing. So the karaoke machine's own receipt is
# what decides, and this stops at its own folder instead.
app_is_shared() {
  pkgutil --pkgs 2>/dev/null | grep -qx "$OTHER_RECEIPT"
}

app_path() {
  for candidate in /Applications/*.app; do
    [ -d "$candidate" ] || continue
    id=$(plutil -extract CFBundleIdentifier raw -o - "$candidate/Contents/Info.plist" 2>/dev/null) || continue
    [ "$id" = "$APP_ID" ] || continue
    printf '%s\n' "$candidate"
    return 0
  done
  [ -d "$APP" ] && printf '%s\n' "$APP"
}

found="$(app_path || true)"
if [ -z "$found" ]; then
  :
elif app_is_shared; then
  say "  kept          $found  (the KaraokeMachine install on this Mac carries it too)"
else
  take "$found" "the application"
fi

# -- this folder -------------------------------------------------------------------------------------
#
# The terms, the README and this script. Removed whole, and last among the files, because the script
# doing the removing is inside it: `rm -rf` on a running `sh` script is safe -- the shell has already
# read it -- and anything after this point must not be read from disk.
take "$PREFIX" "the documents and this uninstaller"

# `rmdir` and not `rm -rf`: /usr/local is not ours and may hold Homebrew's entire installation. This
# removes it only when this package was the last thing in it, which is the case on a Mac that never
# had Homebrew.
if [ "$DRY" -eq 0 ] && [ -d /usr/local ]; then
  rmdir /usr/local 2>/dev/null || true
fi

# -- the receipts -------------------------------------------------------------------------------------
#
# So `pkgutil --pkgs` stops listing something that is no longer here. **Forgotten by prefix, for the
# reason the application is taken by identifier**: a component this package wrote under an id a later
# version stopped using is still a receipt, and two literal ids can only ever name the components of
# the version they were typed in. The prefix is this carrier's own and reaches nothing of the
# all-in-one's, which lives under `com.rrgmc.karaokemachine.`.
if [ "$DRY" -eq 0 ]; then
  for id in $(pkgutil --pkgs 2>/dev/null | grep "$RECEIPT_PREFIX" || true); do
    pkgutil --forget "$id" >/dev/null 2>&1 || true
  done
fi

# -- what is left, which is the part people are actually asking about ---------------------------------

say ""
if [ "$DRY" -eq 1 ]; then
  say "$REMOVED item(s) would be removed. Nothing was."
else
  say "$REMOVED item(s) removed."
fi
say ""
cat <<'KEPT'
@DATA_LOCATIONS@
KEPT
