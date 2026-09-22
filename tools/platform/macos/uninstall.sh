#!/bin/sh
#
# Removes what the KaraokeMachine setup package put on this Mac, and nothing else.
#
#   sudo /usr/local/karaokemachine/uninstall.sh
#   sudo /usr/local/karaokemachine/uninstall.sh --dry-run    # say what would go, remove nothing
#
# **It never touches your songs, settings or catalog.** Those live under
# ~/Library/Application Support and are yours to delete or not; the last thing this prints is where
# they are. The same promise the Windows uninstaller makes, in the same words -- and the words come
# from tools/platform/macos/pkg/data-locations.txt, which is also what the installer's closing pane
# says, so the two cannot drift apart.
#
# This file is a template: tools/platform/macos/installer.sh substitutes the version and that block of text,
# and stages the result into the `docs` component. It is installed unconditionally, because the one
# way to take a thing off again must not be something you could decline.
#
# **The two placeholders are deliberately not spelled out in this comment**, and that is not
# fastidiousness -- the substitution is a plain string replacement over the whole file, so naming the
# block's marker here dropped ten lines of prose into the middle of a `#` comment and the installed
# script died on `They: command not found`. Found by running it.
#
# `sh` rather than `bash`, and no `set -o pipefail`: this runs on somebody else's Mac, as root, and
# the smallest shell that can do the job is the right one.

set -eu

VERSION="@VERSION@"
PREFIX=/usr/local/karaokemachine
BINDIR=/usr/local/bin

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

# Only ever removes a path this installer created. `--dry-run` prints and does nothing, which is the
# same shape tools/dist/clean.sh uses and for the same reason: the first thing anybody wants from a
# remover is to see the list first.
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

say "KaraokeMachine $VERSION -- removing what the setup package installed."
say ""

# -- the applications -------------------------------------------------------------------------------
#
# **Matched by the identifier inside the bundle, never by the name on it** -- which is the rule the
# commands below already follow, one directory over, for the same reason. A name is what a release is
# free to change; `CFBundleIdentifier` is what macOS uses to mean "this application", and it survives
# a rename because changing it would make the Finder, Launch Services and every receipt treat the
# result as a different program.
#
# **A list of literal paths can only name the bundles of the version it shipped in**, so an
# uninstaller that ships *after* a rename can never reach what the one *before* it left behind. That
# is not hypothetical: two installs a day apart, either side of the rename that turned
# `KaraokeMachine Assets.app` into `KM Admin.app` and `KaraokeMachine Package Builder.app` into
# `KM Package Builder.app`, left three pre-rename bundles standing here with nothing able to remove
# them. `tools/dist/clean.sh` says in its own comment that telling a fossil from a bundle needs
# something better than a list of dead names; in /Applications that something is the identifier.
#
# **The receipts cannot do this job**, which is worth writing down because it is the obvious first
# idea. `pkgutil` records only the payload of the install that wrote it, so a rename that keeps its
# component id -- `builder` and `remote` both did -- updates that receipt to the new name and leaves
# no trace of the old one. It sees only the case where the id changed as well.
#
# `com.karaokemachine.` is the bundle namespace and the scan below keys on it. Receipts are a
# different namespace: `com.rrgmc.` holds this product's receipts and the video downloader's alike --
# a separate product from a separate repository that installs beside these -- so a receipt is ours by
# its second segment, `karaokemachine` or `km-remote`, and never by the prefix.
# **The four current names are added to the same list, and the list is deduplicated.** They are what
# the round trip checks and the one thing this script can state without asking the system a question
# -- a bundle whose `Info.plist` will not parse is invisible to the scan above and is still ours.
# Deduplicating is what keeps `--dry-run` honest: nothing is removed under it, so a bundle found both
# ways would otherwise be printed twice and counted twice.
#
# Newline-delimited because `KM Package Builder.app` has spaces in it and a path cannot contain a
# newline. `sort -u` rather than `sort` alone: the order this prints in does not matter, and being
# able to say each bundle once does.
APP_ID_PREFIX=com.karaokemachine.

app_candidates() {
  for app in /Applications/*.app; do
    [ -d "$app" ] || continue
    id=$(plutil -extract CFBundleIdentifier raw -o - "$app/Contents/Info.plist" 2>/dev/null) || continue
    case "$id" in "$APP_ID_PREFIX"*) printf '%s\n' "$app" ;; esac
  done
  printf '%s\n' \
    "/Applications/Karaoke Machine.app" \
    "/Applications/KM Stream.app" \
    "/Applications/KM Package Builder.app" \
    "/Applications/KM Simple Package.app" \
    "/Applications/KM Remote.app" \
    "/Applications/KM Admin.app"
}

# **Read from a here-document rather than a pipe**, so the loop runs in this shell. `take` counts
# what it removes, and a `... | while` counts in a subshell -- the list printed above would be right
# and the total underneath it would be short by every application in it. The same reason
# tools/dev/worktree.sh gives for the same shape.

# **One bundle may belong to the other carrier, and this is where that is asked.** The remote has a
# setup program of its own, and both place `/Applications/KM Remote.app` -- the identifier names the
# product rather than the carrier, and two bundles may not share one. The receipts stay separate, so
# each package knows what it archived; what neither can do is avoid reaching the same application. So
# a remote installed by its own package is left where it is, and its own uninstaller takes it.
REMOTE_APP_ID=com.karaokemachine.remote
REMOTE_RECEIPT=com.rrgmc.km-remote.app

remote_is_theirs() { # <path to a bundle>
  pkgutil --pkgs 2>/dev/null | grep -qx "$REMOTE_RECEIPT" || return 1
  id=$(plutil -extract CFBundleIdentifier raw -o - "$1/Contents/Info.plist" 2>/dev/null) || return 1
  [ "$id" = "$REMOTE_APP_ID" ]
}

while IFS= read -r app; do
  [ -n "$app" ] || continue
  if [ -d "$app" ] && remote_is_theirs "$app"; then
    say "  kept          $app  (installed by the KM Remote package, which removes it)"
    continue
  fi
  take "$app" "an application"
done <<EOF
$(app_candidates | sort -u)
EOF

# -- the commands -----------------------------------------------------------------------------------
#
# **Matched by what they point at, never by name alone.** /usr/local/bin is a directory this package
# did not create and does not own -- on an Intel Mac it is Homebrew's -- so removing six names out
# of it because they happen to be ours-sounding is exactly the damage an uninstaller must not do.
# A symlink qualifies if it resolves into $PREFIX; the machine's entry is a shim script rather than a
# symlink (see the postinstall for why) and qualifies if it carries the marker line the shim is
# written with.
ours() { # <path in /usr/local/bin> -> true if this package put it there
  if [ -L "$1" ]; then
    target="$(readlink "$1")"
    # **A relative target is resolved against the link's own directory**, and that is not
    # hypothetical tidiness: the postinstall writes absolute targets, so the plain comparison
    # worked -- and a test that made the same links relatively reported every one of them as
    # somebody else's, which would have left six entries in /usr/local/bin behind. Whether the
    # link is relative is not a thing this script should depend on.
    case "$target" in
      /*) ;;
      *)  target="$(cd "$(dirname "$1")" 2>/dev/null && cd "$(dirname "$target")" 2>/dev/null \
                    && pwd)/$(basename "$target")" ;;
    esac
    case "$target" in
      "$PREFIX"/*) return 0 ;;
      *) return 1 ;;
    esac
  fi
  [ -f "$1" ] && grep -q 'Installed by the KaraokeMachine setup package' "$1" 2>/dev/null
}

# **Every entry in the directory is offered to `ours`, rather than a list of names being looked up.**
# `ours` is already the whole of the decision -- a symlink into $PREFIX, or the shim carrying its
# marker line -- so a name list adds nothing to the safety and takes away the only thing this sweep
# needs to survive: a command that a later version stopped shipping under that name. The dangling
# `km-assets` and `wallpaper-pack` links a rename left behind are exactly what a fixed list walks
# past, and they point into $PREFIX, so `ours` claims them without being told they exist.
#
# `[ -e ] || [ -L ]` because a link whose target has already gone is still ours to remove, and `-e`
# alone says no about it.
for entry in "$BINDIR"/*; do
  { [ -e "$entry" ] || [ -L "$entry" ]; } || continue
  if ours "$entry"; then
    take "$entry" "a command"
  fi
done

# **...and a word about the ones we expected and did not get.** Silence above means "not ours", which
# for a name this package does ship has to be said out loud: it is how somebody finds out that
# /usr/local/bin/km-pack is Homebrew's, or somebody else's build, rather than assuming the uninstall
# missed it. Anything taken above no longer exists, so this only ever speaks about what stayed.
for name in karaokemachine km-pack km-lyrics km-wallpaper-pack \
            km-package-builder km-package-simple km-remote km-admin
do
  entry="$BINDIR/$name"
  { [ -e "$entry" ] || [ -L "$entry" ]; } || continue
  # `ours` again rather than "does it still exist": under --dry-run nothing was actually removed, so
  # existence cannot distinguish what this script claimed from what it declined, and every name would
  # be reported twice -- once as going, once as kept.
  ours "$entry" || say "  kept          $entry  (not ours; something else put it there)"
done

# -- the folder -------------------------------------------------------------------------------------
#
# Including this script. Removing a running `sh` script is safe: the shell has already read it.
take "$PREFIX" "the tools, their libraries and this script"

# Only if they are empty, and never forced. /usr/local/bin may well be Homebrew's.
if [ "$DRY" -eq 0 ]; then
  rmdir "$BINDIR" 2>/dev/null || true
  rmdir /usr/local 2>/dev/null || true
fi

# -- the receipts -------------------------------------------------------------------------------------
#
# So `pkgutil --pkgs` stops listing something that is no longer here. A component nobody selected has
# no receipt, hence the `|| true` on each.
# **Forgotten by prefix, for the reason the applications are taken by identifier**: a component this
# package wrote under an id a later version stopped using is still a receipt, and six literal ids can
# only ever name the components of the version they were typed in. The `assets` component outlived
# its own bundle exactly that way, and its receipt sat beside the live `admin` one with nothing
# listing it.
if [ "$DRY" -eq 0 ]; then
  for id in $(pkgutil --pkgs 2>/dev/null | grep '^com\.rrgmc\.karaokemachine\.' || true); do
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
