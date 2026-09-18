#!/usr/bin/env bash
#
# Deploys the machine to a Debian box over ssh, and turns that box into an appliance: the service
# enabled, the screen driven from a bare virtual terminal, and the whole thing coming back by itself
# after a power cut.
#
#   tools/platform/linux/deploy.sh user@box                     # build, copy, install, enable, restart
#   tools/platform/linux/deploy.sh user@box --no-build          # use what is already in dist/karaokemachine/linux
#   tools/platform/linux/deploy.sh user@box --no-video          # the plain build, from dist/.../linux/no-video
#   tools/platform/linux/deploy.sh user@box --songs ./packages  # also send .kmpkg files and register them
#   tools/platform/linux/deploy.sh user@box --port 2222
#   tools/platform/linux/deploy.sh user@box --identity ~/.ssh/karaoke
#
# `user@box` is the *sudo-enabled* account, not `karaoke`. The karaoke account is created by the
# package's postinst, has no password, and is the one the service runs as; nothing ever logs in as
# it.
#
# `--songs DIR` sends every `*.kmpkg` in DIR. A package is one file -- media included -- so there is
# nothing beside it that can be left behind, and nothing to keep in step.
#
# Two password prompts per deploy -- one for the copy, one for everything else -- which is why all
# the remote work happens in a single ssh invocation rather than five. Install an ssh key and there
# are none; this prints the one-line incantation if it looks like you have not.
#
# Requirements here: Docker (for the build, unless --no-build), ssh and scp. Requirements there:
# Debian 13 or newer, systemd, and a graphics device with a kernel mode-setting driver.

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/dist/common.sh
DIST_SCRIPT=deploy

HOST=""
PORT=""
IDENTITY=""
SONGS=""
BUILD=1
VIDEO=1

while [ $# -gt 0 ]; do
  case "$1" in
    --no-build) BUILD=0 ;;
    --no-video) VIDEO=0 ;;
    --port) PORT="$2"; shift ;;
    --identity) IDENTITY="$2"; shift ;;
    --songs) SONGS="$2"; shift ;;
    -h|--help) sed -n '2,26p' "$0" | sed 's/^# \?//'; exit 0 ;;
    -*) echo "deploy: unknown option $1" >&2; exit 2 ;;
    *)
      if [ -n "$HOST" ]; then echo "deploy: more than one host given" >&2; exit 2; fi
      HOST="$1"
      ;;
  esac
  shift
done

if [ -z "$HOST" ]; then
  echo "deploy: usage: tools/platform/linux/deploy.sh [user@]host [--no-build] [--no-video] [--songs DIR]" >&2
  echo "                                     [--port N] [--identity PATH]" >&2
  exit 2
fi

# The same reasoning as tools/platform/linux/deb.sh: MSYS rewrites anything that looks like a Unix path before
# handing it over, and here that would turn the remote `/tmp` into `C:/Program Files/Git/tmp` and
# copy the package to a directory that does not exist on either machine. Git for Windows' ssh is
# itself an MSYS program, so in practice the conversion does not fire -- but it costs nothing to say
# so out loud rather than depend on which build of ssh happens to be first on PATH.
export MSYS2_ARG_CONV_EXCL='*'

SSH_OPTS=()
SCP_OPTS=()
[ -n "$PORT" ] && { SSH_OPTS+=(-p "$PORT"); SCP_OPTS+=(-P "$PORT"); }
[ -n "$IDENTITY" ] && { SSH_OPTS+=(-i "$IDENTITY"); SCP_OPTS+=(-i "$IDENTITY"); }

# -- 1. the package ------------------------------------------------------------------------------

# Carried as an array rather than a string, so the empty case really is no argument at all.
DEB_ARGS=()
[ "$VIDEO" -eq 0 ] && DEB_ARGS+=(--no-video)

if [ "$BUILD" -eq 1 ]; then
  echo "== build"
  tools/platform/linux/deb.sh "${DEB_ARGS[@]+"${DEB_ARGS[@]}"}"
  echo
fi

# The declined package goes in a subfolder of the platform directory, because both builds produce a
# file of exactly the same name -- cargo-deb names it from the package and the version, neither of
# which a feature changes -- and the selection below is "newest wins". Without the split, the two
# would overwrite each other and this script would send whichever was built last, silently. The same
# reasoning as the `-no-video` leaf folder tools/platform/windows/dist.sh produces, and the marker is on the
# same one: video is the default, so the plain path holds what the plain command built.
OUT="$(dist_dir karaokemachine linux)"
[ "$VIDEO" -eq 0 ] && OUT="$OUT/no-video"
if ! ls "$OUT"/*.deb >/dev/null 2>&1; then
  echo "deploy: no package in $OUT. Run tools/platform/linux/deb.sh ${DEB_ARGS[*]-}, or drop --no-build." >&2
  exit 1
fi

# Newest wins, the same idiom tools/platform/linux/deb-in-container.sh uses to report what it just built.
DEB="$(ls -t "$OUT"/*.deb | head -1)"
DEB_NAME="$(basename "$DEB")"
echo "== package"
# `wc -c` rather than `stat`, whose size flag differs between GNU and BSD -- and this half runs on
# the developer's machine, which is as often macOS as it is Linux. The `stat -c` in
# deb-in-container.sh is correct where it stands, because that half runs inside the Debian image.
printf '  %s (%s bytes)\n' "$DEB" "$(wc -c < "$DEB" | tr -d ' ')"
echo

# -- 2. can we get in at all --------------------------------------------------------------------

# Asked before anything is copied, so a wrong hostname costs a second rather than a 31 MiB upload.
# BatchMode makes it fail instead of prompting, which is exactly what distinguishes "key installed"
# from "will prompt three times".
if ssh "${SSH_OPTS[@]+"${SSH_OPTS[@]}"}" -o BatchMode=yes -o ConnectTimeout=10 \
       "$HOST" true >/dev/null 2>&1; then
  KEYED=1
else
  KEYED=0
  echo "== note"
  echo "  No key-based login to $HOST, so ssh will ask for a password twice below."
  echo "  One-time fix:  ssh-copy-id ${PORT:+-p $PORT }$HOST"
  echo
fi

# -- 3. copy ------------------------------------------------------------------------------------

echo "== copy"
scp "${SCP_OPTS[@]+"${SCP_OPTS[@]}"}" "$DEB" "$HOST:/tmp/$DEB_NAME"
echo

# -- 4. install, enable, restart, report ---------------------------------------------------------

# Everything remote in one invocation, for the password-prompt reason at the top. The script is
# handed over base64-encoded rather than as a quoted argument or on stdin: quoting a multi-line shell
# script through ssh *and* sudo is a well-known way to lose a dollar sign, and stdin is unavailable
# because -t is needed for sudo's own password prompt to work.
REMOTE_SCRIPT=$(cat <<'REMOTE'
set -eu

deb="$1"
echo "== install"
# apt-get rather than dpkg -i, so Depends are resolved from the archive and a missing one is an error
# here rather than a mystery on the television. Same reasoning as tools/platform/linux/verify-deb.sh.
#
# **`--reinstall`, and it is the whole reason a deploy works.** apt compares versions and nothing else:
# handed a local .deb whose version is already installed it prints "karaokemachine is already the
# newest version" and does nothing at all. Development builds are almost always the same version as
# the one on the box -- the version changes at a release, not at a commit -- so without this the
# normal case is a deploy that copies 31 MiB, says `0 upgraded, 0 newly installed`, restarts the
# service, prints a healthy state block, and ships **none of the code you just built**. It cost an
# afternoon here: several rounds of "the change is not on the box" that looked like a build problem,
# a caching problem and a logging problem in turn, and were none of them.
#
# **`--no-install-recommends` is what makes the appliance carry no X**, and it is a deploy's decision
# rather than the package's. The X11 and Wayland libraries are Recommends precisely so that apt can
# be told to decline them: a desktop installing this package takes them and gets its backend, and a
# box under a television takes none and runs kmsdrm, which is the only backend it was ever going to
# use. Nothing is lost by declining them here -- SDL opens every backend with dlopen, so the ones
# that are absent are the ones this box would never have opened. See `What the machine is, on Linux`
# in docs/decisions/distribution.md.
sudo apt-get update -qq
sudo apt-get install -y --reinstall --no-install-recommends "$deb"
rm -f "$deb"
echo

# Said out loud, because the failure above was invisible: the point of a deploy is that the binary on
# the box is the binary that was just built, and until now nothing checked or even mentioned it.
echo "  installed: $(sha256sum /opt/karaokemachine/karaokemachine 2>/dev/null | cut -c1-16)"
echo

# The documented fallback in the unit file. Installed now, while there is a network and an operator,
# rather than at two in the morning when kmsdrm has turned out not to like this GPU. Best effort:
# a box without it in its archive is not a failed deployment.
if ! command -v cage >/dev/null 2>&1; then
  echo "== fallback compositor"
  sudo apt-get install -y cage >/dev/null 2>&1 \
    && echo "  cage installed (unused; see the commented lines in the unit file)" \
    || echo "  cage unavailable -- fine unless kmsdrm fails, then it has to come from somewhere"
  echo
fi

echo "== service"
# Deliberately not fatal. A service that will not start is the moment the journal below matters most,
# and `set -e` here would abort the script just before printing it -- leaving the operator with
# "Job for karaokemachine.service failed" and nothing to act on. The state block reports what
# actually happened.
sudo systemctl enable karaokemachine.service || true
sudo systemctl restart karaokemachine.service || true
echo

# A display manager holds DRM master for itself, and kmsdrm cannot then become master. This is the
# single most likely reason for a black television on a box installed as a desktop, and it is
# invisible from here unless somebody looks -- so look.
for dm in gdm3 gdm lightdm sddm xdm lxdm; do
  if systemctl is-enabled --quiet "$dm.service" 2>/dev/null; then
    echo "== WARNING"
    echo "  $dm is enabled and will hold DRM master, so the machine cannot take the screen."
    echo "  sudo systemctl disable --now $dm"
    echo
  fi
done

echo "== power button"
# The one control a box under a television has, and the step that gives it to the machine. The
# package ships the drop-in inert under /opt precisely so that installing the application on a
# desktop does not repurpose its power button; putting it into /etc is a deploy's act, the same way
# enabling the unit is. See linux/logind-powerkey.conf for the whole argument.
if [ -f /opt/karaokemachine/logind-powerkey.conf ]; then
  sudo install -d -m 755 /etc/systemd/logind.conf.d
  sudo install -m 644 /opt/karaokemachine/logind-powerkey.conf \
    /etc/systemd/logind.conf.d/10-karaokemachine-powerkey.conf && echo "  installed"
  # What logind believes *now*, which is not what the file says until the next boot: logind has no
  # reload, and restarting it underneath a PAM session holding DRM master is a risk for no gain when
  # the file restates the default anyway. So this reports rather than acts.
  effective=$(busctl get-property org.freedesktop.login1 /org/freedesktop/login1 \
    org.freedesktop.login1.Manager HandlePowerKey 2>/dev/null | awk '{print $2}' | tr -d '"')
  printf '  in force:  %s\n' "${effective:-?}"
  if [ -n "$effective" ] && [ "$effective" != "poweroff" ]; then
    echo "  (it becomes poweroff at the next boot)"
  fi
  # A second handler is the likeliest reason a press does nothing or does the wrong thing, and it is
  # invisible unless something looks -- the same species as the display-manager warning above.
  if systemctl is-enabled --quiet acpid.service 2>/dev/null; then
    echo "== WARNING"
    echo "  acpid is enabled and may handle the power button itself."
    echo "  Check /etc/acpi/events/ before trusting a single press."
  fi
else
  echo "  (this package predates the drop-in)"
fi
echo

echo "== state"
printf '  version:   %s\n' "$(karaokemachine --version 2>/dev/null || echo '?')"
printf '  active:    %s\n' "$(systemctl is-active karaokemachine.service || true)"
printf '  enabled:   %s\n' "$(systemctl is-enabled karaokemachine.service || true)"
printf '  paths:\n'
# HOME is the whole question. --show-paths resolves the XDG directories from it and reads nothing, so
# the answer depends on which HOME is set and not on who is asking -- setting it to the karaoke
# account's home gives the service's real paths from any account, with no privilege at all.
#
# The obvious version, `sudo -u karaoke -H karaokemachine --show-paths`, was what this did and it is
# a trap twice over. It needs a sudoers rule permitting `sudo -u karaoke`, which a box locked down to
# the specific commands a deploy runs will not have -- on the appliance this printed "(unavailable)"
# every time and nobody ever saw the real output. And -H is load-bearing in that form: without it
# sudo leaves HOME pointing at the calling user and the check answers a different question while
# looking like it passed. Asking `getent` for the home rather than writing /var/lib/karaoke keeps
# this the package's answer instead of this script's guess at it.
karaoke_home=$(getent passwd karaoke | cut -d: -f6 || true)
if [ -n "$karaoke_home" ]; then
  HOME="$karaoke_home" karaokemachine --show-paths 2>/dev/null | sed 's/^/    /' \
    || echo "    (unavailable)"
else
  echo "    (no karaoke account yet)"
fi
echo

echo "== journal"
# The four ways this comes up looking fine and showing nothing are all logged and none are fatal:
# a display that would not start, no usable font, no audio device, no SoundFont. Printed every time
# rather than only when somebody complains.
#
# `-t karaokemachine` (the syslog identifier), NOT `-u karaokemachine`. This is a direct consequence
# of PAMName=login: logind puts the process in a *session scope*, so journald files the application's
# own output under `session-N.scope` in `user-988.slice` rather than under the service. `-u` returns
# the two lines systemd itself wrote and none of the eleven the machine wrote -- which reads exactly
# like an application that started and then said nothing, and cost an hour the first time. The very
# thing that makes the screen work is what hides the log.
sudo journalctl -t karaokemachine -n 30 --no-pager | sed 's/^/  /'
REMOTE
)

echo "== remote"
ssh "${SSH_OPTS[@]+"${SSH_OPTS[@]}"}" -t "$HOST" \
  "echo $(printf '%s' "$REMOTE_SCRIPT" | base64 | tr -d '\n') | base64 -d | bash -s -- /tmp/$DEB_NAME"
echo

# -- 5. songs, optionally ------------------------------------------------------------------------

if [ -n "$SONGS" ]; then
  if [ ! -d "$SONGS" ]; then
    echo "deploy: --songs $SONGS is not a directory" >&2
    exit 1
  fi
  echo "== songs"
  # km-catalog reads a package in place, so these files have to stay where they land, permanently.
  # They land in the machine's own packages folder, and **that folder is what says what is
  # installed**: a package that arrives here is installed whether or not anything announces it, it
  # stays installed across a reinstall of the .deb, and taking it out again *is* the uninstall. The
  # older reason given here -- that settings.packages was replayed at every start -- is gone with
  # that list, and the conclusion is stronger without it rather than weaker.
  #
  # Asked for rather than written down. It is derived from the data directory, so hard-coding it here
  # would be this script's guess at another program's answer -- and `--show-paths` is that program
  # saying it. HOME is what selects which account's answer, and it is set explicitly rather than
  # reached through `sudo -u karaoke -H`: see the long note beside the paths check above for why that
  # form silently fails on a box whose sudoers lists only the commands a deploy needs.
  DEST=$(ssh "${SSH_OPTS[@]+"${SSH_OPTS[@]}"}" "$HOST" \
    "HOME=\$(getent passwd karaoke | cut -d: -f6) karaokemachine --show-paths 2>/dev/null \
       | awk '/^packages/ {print \$2}'")
  if [ -z "$DEST" ]; then
    echo "deploy: could not ask the box where its packages folder is." >&2
    echo "        Is karaokemachine installed and on PATH there? Try: $0 $HOST" >&2
    echo "        A build older than the packages folder itself has no such path to report." >&2
    exit 1
  fi
  echo "  into $DEST"
  ssh "${SSH_OPTS[@]+"${SSH_OPTS[@]}"}" "$HOST" \
    "sudo install -d -o karaoke -g karaoke -m 755 $DEST"

  ssh "${SSH_OPTS[@]+"${SSH_OPTS[@]}"}" "$HOST" "mkdir -p /tmp/kmpkg"

  # A package is one file, so there is nothing here to leave behind. Two things travelling
  # together -- the `.kmpkg` and a `<name>.media/` folder beside it -- would make this block thirty
  # lines of rsync filters, a separate scp for the folders, and an `rm -rf` on the far end to stop
  # `mv` nesting one inside another; and sending only the packages would install a catalog whose
  # every video song is a missing-media skip: listed, queueable, and passed over while somebody
  # holds a microphone.
  printf '  %d package(s)\n' "$(ls -1 "$SONGS"/*.kmpkg 2>/dev/null | wc -l)"

  if command -v rsync >/dev/null 2>&1; then
    # The transport has to be built as an array. `${PORT:+-e "ssh -p $PORT"}` looks right and word
    # splits into four arguments, of which `"ssh` is not a program.
    RSYNC_OPTS=()
    [ -n "$PORT" ] && RSYNC_OPTS+=(-e "ssh -p $PORT")
    [ -n "$IDENTITY" ] && RSYNC_OPTS+=(-e "ssh ${PORT:+-p $PORT }-i $IDENTITY")
    rsync -a --progress "${RSYNC_OPTS[@]+"${RSYNC_OPTS[@]}"}" \
      --include='*.kmpkg' --exclude='*' \
      "$SONGS/" "$HOST:/tmp/kmpkg/"
  else
    # Git Bash has no rsync, so this is the branch that actually runs on the development machine.
    # It re-sends everything every time, which was merely wasteful when a package was a manifest and
    # some MIDI, and is worth minding now that one can be twenty gigabytes -- install rsync if that
    # starts to matter.
    scp "${SCP_OPTS[@]+"${SCP_OPTS[@]}"}" "$SONGS"/*.kmpkg "$HOST:/tmp/kmpkg/"
  fi

  # Moved into place as root, then the machine is restarted so that it reads the folder again.
  #
  # **There is deliberately no registration call here.** `POST /api/v1/packages` lives under the
  # admin router, because installing is an owner's act rather than the one write a stranger may
  # perform, and only `GET` is public at that path -- so a script posting there earns a **405 on
  # every package**, prints `failed` beside each one, and still exits 0, while the songs sit in the
  # folder unread until something else restarts the machine. The symptom is a television with an
  # empty catalog after a deploy that said it had sent the songs.
  #
  # **A restart rather than the admin route, because the alternative is a credential.** That route
  # wants `Authorization: Bearer`, and a shell script can only hold one by growing a
  # `--password` flag or by reading the machine's own settings file -- which this account cannot do,
  # the data directory being 0700 and owned by `karaoke`. The block above already says that the
  # folder is what says what is installed, and a restart is simply the machine reading that folder
  # again. It needs no password and it is true on every box, not just one whose sudoers was written
  # generously.
  #
  # The cost is one more restart. The main block has already restarted the machine by this point, so
  # this is a second interruption during a deploy rather than a new kind of one.
  SONGS_SCRIPT=$(cat <<'REMOTE_SONGS'
set -eu
dest="$1"

# One `mv` and nothing else. A second loop for `<name>.media/` folders would need an `rm -rf` in
# front of each, because `mv` of a directory onto an existing one of the same name nests it inside
# instead of replacing it -- which puts videos at `vol1.media/vol1.media/` and leaves every one of
# them unresolvable while looking like it worked. A package is one file, so there is no folder, no
# nesting and no `rm -rf` on the appliance.
sudo mv /tmp/kmpkg/*.kmpkg "$dest"/
sudo chown -R karaoke:karaoke "$dest"
rmdir /tmp/kmpkg 2>/dev/null || true

# Deliberately not fatal, for the same reason the enable and restart in the main block are not: the
# packages are already where they belong and are already installed by virtue of being there, so a
# machine that will not come back is a problem the journal describes better than an abrupt exit
# would.
echo "  restarting the machine so it reads the packages folder again"
sudo systemctl restart karaokemachine.service || true
REMOTE_SONGS
)
  ssh "${SSH_OPTS[@]+"${SSH_OPTS[@]}"}" -t "$HOST" \
    "echo $(printf '%s' "$SONGS_SCRIPT" | base64 | tr -d '\n') | base64 -d | bash -s -- $DEST"
  echo
fi

# -- 6. what to do next ---------------------------------------------------------------------------

BOX="${HOST#*@}"
echo "== done"
echo "  Look at the television. That is the test nothing here substitutes for."
echo
echo "  remote:   http://$BOX:8177/"
echo "  logs:     ssh $HOST 'sudo journalctl -t karaokemachine -f'"
echo "  restart:  ssh $HOST 'sudo systemctl restart karaokemachine'"
[ "$KEYED" -eq 0 ] && echo "  no more password prompts:  ssh-copy-id ${PORT:+-p $PORT }$HOST"
echo
echo "  If the service is active but the screen is black, that is the DRM-master problem:"
echo "    ssh $HOST 'sudo journalctl -t karaokemachine | grep -i \"display could not start\"'"
echo "  and the cure is the commented cage lines in /lib/systemd/system/karaokemachine.service."
