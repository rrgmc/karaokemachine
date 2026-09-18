#!/usr/bin/env bash
#
# Stages a portable Windows build of the machine.
#
#   tools/platform/windows/dist.sh              # fetch assets, build release, stage the folder
#   tools/platform/windows/dist.sh --no-fetch   # skip the SoundFont step (it is already cached)
#   tools/platform/windows/dist.sh --zip        # also produce the .zip beside the folder
#   tools/platform/windows/dist.sh --no-video   # build without the `video` feature, and stage no DLLs
#   tools/platform/windows/dist.sh -v           # watch the build; quiet is the default
#
# **Quiet by default.** The phases, the report at the end and every warning are printed; cargo's
# compile stream is not. A step that fails replays everything it held back, so `-v` is for watching
# a build rather than for diagnosing one afterwards.
#
# **Video is on by default.** The easy command produces the machine that can play every kind of song
# a catalog can hold; `--no-video` is how you ask for the smaller folder. This is a *release*
# default and nothing else -- the cargo feature is still off by default, so `cargo build -p
# karaokemachine` needs neither ffmpeg nor libclang.
#
# The output is a folder that runs when copied to another machine:
#
#   dist/karaokemachine/windows/karaokemachine-1.1.0-x86_64-pc-windows-msvc/
#     karaokemachine.exe            <- double-click this: the machine, and no console beside it
#     karaokemachine-console.exe    <- for diagnosing: it prints, so --help and --set-password work
#     README.txt
#     assets/soundfont/GeneralUser-GS.sf2
#     assets/wallpapers/*.png
#
# App, then platform, then the folder -- the layout rule lives in tools/dist/common.sh, which this
# script sources along with the staging idioms it shares with tools/dist/cmd.sh and
# tools/platform/macos/app-bundle.sh.
#
# Why a folder rather than just the exe: `Paths::discover_asset_dir` looks for an `assets` directory
# beside the executable and falls back to the working directory. `cargo run` from the repository root
# finds `assets/` by that fallback, which is exactly why a bare copied exe silently loses its
# SoundFont, its wallpapers and its font -- it comes up on a sine test tone over a plain gradient and
# nothing says why. The layout below is the one that function was written for.
#
# Why the script exists rather than a line of documentation saying `cargo build --release`: it calls
# tools/setup/fetch-assets.sh, so a staged build ships the SoundFont instead of depending on whoever ran
# the build having fetched it first. That was the open loose end in docs/ARCHITECTURE.md.
#
# Prerequisites: a Rust toolchain with the MSVC linker, and CMake -- SDL3 and SDL3_ttf are built from
# source and linked statically, so the exe needs no SDL DLL beside it.
#
# Video additionally needs tools/setup/fetch-ffmpeg.sh to have been run once on this machine. ffmpeg is
# the one library that is *not* static here, and Windows is the one platform where that costs
# nothing: the loader searches the executable's own directory first, so the DLLs staged beside the
# exe are found with no PATH, no installer and no environment variable. The same folder still copies
# to a USB stick and runs. (macOS cannot do this without rewriting install names, and on Linux the
# .deb declares the libraries as dependencies instead -- see docs/ARCHITECTURE.md.)

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/dist/common.sh
DIST_SCRIPT=dist

TARGET="x86_64-pc-windows-msvc"
FETCH=1
ZIP=0
VIDEO=1
for arg in "$@"; do
  case "$arg" in
    --no-fetch) FETCH=0 ;;
    --zip) ZIP=1 ;;
    --no-video) VIDEO=0 ;;
    # The build logs, which are quiet by default. A failure replays whatever was held back, so this
    # is for watching a build rather than for diagnosing one after the fact.
    -v|--verbose) DIST_VERBOSE=1 ;;
    *) echo "dist: unknown option $arg" >&2; exit 2 ;;
  esac
done

# -- the assets ----------------------------------------------------------------------------------

# Cheap when the machine-local cache is warm: it touches nothing and, at this verbosity, says nothing
# either. It is wrapped rather than taught a flag of its own, so that running it directly -- which is
# what CLAUDE.md tells somebody to do once per machine -- is unchanged.
if [ "$FETCH" -eq 1 ]; then
  dist_step assets
  dist_run "fetch-assets" tools/setup/fetch-assets.sh
fi

# -- the build -----------------------------------------------------------------------------------

# Resolved *before* the build, so a missing install fails in a second with a sentence naming the fix,
# rather than after a release build that then dies in `ffmpeg-sys-next` with pkg-config noise
# mentioning neither ffmpeg nor clang. `dist_ffmpeg_on_path` is what lets the freshly built exe be
# *run* below, before its DLLs are staged beside it -- see tools/dist/common.sh for why that PATH
# entry has to be converted back to POSIX form first.
# **`tray` rides with `video` and is named here rather than in a features file**, which is the same
# bargain `km-remote`'s `desktop` makes: the icon's Linux backend links `libayatana-appindicator` at
# load time, so the feature has to be switched on per carrier rather than by anything a workspace
# build reads. Windows and macOS stage it; the two Linux carriers never mention it.
#
# It is joined to `video` because the run that wants an icon is the streaming one, and that needs an
# encoder: an icon on a build that cannot stream would be an icon for a window that is already there.
FEATURES=()
if [ "$VIDEO" -eq 1 ]; then
  FFMPEG_DIR="$(dist_ffmpeg_dir)"
  FEATURES=(--features video,tray)
  dist_ffmpeg_on_path "$FFMPEG_DIR"

  dist_step video
  dist_detail "ffmpeg  $FFMPEG_DIR"
fi

# The blank lines between phases went with the detail they used to separate. At this verbosity most
# steps are a single line, and a run of headers each trailed by an empty one reads as three sections
# that failed to say anything rather than as three that had nothing to report.
dist_step build
build_started=$SECONDS
# shellcheck disable=SC2046  # dist_cargo_quiet prints one flag or nothing at all
cargo build --release $(dist_cargo_quiet) -p karaokemachine "${FEATURES[@]+"${FEATURES[@]}"}"
printf '   built in %s\n' "$(dist_elapsed "$build_started")"
# **Two executables, and both are checked for.** They are the same library under two subsystems --
# `karaokemachine` GUI, so a double-click opens the machine and no console beside it, and
# `karaokemachine-console` not, so there is somewhere for `--help` and `--set-password` to print. See
# the `The machine's console window` decision in docs/decisions/. Neither is behind a cargo feature, so
# building the package builds the pair; a missing one is a real fault and not a configuration.
EXE="$(dist_target_dir)/release/karaokemachine.exe"
TWIN="$(dist_target_dir)/release/karaokemachine-console.exe"
for produced in "$EXE" "$TWIN"; do
  if [ ! -f "$produced" ]; then
    echo "dist: $produced was not produced" >&2
    exit 1
  fi
done
echo

# -- the folder ----------------------------------------------------------------------------------

# **Deliberately the GUI-subsystem one**, though the console twin is right here and would obviously
# work. Standard handles are inherited whatever the subsystem, so a GUI-subsystem process answers
# `--version` perfectly well down the pipe `dist_version` puts it on -- and every release depending on
# that is worth one line that would fail loudly if it ever stopped being true. Asking the twin instead
# would stage releases without ever exercising it. Verified on Windows: prints down a pipe, and prints
# nowhere and does not panic when launched with no handles at all.
VERSION="$(dist_version "$EXE")"

# The two are different products in the same version -- one plays a kind of song the other cannot --
# so they cannot share a folder: the second staging would silently replace the first, and `--zip`
# would overwrite a zip that had already been sent to somebody. **The marker is on the declined
# build**, because video is the default and the plain name should name what the plain command
# produces. It spells the flag exactly, so the folder is guessable from the command that made it.
NAME="karaokemachine-$VERSION-$TARGET"
if [ "$VIDEO" -eq 0 ]; then NAME="$NAME-no-video"; fi
PARENT="$(dist_dir karaokemachine windows)"
DEST="$PARENT/$NAME"

dist_clear "$DEST"

cp "$EXE" "$DEST/karaokemachine.exe"
cp "$TWIN" "$DEST/karaokemachine-console.exe"

# -- ffmpeg, for a video build ---------------------------------------------------------------------
#
# Beside the exe, not in a subfolder: the Windows loader searches the directory the executable was
# loaded from, and nowhere else that would help here. The staging and its LGPL obligation are in
# tools/dist/common.sh, because tools/dist/cmd.sh has exactly the same duty for km-pack and
# km-package-builder.
dlls=0
if [ "$VIDEO" -eq 1 ]; then
  dlls="$(dist_stage_ffmpeg "$DEST" "$FFMPEG_DIR")"
fi

assets="$(dist_stage_assets "$DEST")"

# No `remote-dev` folder: the web remote is compiled into the executable (`DEV_REMOTE_HTML` in
# km-api). It is one self-contained page that fetches nothing, so there is nothing for a folder to
# carry -- and a staged build that forgot it would serve the landing page at /dev/, which looks like
# the feature is switched off rather than like a file is missing.
#
# A `remote-dev` directory beside the exe still wins if one is put there, which is how the page gets
# edited and reloaded while somebody is working on it.

# -- the note for whoever receives the folder -----------------------------------------------------

# Written in pieces rather than as one heredoc. Two passages differ between a video build and a plain
# one, and the body has to stay a *quoted* heredoc because it contains a Windows path full of
# backslashes -- which an unquoted one would be at liberty to eat.
{
  printf 'karaokemachine\n==============\n\n'
  if [ "$VIDEO" -eq 1 ]; then
    printf 'Plays karaoke MIDI files (.mid / .kar) and video songs (.mp4), with the lyrics\n'
    printf 'highlighted in time with the music.\n'
  else
    printf 'Plays karaoke MIDI files (.mid / .kar) with the lyrics highlighted in time with the music.\n'
  fi
} > "$DEST/README.txt"

cat >> "$DEST/README.txt" <<'README'

Running it
----------

Double-click karaokemachine.exe. Nothing else appears: no console window beside the lyrics.

This folder is portable: copy it anywhere, including a USB stick. The one rule is that the
assets folder must stay beside karaokemachine.exe. That is where the instrument bank and the
wallpapers live; without it the app still runs, but every instrument becomes a plain test tone
and the screen shows a gradient instead of a wallpaper.

If there is a karaokemachine-console.exe beside it, that is the same machine with somewhere to
print, for when you need to see why something did not start. Every command below works with either
name.

Nothing needs installing first, with one possible exception: the app links the Microsoft Visual
C++ runtime dynamically, so it needs VCRUNTIME140.dll. That comes with the "Microsoft Visual C++
2015-2022 Redistributable (x64)", which is already on almost every Windows machine. If the app
refuses to start with an error naming that file, install the redistributable from Microsoft.

The app starts in a window. F makes it fullscreen and takes it back out again; Esc leaves
fullscreen, or quits if the app is already in a window. To start it fullscreen every time, set
"fullscreen": true under "display" in settings.json -- --show-paths says where that is -- or pass
--fullscreen for one run. (The setup program does this for you; a folder like this one cannot,
because it does not know it is the machine under your television rather than a copy you are trying
out.)

There are no songs yet
----------------------

The machine plays songs from a package (a .kmpkg file), and a fresh install has none.

Put the .kmpkg into the packages folder and start the machine again. It is made for you the first
time the machine runs, and --show-paths says where it is. A package is one file, videos and all,
so there is nothing beside it to remember to copy.

Packages can also be installed over the network with the control API, which does not need a restart.
Or -- to check that the machine works at all before you have a package -- point it straight at a
file:

    karaokemachine.exe --play "C:\path\to\song.kar"

Options
-------

    --show-paths          where settings, the catalog, your packages and the assets are
    --set-password PASS   set the admin password for the control API
    --reset-password      go back to a freshly generated PIN, shown on the machine's screen
    --song-book FILE      write every installed song to FILE as a PDF, to print. Read from the
                          catalog as it stands, so start the machine once after adding a package
    --book-name NAME      what that book says at the top left. Defaults to KaraokeMachine; worth
                          setting if there is a machine in more than one room
    --headless            no window: the API, the engine and the catalog only
    --data-dir DIR        keep settings, the catalog and packages in DIR instead of your
                          user profile
    --play FILE           play one MIDI or KAR file directly, bypassing packages
    -v, -vv               say more in the log; -v is this app's own detail
    --frame-stats         report the frame rate once a second, to answer "is it stuttering?"
    --log-file            also write this run's log to a file, in a logs folder beside the
                          catalog. Double-clicking karaokemachine.exe opens no console, so
                          without this there is nowhere for it to say what went wrong. One file
                          per run, the ten newest kept; --show-paths says where they are
    --log-keep COUNT      how many of those to keep, or "all" to keep every one of them. This
                          turns the log file on by itself. Each name carries the date and time
                          the run started, so "all" leaves the machine's whole history there.
                          A crash report is written whatever these two say, and is kept apart
    --version

Settings live in settings.json, the song catalog in library.sqlite and your packages in a folder
called packages, all three under your user profile. --show-paths prints exactly where. To keep a run
self-contained inside this folder instead, start it with --data-dir . from here -- the packages
folder is then in here too, beside the program.

To have every run write a log without typing anything -- which is what you want on a machine you
start by double-clicking it -- put this in settings.json:

    "logging": { "file": true, "keep": "all" }

The flag wins over the file, so one run's --log-file changes nothing permanently.

Controlling it from another device
---------------------------------

The app listens on every network interface, so anything on the same Wi-Fi can control it -- a
laptop, a tablet, or a phone. The address is shown on screen with a QR code; scan it or type it:

    http://<this machine's address>:8177/

That is the remote a singer holds: search the catalog, queue songs, and control playback, key and
tempo. It is built for a phone.

Add /admin/ to the same address for the owner's page -- installing packages, wallpapers, sound
banks and the machine's own settings. It is laid out for a desktop or a tablet.

    http://<this machine's address>:8177/admin/

There is a third page, /dev/, and it is off unless you ask for it. It is a console that exercises
every part of the control API, including things the two pages above deliberately do not offer, like
editing which endpoints need the password and playing a file straight off the disk. Start the app
with --dev-remote to serve it for one run, or put "serve_dev_remote": true under "api" in
settings.json to serve it always. The API itself is available either way, so anything you write
yourself keeps working.

The machine gives itself a password the first time it starts -- six digits, shown on its own
screen beside the address. Everything that reconfigures the machine needs it: installing and
removing songs, choosing the sound output, renaming the machine, adding pictures. Everything a
singer does needs nothing, which is the point -- anybody in the room can queue a song.

Change it from the machine's own page, at

    http://<this machine>:8177/admin/

or with

    karaokemachine.exe --set-password <something>

Until you do, the page says so on every tab. That matters most if port 8177 is ever reachable from
outside your house: the PIN on the screen is fine for a room and is not a secret from the internet.

To keep the machine off the network entirely, set "bind": "127.0.0.1:8177" under "api".
README

if [ "$VIDEO" -eq 1 ]; then
  cat >> "$DEST/README.txt" <<'README'

Video songs
-----------

This build plays video songs as well as MIDI ones. A video song is an MP4 with the words burned
into the picture, so nothing is highlighted over it -- the machine plays the file and shows it.

The four avcodec/avformat/avutil/swresample DLLs beside karaokemachine.exe are what decode them.
They must stay in this folder; without them the app will not start at all, with an error naming
one of the files rather than anything about video. Nothing needs installing for them to work.

--play accepts a video file in this build:

    karaokemachine.exe --play "C:\path\to\song.mp4"
README
else
  cat >> "$DEST/README.txt" <<'README'

No video songs
--------------

This is the --no-video build. A catalog holding video songs still lists, searches and queues them,
and the machine says at startup that it cannot play them -- so a song that is skipped is telling you
which build you have, not that anything is wrong with the file. The ordinary build plays them.
README
fi

cat >> "$DEST/README.txt" <<'README'

Licenses
--------

The bundled instrument bank is GeneralUser GS; its terms are in assets/soundfont/LICENSE.txt.
karaokemachine itself is MIT OR Apache-2.0, at your option -- both texts are beside this file, in
LICENSE-MIT.txt and LICENSE-APACHE.txt.
README

dist_stage_app_licenses "$DEST"

if [ "$VIDEO" -eq 1 ]; then
  # The LGPL obligation, stated plainly and in the folder rather than in a repository nobody
  # receiving this zip can see. Unmodified binaries, dynamically linked, terms included, source
  # named -- which is what makes shipping them legitimate. Written once, in tools/dist/common.sh,
  # because getting it subtly different in three folders is exactly the kind of thing nobody notices
  # until it matters.
  dist_ffmpeg_license_note "$FFMPEG_DIR" >> "$DEST/README.txt"
fi

# -- report ---------------------------------------------------------------------------------------

total="$(dist_bytes "$DEST")"
printf 'staged %s\n' "$DEST"
printf '  karaokemachine.exe %s   (+ karaokemachine-console.exe, for diagnosing)\n' "$VERSION"
printf '  %s asset file(s)\n' "$assets"
if [ "$VIDEO" -eq 1 ]; then
  printf '  %s ffmpeg DLL(s) + ffmpeg-LICENSE.txt\n' "$dlls"
  printf '  video yes\n'
else
  printf '  video no  -- video songs catalog and queue, but will not play\n'
fi
printf '  %s bytes total (~%s MiB)\n' "$total" "$((total / 1024 / 1024))"

# The claim this script makes about a video build is that the folder is self-contained, and that is
# worth *testing* rather than asserting: the whole point of staging the DLLs is that the exe starts
# with no ffmpeg anywhere else on the machine. So run it with a PATH stripped of everything that
# could be hiding the failure. `--show-paths` prints and exits, which makes it a cheap way to prove
# the process got as far as running its own code.
#
# A plain build needs no such check: it links nothing that is not either static or already in
# Windows.
#
# **Both executables, because a folder half of which works is the failure this is here to catch.**
# They link the same DLLs, so in practice they pass or fail together -- but "in practice" is what the
# check exists to replace, and the loader searches the directory of the executable that was started,
# which is a different directory question for each of the two.
if [ "$VIDEO" -eq 1 ]; then
  echo
  ok=1
  for exe in karaokemachine karaokemachine-console; do
    ( cd "$DEST" && PATH="/c/Windows/System32:/c/Windows" "./$exe.exe" --show-paths >/dev/null 2>&1 ) || ok=0
  done
  if [ "$ok" -eq 1 ]; then
    echo "verified: both executables start with no ffmpeg on PATH, so the folder is self-contained."
  else
    echo "warning: a staged exe would not start with a bare PATH." >&2
    echo "         Something it links is missing from the folder; it will fail on another machine." >&2
    exit 1
  fi
fi

# Said here rather than left to be discovered by ear on someone else's machine: no bank
# means the machine comes up on its sine test tone, which sounds broken to anyone who does not know
# it is the documented fallback.
if ! find "$DEST/assets" -name '*.sf2' | grep -q .; then
  echo
  echo "warning: no SoundFont in the folder -- the app will fall back to a test tone."
  echo "         Run tools/setup/fetch-assets.sh, then this script again."
fi

# -- the zip, if asked for ------------------------------------------------------------------------

if [ "$ZIP" -eq 1 ]; then
  echo
  dist_zip "$PARENT" "$NAME"
fi

echo "now:  cd $DEST && ./karaokemachine-console.exe --show-paths"
