#!/bin/sh
#
# The double-clickable way to remove KM Remote from this Mac.
#
# **Why this exists when uninstall.sh beside it does the same job.** The Finder will not open a
# `.sh` -- it has no handler for one, so a double-click reveals it in a text editor at best. It hands
# a `.command` to Terminal and runs it. That extension is the whole mechanism, and the name is the
# whole of the discoverability: somebody who opens this folder sees a file that says what it does.
#
# **It removes nothing itself.** uninstall.sh is the only thing that does, so there is no second copy
# of the rules about what belongs to this package and what belongs to the karaoke machine's.
#
# This file is a template: tools/platform/macos/installer-remote.sh substitutes the version and
# stages the result into the `docs` component as "Uninstall KM Remote.command".

set -eu

VERSION="@VERSION@"

# **First, and load-bearing.** Double-clicked, Terminal starts in the user's home directory, so
# `./uninstall.sh` would not resolve. $0 is the full path the Finder passed.
cd "$(dirname "$0")" || {
  echo "Could not find the folder this was run from." >&2
  exit 1
}

if [ ! -x ./uninstall.sh ]; then
  echo "uninstall.sh is not beside this file, so there is nothing to run." >&2
  echo "This should be in /usr/local/km-remote." >&2
  exit 1
fi

echo "KM Remote $VERSION"
echo
echo "This will remove the application and the folder listed below."
echo "It does NOT touch your copy of the song list or your favorites -- it says"
echo "where those are when it has finished."
echo

# **The list comes before the password prompt**, which is the reason this wrapper does two steps
# rather than going straight to sudo. `--dry-run` deliberately needs no root, so what is about to be
# removed can be shown to somebody who has not yet agreed to anything. Being asked for an
# administrator password by a window that has not said what it wants it for is exactly the thing
# people are right to refuse.
./uninstall.sh --dry-run
echo

# Not a tty: something other than a double-click is running this -- a script, a pipe, a CI step. Do
# not hang on `read`, and do not remove anything on an answer nobody gave. Name the direct command
# instead, since that is what a caller in that position actually wants.
if [ ! -t 0 ]; then
  echo "Nothing was removed: this is not an interactive terminal, so there was nobody to ask."
  echo
  echo "To remove it without being asked:"
  echo "    sudo /usr/local/km-remote/uninstall.sh"
  exit 0
fi

# Defaults to no. Anything but an explicit yes leaves the Mac alone.
printf 'Remove all of that? Type yes to confirm: '
read -r answer || answer=""
case "$answer" in
  [Yy]|[Yy][Ee][Ss]) ;;
  *)
    echo
    echo "Nothing was removed."
    exit 0
    ;;
esac

echo
echo "macOS will ask for your password now. That is to remove files from /Applications"
echo "and /usr/local, which is where the installer put them."
echo

# `exec`, so the password prompt and everything uninstall.sh prints -- including the block saying
# where your song list is -- land in the window that is already open and already showing the list
# they refer to.
exec sudo ./uninstall.sh
