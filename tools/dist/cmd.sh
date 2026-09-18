#!/usr/bin/env bash
#
# Stages portable builds of the command-line tools.
#
#   tools/dist/cmd.sh                    # all six
#   tools/dist/cmd.sh km-pack            # just one, or any subset
#   tools/dist/cmd.sh --zip              # also produce a .zip beside each folder
#   tools/dist/cmd.sh --no-video         # build the video-capable tools without the `video` feature
#   tools/dist/cmd.sh -v                 # watch the builds; quiet is the default
#
# **Quiet by default**, and this is the script it matters most in: with no arguments it runs
# `cargo build --release` six times. The phases, the per-tool reports and every warning are printed;
# the compile streams are not. A step that fails replays everything it held back, so `-v` is for
# watching a build rather than for diagnosing one afterwards.
#
# **Video is on by default**, for the tools that have such a feature. A staged km-package-builder
# pointed at a folder of MP4s used to index every one as "a video, and this build has no `video`
# feature to read it with" -- correct behavior for a build somebody asked for, and the wrong
# product to get from the easy command. So the easy command now produces the tool that can read
# them, and `--no-video` is how you ask for the smaller one. Note that this is a *release* default
# and nothing else: the cargo feature is still off by default, so `cargo build --workspace` needs
# neither ffmpeg nor libclang.
#
# The output follows the layout rule in tools/dist/common.sh -- app, then platform, then the folder:
#
#   dist/km-pack/windows/km-pack-1.1.0-x86_64-pc-windows-msvc/
#     km-pack.exe
#     README.txt
#   dist/km-lyrics/windows/km-lyrics-1.1.0-x86_64-pc-windows-msvc/
#   dist/km-package-builder/windows/km-package-builder-1.1.0-x86_64-pc-windows-msvc/
#     km-package-builder.exe            <- double-click this: a window, and no console flashing past
#     km-package-builder-console.exe    <- type this: it prints, so --help and --version work
#     README.txt
#   dist/km-admin/windows/km-admin-1.1.0-x86_64-pc-windows-msvc/
#   dist/km-wallpaper-pack/windows/km-wallpaper-pack-1.1.0-x86_64-pc-windows-msvc/
#
# Each tool gets a folder of its own rather than the six sharing one. They are six separate
# things -- a packager, a parser you point at a file when something looks wrong, a web server you
# leave running for an afternoon, the singer's remote, a finder of pictures and instrument banks,
# and a wallpaper builder -- and somebody who wants the curation tool has no use for the other
# five. A folder per tool also means the app-name level of the path always names a real product.
#
# Every executable here is self-contained. There is no asset-fetching step, because none of them has
# a SoundFont, a wallpaper or a font; km-package-builder's page, script and stylesheet are compiled
# into it. The folder exists to carry the README. That is the difference from tools/platform/windows/dist.sh,
# which stages 31 MiB of instrument bank beside the machine because `Paths::discover_asset_dir` goes
# looking for it.
#
# **km-wallpaper-pack is the one exception to "the exe alone is a working install"**, and its README
# says so first rather than last: it needs a config file beside it and two API keys in the
# environment, and what it produces -- a zip of photographs -- is the deliverable rather than the
# executable. It is staged anyway because a release build of it is worth having: `analyze` is a few
# thousand JPEG decodes and blurs, and an unoptimized one over a full cache took 78 minutes against
# single-digit ones optimized.
#
# This script is not under tools/platform/windows/ because, unlike the machine, these all build and run the
# same way on all three desktops -- no SDL, no CMake, no C toolchain beyond the bundled SQLite that
# cargo handles itself. The one exception is video, and it is handled per platform below.
#
# Replaces tools/dist-curate.sh, which did exactly this for one of the three.

set -euo pipefail

cd "$(dirname "$0")/../.."

. tools/dist/common.sh
DIST_SCRIPT=dist-tools

ALL_TOOLS=(km-pack km-lyrics km-package-builder km-remote km-admin km-wallpaper-pack)

# Which tools can be built with video, and which simply have no such feature. km-lyrics parses MIDI
# text and never touches a container, so asking for `--features video` there is not a smaller build --
# it is a cargo error. Naming the fact here means video can stay a whole-run default rather than
# something the caller has to remember to apply per tool. It is also what keeps the ffmpeg
# precondition off a run that cannot use one: `tools/dist/cmd.sh km-lyrics` stages on a machine
# with no ffmpeg at all, because nothing it was asked for could have taken the feature.
video_capable() { case "$1" in km-pack|km-package-builder) return 0 ;; *) return 1 ;; esac; }

# Which tools have a window to build, and on which platforms it is a good idea.
#
# **Two of them: km-package-builder and km-remote.** Both are local web servers whose page is the
# whole interface, which is exactly the shape a webview frames well; the other four are commands that
# run and exit, and have nothing to put in a window.
#
# Linux is deliberately excluded for both, rather than merely untested. `wry` links libwebkit2gtk at
# load time, so a Linux build carrying the feature does not *start* on a machine that lacks it -- and
# `--browser` cannot rescue that, because the failure is in the dynamic loader before `main` runs.
# That would trade these tools' best property, the one the READMEs below promise ("copy it anywhere
# and it works"), for a window, on the one platform where opening a browser is universal. It is the
# same reasoning that makes tools/platform/linux/tarball.sh build its own ffmpeg rather than depend on
# Debian's. See the `The package builder's window` and `The remote's window` rows in docs/decisions/.
#
# So a Linux build of either serves its page to the default browser, exactly as it always has, and
# stays one file with no runtime dependency beyond libc.
desktop_capable() {
  case "$1" in
    km-package-builder|km-remote|km-admin) [ "$PLATFORM" != "linux" ] ;;
    *) return 1 ;;
  esac
}

# Which tools are additionally staged as a macOS application bundle, beside the folder.
#
# **Beside, and not instead of.** The folder is still the CLI -- `--help`, `--init`, `--scan`,
# `--register` -- and a `.app` cannot be typed. What the bundle adds is the half a folder cannot have
# on that platform: a Dock icon and a double-click, and for the builder a declared document type,
# which is what makes a `.kmbuild` open its corpus. See `tools/platform/macos/Info.package-builder.plist`,
# which has existed and been read by nothing since M17.
#
# macOS only, because a bundle is a macOS thing; Windows gets the same double-click from the exe's own
# resource and Linux from the `.desktop` entry `--register` writes.
#
# **Two of them now, and the second is here for the setup program.** km-remote is the other local
# web server with a window, and until the macOS installer there was nothing on that platform for its
# `remote` component to install that somebody could start without a terminal. It declares no document
# type -- it opens no files -- so its `tools/platform/macos/Info.remote.plist` is the short kind, and the one
# thing it does declare that neither sibling does is `NSLocalNetworkUsageDescription`, mDNS being how
# it finds a machine at all.
#
# **`--no-desktop` takes the bundle with it, and that is not tidiness.** For the builder the reason is
# sharp: a document double-clicked on macOS arrives as an Apple Event, which needs an event loop to
# receive it -- so a bundle around a build with no window would declare the `.kmbuild` type, be handed
# a corpus, and have nowhere for it to land, and declaring a type you cannot then open is worse than
# not declaring it. The remote declares no type, so that argument does not reach it; the conclusion
# does, for a duller reason. A `.app` whose double-click opens a browser tab rather than a window is
# a Dock icon that lies about what it is, and the folder beside it is the honest form of that build.
# See `bundle=` in the staging loop, which is where the two are actually combined.
bundle_capable() {
  case "$1" in
    km-package-builder|km-remote|km-admin) [ "$PLATFORM" = "macos" ] ;;
    *) return 1 ;;
  esac
}

# What the staged bundle is called, and what goes in it. Kept beside `bundle_capable` so that adding a
# tool means editing one place; the plist's `CFBundleExecutable` must name the tool, and its
# `CFBundleIconFile` the icon here without its extension.
#
# **Each has a default arm because an unmatched `case` prints nothing and says nothing.** A tool
# `bundle_capable` admits and these have never heard of would otherwise stage `.app` from an empty
# name, against a plist that is not there -- a release with a bundle nobody can name rather than a
# build that stopped. The arm is a diagnostic rather than the guard: two of the three are read in
# argument position, where a subshell's `exit` does not reach the script. `tool_preconditions` below
# is what actually stops the run, and it does it before anything is built.
bundle_name()  {
  case "$1" in
    km-package-builder) printf 'KM Package Builder' ;;
    km-remote)          printf 'KM Remote' ;;
    km-admin)           printf 'KM Admin' ;;
    *) echo "dist-tools: no bundle name for $1" >&2; exit 1 ;;
  esac
}
bundle_plist() {
  case "$1" in
    km-package-builder) printf 'tools/platform/macos/Info.package-builder.plist' ;;
    km-remote)          printf 'tools/platform/macos/Info.remote.plist' ;;
    km-admin)           printf 'tools/platform/macos/Info.admin.plist' ;;
    *) echo "dist-tools: no bundle plist for $1" >&2; exit 1 ;;
  esac
}
bundle_icns()  {
  case "$1" in
    km-package-builder) printf 'icon/km-package-builder.icns' ;;
    km-remote)          printf 'icon/km-remote.icns' ;;
    km-admin)           printf 'icon/km-admin.icns' ;;
    *) echo "dist-tools: no bundle icon for $1" >&2; exit 1 ;;
  esac
}

# `km-wallpaper-pack` and `km-admin` are in the second workspace, `tools/cmd/assets` -- see the
# `exclude` note in the root Cargo.toml -- so a bare `-p` cannot reach them and a manifest has to be
# named. That one fact is recorded here, once, so the staging loop below is identical for all of them.
#
# **The manifest named is the workspace root and `-p` picks the member**, which is what keeps this a
# two-line table as members are added rather than one line per program.
#
# **Where that member's executable lands is deliberately not written down here.** "Its own `target/`
# rather than the workspace's" is true only when nothing has moved the target directory, and
# something usually has: `dist_target_dir` asks cargo per manifest instead. See its comment in
# tools/dist/common.sh.
#
# It is staged as a release build for a reason beyond tidiness: its `analyze` phase is a few thousand
# JPEG decodes, resizes and blurs, and an unoptimized one over a full cache took 78 minutes against
# single-digit ones here. A `cargo run` of it is a development convenience; this is the build to use.
tool_manifest() { # <tool>  -> prints the manifest that defines it
  # **The `-console` spellings are here on purpose.** `tool_exe` asks this about `<tool>-console`
  # when it goes looking for the Windows twin, and a case that named only the plain tool sent it to
  # the main workspace's `target/` — where `km-admin-console.exe` is not, and never will be. It
  # went unnoticed for `km-package-builder-console` because that one really is in the main
  # workspace, so the fall-through happened to be right.
  case "$1" in
    km-wallpaper-pack|km-admin|km-admin-console) printf 'tools/cmd/assets/Cargo.toml' ;;
    *)                                          printf 'Cargo.toml' ;;
  esac
}
tool_build_args() { # <tool>  -> prints the cargo arguments that select it
  case "$1" in
    km-wallpaper-pack|km-admin)
      printf -- '--manifest-path tools/cmd/assets/Cargo.toml -p %s' "$1" ;;
    *) printf -- '-p %s' "$1" ;;
  esac
}
tool_exe() { # <tool> <exe extension>  -> prints where cargo will have put it
  printf '%s/release/%s%s' "$(dist_target_dir "$(tool_manifest "$1")")" "$1" "$2"
}

ZIP=0
VIDEO=1
DESKTOP=1
TOOLS=()
for arg in "$@"; do
  case "$arg" in
    --zip) ZIP=1 ;;
    --no-video) VIDEO=0 ;;
    # Declines the window on km-package-builder and km-remote, leaving each the browser-served
    # tool it has always been. Nothing else has one. Named on the declined build below, as
    # `-no-video` is, because the plain name should name what the plain command produces.
    --no-desktop) DESKTOP=0 ;;
    # The build logs, which are quiet by default. This one matters most here: a run with no arguments
    # builds six tools, so it is six cargo compile streams that are being held back rather than one.
    # A failure replays whatever was suppressed, so `-v` is for watching rather than diagnosing.
    -v|--verbose) DIST_VERBOSE=1 ;;
    -*) echo "dist-tools: unknown option $arg" >&2; exit 2 ;;
    *)
      known=0
      for t in "${ALL_TOOLS[@]}"; do [ "$arg" = "$t" ] && known=1; done
      if [ "$known" -eq 0 ]; then
        echo "dist-tools: unknown tool $arg (known: ${ALL_TOOLS[*]})" >&2
        exit 2
      fi
      TOOLS+=("$arg")
      ;;
  esac
done
if [ "${#TOOLS[@]}" -eq 0 ]; then TOOLS=("${ALL_TOOLS[@]}"); fi

TARGET="$(dist_host_triple)"
PLATFORM="$(dist_platform "$TARGET")"
EXT="$(dist_exe_ext "$TARGET")"

# -- video, unless it was declined -------------------------------------------------------------------

# Resolved before anything is built, so a missing install costs a second rather than a release build.
#
# Gated on `video_wanted` rather than on `$VIDEO` alone, and that is the difference between a default
# and a demand: `tools/dist/cmd.sh km-lyrics` asks for nothing that could take the feature, so it
# must go on working on a machine that has never seen ffmpeg. Only a run that would actually build
# something with video has to have one.
#
# Two questions, not one, and which one gets asked is now a two-to-one split rather than a
# Windows-and-everyone-else one. **Windows and macOS both ask *where* ffmpeg is**, because both are
# about to copy libraries out of it: Windows because its loader searches the executable's own
# directory, macOS because the install names in the copies get rewritten to `@rpath` (see
# `dist_stage_ffmpeg_macos`). Linux asks only *whether* there is one to build against -- there the
# libraries are a package dependency, which is what the .deb declares and what a bare folder cannot.
#
# **The *shape* of the two questions differs, and asking the Windows one everywhere is what breaks
# this script off Windows:** it rejects Homebrew's ffmpeg for having no `bin/avcodec-61.dll` in it
# and blames the ffmpeg, and on Linux reports an install missing that pkg-config can see. So macOS
# asks `dist_ffmpeg_check` first, as Linux does, and only then looks for the prefix.
video_wanted() { # -> true when at least one selected tool would be built with the feature
  local t
  [ "$VIDEO" -eq 1 ] || return 1
  for t in "${TOOLS[@]}"; do video_capable "$t" && return 0; done
  return 1
}

FFMPEG_DIR=""
if video_wanted; then
  dist_step video
  if [ "$PLATFORM" = "windows" ]; then
    FFMPEG_DIR="$(dist_ffmpeg_dir)"
    dist_ffmpeg_on_path "$FFMPEG_DIR"
    dist_detail "ffmpeg  $FFMPEG_DIR"
  elif [ "$PLATFORM" = "macos" ]; then
    # macOS needs the *where* too now, not only the *whether*: the libraries are copied into the
    # folder and their load commands rewritten, exactly as on Windows. Resolved through the same one
    # place tools/platform/macos/app-bundle.sh uses, so the two cannot disagree about which ffmpeg -- and
    # therefore which license -- a release was built against.
    FFMPEG_DIR="$(dist_ffmpeg_dir_macos)"
    dist_detail "ffmpeg  $FFMPEG_DIR"
  else
    dist_ffmpeg_check
    # Said once, here, rather than discovered on the receiving machine. Linux is now the only platform
    # where this is true: its ffmpeg is a package dependency, which is what the .deb declares and what
    # a bare folder cannot.
    #
    # **Printed at every verbosity, unlike the two `ffmpeg <dir>` lines above.** Those say which build
    # this one linked, which is a question you go looking for; this one says the folder about to be
    # produced will not start elsewhere, which is a thing to be told.
    echo "   note    on $PLATFORM these folders are not self-contained: the machine you copy them"
    echo "           to needs ffmpeg's shared libraries installed."
  fi
fi

# -- the notes for whoever receives a folder ---------------------------------------------------------
#
# One function per tool, each taking the file to write and the `-video` suffix (empty for a plain
# build). Kept here rather than in tools/dist/common.sh because these are the part a person actually
# reads: a shared template with three substitutions would be worse than three honest notes.
#
# Quoted heredocs throughout -- they contain Windows paths full of backslashes, which an unquoted one
# would be at liberty to eat.
#
# The paragraph below is the one exception on both counts -- shared, and unquoted. Shared because what
# a video build needs at run time is a different sentence on Windows than anywhere else and both
# km-pack and km-package-builder need whichever one applies: two folders disagreeing about where
# their ffmpeg comes from is exactly the kind of thing nobody notices until a folder has been
# copied to another machine and will not start. Unquoted because the tool's name has to be
# substituted into it, which is safe here for the reason the rule exists -- there is not a backslash
# in either version.
video_runtime_note() { # <file> <tool name>
  if [ "$PLATFORM" = "windows" ]; then
    cat >> "$1" <<NOTE

The four avcodec/avformat/avutil/swresample DLLs beside $2.exe are what read them. They must
stay in this folder; without them the tool will not start at all, with an error naming one of the
files rather than anything about video.
NOTE
  elif [ "$PLATFORM" = "macos" ]; then
    cat >> "$1" <<NOTE

The libraries that read them are in the lib/ folder beside $2, and this folder needs no ffmpeg
installed on the machine you copy it to. Keep lib/ where it is: $2 finds it relative to itself,
and without it the tool will not start at all, with an error naming one of the libraries rather
than anything about video.
NOTE
  else
    cat >> "$1" <<NOTE

Reading them needs ffmpeg's shared libraries, and they come from the machine rather than from this
folder -- there is nothing here to copy them out of. Install ffmpeg (Homebrew on macOS, the
distribution's own packages on Linux); without them $2 will not start at all, with an error
naming one of the libraries rather than anything about video.
NOTE
  fi
}

readme_km_pack() { # <file> <video: 1 or 0>
  cat > "$1" <<'README'
km-pack
=======

Build, inspect and validate .kmpkg song packages for karaokemachine. A package is one file holding
the songs, their numbers and everything the machine knows about them; it is what you install on a
machine rather than a folder of loose MIDI files.

Running it
----------

Two steps, and the middle one is a file you can read.

    km-pack spec <folder> --out vol1.kmspec.yaml
    km-pack build vol1.kmspec.yaml

<folder> is scanned recursively for .mid, .kar, video and MP3+G files. Every song is parsed,
analyzed and given a number, and what was decided is written into vol1.kmspec.yaml -- a plain YAML
list of the songs with their titles, artists, languages and numbers. Open it, correct anything that
is wrong, and build it. The result is one .kmpkg you can install over the control API.

Correcting a title in that file is all it takes: the build compares what you wrote against what the
file itself says, and records the difference as a correction, so a later rebuild keeps it.

`build` takes a description and nothing else. Keep the description beside the songs and it is the
record of what is in the package.

    km-pack check vol1.kmpkg

validates a package and reports anything wrong with it -- songs that will not parse, songs with no
lyrics, songs scoring badly. Worth running before handing a package to anyone.

Commands
--------

    spec <folder> --out FILE      describe a folder, for editing and then building
    build <description>           build the package a description names
    inspect <package>             list what a package contains
    check <package>               validate it and report what is wrong
    reanalyze <package>           re-run analysis, writing a new package
    export <package>              write the song metadata to a CSV for editing
    apply <package> --index CSV   apply an edited CSV back to a package
    edit <package> --number N     correct one song's title, artist, encoding or transposition
    --version

Options on `build`
------------------

    --out FILE              write here, rather than where the description says
    --dry-run               report what would be built without writing anything

Options on `spec`
-----------------

    --out FILE              where the description goes; default <folder>/<id>.kmspec.yaml
    --id ID                 package identifier; reinstalling the same id replaces it, so keep it
                            stable across rebuilds. Defaults to the folder's own name
    --name NAME             display name
    --package-version V     defaults to 1.0.0
    --publisher NAME        who made it
    --start-number N        first song number to assign, default 1
    --package-out FILE      what the description should name as the package; default <id>.kmpkg
    --default-language CODE file any song whose language could not be worked out under this code.
                            A package cannot ship a song with no language; `und` is the standard's
                            own "undetermined" and is the honest answer for a mixed folder
    --index CSV             a CSV with a `file` column plus any of number, title, artist, encoding,
                            to override what is read from the files. Mostly superseded by the
                            description itself, which says all of this and can be edited in place
    --from OLD.kmpkg        seed hand-edited titles, artists and numbers from an existing package,
                            matched by content hash
    --min-suitability N     leave out songs scoring below N out of 10
    --require-lyrics        leave out files with no lyrics at all, which cannot be sung from
    --encoding NAME         force the lyric encoding rather than detecting it
    --limit N               stop after N songs
    --dry-run               walk the folder and report, without writing the description

`--dry-run` is on every command that writes a file: `spec`, `build`, `apply`, `edit` and
`reanalyze`. On `reanalyze` it is also the one run that needs no `--out`.

The 0-10 suitability rates the MIDI *file* -- whether it has lyrics, whether they are timed, and
whether there is a melody to follow. It says nothing about anybody's singing.
README
  if [ "$2" -eq 1 ]; then
    cat >> "$1" <<'README'

Video songs
-----------

This build packages video songs (.mp4) as well as MIDI ones. A video song is a file with the words
burned into the picture, so nothing is highlighted over it -- the machine plays the file and shows
it.

Each video is checked against one profile -- H.264 in 8-bit 4:2:0, at most 1080p30, AAC, MP4 -- and
copied byte-for-byte when it already matches, which a download normally does. Only a file outside
the profile is re-encoded, and that needs a plain `ffmpeg` on your PATH: what this tool links
decodes but does not encode.

    km-pack build vol1.kmspec.yaml

writes vol1.kmpkg, holding every video the description named as a stored entry inside it. One file
to copy, and one to hand somebody.

Re-encoding is on unless the description turns it off, which `spec` writes for you:

    km-pack spec <folder> --out vol1.kmspec.yaml --no-transcode

stores irregular files as they are rather than re-encoding them. A file the machine could not play
at all is still refused rather than packaged. To change your mind, set `transcode:` in the
description's `package` block rather than describing the folder again.
README
    video_runtime_note "$1" km-pack
  else
    cat >> "$1" <<'README'

No video songs
--------------

This is the --no-video build. It packages MIDI only; `build` reports any video files it finds as
skipped rather than passing over them in silence. The ordinary build of this tool packages them.
README
  fi
  cat >> "$1" <<'README'

Licenses
--------

km-pack is MIT OR Apache-2.0, at your option; both texts are beside this file.
README
}

readme_km_lyrics() { # <file> <video: 1 or 0>
  cat > "$1" <<'README'
km-lyrics
=========

Show what karaokemachine makes of a karaoke MIDI file: the lyric timeline it parsed, the encoding it
guessed, the melody it found and the score it gave the file. A debugging tool -- reach for it when a
song's words come out wrong, arrive at the wrong moment, or do not arrive at all.

Running it
----------

    km-lyrics dump song.kar

prints the parsed lyric timeline and the file's analysis in readable form.

    km-lyrics scan <folder>

walks a folder recursively and summarizes what parsed, what did not, and why. Useful for sizing up a
collection before packaging it.

Commands and options
--------------------

    dump <file>             show what one file parsed to
      --json                emit JSON instead of readable text
      --encoding NAME       force a lyric encoding, as a package manifest would (for example
                            windows-1252). This is how you confirm a fix before writing it into a
                            package.
      --events              include the channel event list, which is usually very long
      --raw                 list every text meta event as it appears in the file, uninterpreted --
                            the last resort when the parsed view looks nothing like the file

    scan <folder>           parse everything under a folder and summarize
      --limit N             stop after N files
      --ext mid,midi,kar    which extensions to consider
      --jobs N              worker threads, default 8
      --failures FILE       write the path of every failing file here
      --examples N          show N example paths per failure reason, default 3

    --version

Words in the wrong character set
--------------------------------

A karaoke MIDI file says nothing about how its text is encoded, so the encoding is a guess. When the
guess is wrong the words come out as mojibake. `dump --encoding <name>` re-reads the file as if the
package had specified that encoding, which is how you find the right one; `km-pack edit --encoding`
is how you then record it.

Licenses
--------

km-lyrics is MIT OR Apache-2.0, at your option; both texts are beside this file.
README
}

readme_km_remote() { # <file> <video: 1 or 0> <console twin: 1 or 0> <macOS bundle: 1 or 0>
  cat > "$1" <<'README'
km-remote
=========

The karaoke remote, with your own copy of the song list.

Point it at a karaoke machine once and it copies the whole catalog onto this device. After that,
browsing, searching and your favorites all work **whether or not the machine is
switched on** -- which is the point of it. When the machine is on, the same pages queue songs, show
what is playing and drive the controls.
README
  # Only the folder that has two, which is the Windows one with a window. Saying it anywhere else
  # would send somebody looking for a file that is not there.
  if [ "${3:-0}" -eq 1 ]; then
    cat >> "$1" <<'README'

If there is a km-remote-console.exe beside km-remote.exe, that is the same program with somewhere
to print, for when you need to see why something did not start. Every command below works with
either name.

The window is a phone-shaped one, because the page is: a single column with the three tabs along the
bottom. Close it and the remote stops -- there is no Quit button on the page, because that page is
also what this will be on a phone one day and a Quit tab would be wrong there.
README
  fi
  # macOS gets the same two forms under different names: the bundle beside this folder is the one with
  # a Dock icon, and the executable in the folder is the one to type. Said only where the bundle was
  # actually staged, so a --no-desktop folder does not point at a file that is not there.
  if [ "${4:-0}" -eq 1 ]; then
    cat >> "$1" <<'README'

The application beside this folder
----------------------------------

    KM Remote.app

That bundle is this same program with a Dock icon and a window of its own. Double-click it; there is
nothing to run first and nothing to register.
README
    # Only for the build it is true of: an ad-hoc bundle needs the quarantine flag cleared, a signed
    # one does not, and printing the incantation anyway teaches somebody to do it by reflex.
    if ! dist_signing; then
      cat >> "$1" <<'README'

It is signed only ad-hoc, so on any Mac other than the one that built it macOS will refuse to open
it until you clear the quarantine flag:

    xattr -dr com.apple.quarantine "KM Remote.app"

The first launch is also slow while macOS assesses it, and immediate afterwards.
README
    fi
    cat >> "$1" <<'README'

macOS will ask once for permission to find devices on your local network. That is how it finds a
karaoke machine at all. Declining it leaves the song list, the search and your favorites working --
they are answered from this device's own copy -- and stops discovery, queueing and refreshing.

The km-remote executable in this folder is the same program for a terminal, and every command
below is typed against it.
README
  fi
  cat >> "$1" <<'README'

Running it
----------

    km-remote

A build with a window opens one. Otherwise, open the address it prints --
http://127.0.0.1:8179/ -- in a browser. It looks for a machine on the network by itself and
remembers where it found one, so after the first run there is nothing to type.

    km-remote --machine 192.168.1.5

names one, for a network where discovery does not work or where there is more than one.

Options
-------

    --machine <address>     where the karaoke machine is; found automatically if omitted
    --data-dir <folder>     where to keep the song list and the favorites
    --port <number>         the port to serve on (default 8179)
    --lan                   serve to the whole network rather than only this device
    --open                  open a browser once it is up
    --browser               show the remote in your browser rather than a window of its own
    --refresh               re-read the whole song list even if nothing has changed

What it keeps, and where
------------------------

Two files, in this platform's data folder unless --data-dir says otherwise:

    catalog.sqlite        the copy of the machine's song list
    favorites.sqlite       your folders, and the songs in them

They are separate on purpose. Refreshing the song list may throw the first one away and rebuild it;
a collection you have built up over a year must not be able to go with it.

A refresh costs nothing when nothing has changed: the machine reports a version number that moves
only when a package is installed or removed, and this stops there if the copy already matches.

--lan
-----

Off by default, and that is not the same choice the karaoke machine makes. The machine's own remote
is open to the house because anybody in the room should be able to queue a song. This one holds your
personal favorites, so it answers only to this device until you say otherwise.

Notes
-----

* A machine with an admin password set is not supported yet: browsing works, queueing will report
  "not authorized", and there is nowhere to type one. Every route it uses is public in a machine's
  shipped configuration.
* Nothing here reaches the internet. It talks to one karaoke machine on your own network and to
  nothing else.

--------

km-remote is MIT OR Apache-2.0, at your option; both texts are beside this file.
README
}
readme_km_package_builder() { # <file> <video: 1 or 0> <console twin: 1 or 0> <macOS bundle: 1 or 0>
  cat > "$1" <<'README'
km-package-builder
==================

Browse a folder of karaoke files (.mid / .kar) and hand-curate them into .kmpkg packages for
karaokemachine. It is a small web server: start it, then open the address it prints.
README
  # Only the folder that has two, which is the Windows one with a window. Saying it anywhere else
  # would send somebody looking for a file that is not there, and for a good reason -- see the twin's
  # own comment beside the `cp` below.
  if [ "${3:-0}" -eq 1 ]; then
    cat >> "$1" <<'README'

If there is a km-package-builder-console.exe beside km-package-builder.exe, that is the same
program with somewhere to print, for when you need to see why something did not start. Every
command below works with either name.
README
  fi
  cat >> "$1" <<'README'

Running it
----------

    km-package-builder

Start it with nothing and it opens on a page that asks which folder you want: the ones you have
opened before, a browser to find a new one, and a box to paste a path into. You can change folders
from the header afterwards without restarting.

Naming the folder on the command line still works and is unchanged:

    km-package-builder <folder> --init

<folder> is the folder holding your karaoke files. --init creates the tool's database inside it,
and is only needed the first time; afterwards, plain `km-package-builder <folder>` is enough.
Without --init it refuses a folder that has no database, so pointing it at the wrong place is an
error rather than an empty index that looks like your files have vanished. The Open page's
"Create here" button is the same thing, and the only other way a database is ever made.

The database is called km-package-builder.kmbuild and it is the file you can double-click to come
back to a corpus.
README
  # How that is arranged differs by platform, and on macOS it is not a command at all -- the file type
  # is declared in the bundle's own Info.plist and LaunchServices reads it when the .app is placed. So
  # the folder points at the bundle beside it rather than telling somebody to run --register, which
  # there only nudges lsregister and does nothing a person who just unzipped this needs.
  if [ "${4:-0}" -eq 1 ]; then
    cat >> "$1" <<'README'

Open the application beside this folder once and it works from then on:

    KM Package Builder.app

That bundle is this same program with a Dock icon and a window. It is what declares the .kmbuild
file type to macOS, so double-clicking a corpus opens it -- and if one is already open, the folder
is swapped in the window you have rather than a second copy starting.

README
    if ! dist_signing; then
      cat >> "$1" <<'README'

The bundle is signed only ad-hoc, so on any Mac other than the one that built it macOS will refuse
to open it until you clear the quarantine flag:

    xattr -dr com.apple.quarantine "KM Package Builder.app"
README
    fi
    cat >> "$1" <<'README'

The km-package-builder executable in this folder is the same program for a terminal, and every
command below is typed against it.
README
  else
    cat >> "$1" <<'README'

Run this once to make that work:

    km-package-builder --register

That associates .kmbuild files with this copy of the program, for your user only -- it needs no
administrator rights and changes nothing for anyone else who logs in. It records where the program
is right now, so run it again if you move this folder. --unregister undoes it.
README
  fi
  cat >> "$1" <<'README'

Started with no folder, it reopens whichever one you had open last, since most people work on one
corpus. --pick asks for the list instead.

Then open http://127.0.0.1:8178/ and use the Scan page, or start it with --scan to begin at once.
A first scan of ten thousand files takes a few seconds; scanning again afterwards reads nothing,
because a file whose size and date have not changed is skipped.

The executable is self-contained: the page, its script and its stylesheet are all compiled into
it. Copy km-package-builder anywhere on its own and it works -- there is no folder to keep beside
it and nothing that can be left behind.

It never writes to your karaoke files. It reads them, keeps what it found in
km-package-builder.kmbuild inside that same folder, remembers what you decided, and writes packages.

Options
-------

    --init                create the database in this folder (first run only)
    --scan                start a scan as soon as the server is up
    --open                open a browser at the address
    --pick                start on the Open page instead of reopening last time's folder
    --browser             show the page in your browser rather than a window of its own
    --reanalyze           read every song again and update what the analysis says, then exit
                          (on Windows, run km-package-builder-console for this one)
    --register            make .kmbuild files open with this program (per-user), then exit
    --unregister          undo that, then exit
    --port N              listen on N instead of 8178
    --lan                 listen on every interface, not just this machine
    --machine URL         the running karaoke machine, default http://127.0.0.1:8177
    --version

There is no password on this tool, and it can open files and write packages as you. It listens
only on this machine unless you pass --lan, and it warns you when you do.

Playing a song on the karaoke machine
-------------------------------------

The play button asks a running karaokemachine to play the file. That machine refuses to play
anything outside the folders listed in its own settings, and that list starts empty -- so the
first attempt is refused. The refusal tells you exactly which folder to add and where; follow it
and restart the machine.
README
  if [ "$2" -eq 1 ]; then
    cat >> "$1" <<'README'

Video songs
-----------

This build scans, browses, rates and packages video songs (.mp4) beside MIDI ones. A build without
the feature still gives each video a row, saying it cannot read the file rather than passing over it
in silence.
README
    video_runtime_note "$1" km-package-builder
    cat >> "$1" <<'README'

Note that the first open of a database made before video support rewrites its `songs` table -- once,
and it carries every hand-set field across. Back up a large corpus's .sqlite first.
README
  else
    cat >> "$1" <<'README'

No video songs
--------------

This is the --no-video build. It scans, browses, rates and packages MIDI only. Video files still get
a row, saying "a video, and this build has no `video` feature to read it with" rather than being
passed over in silence -- so a folder of MP4s indexed here is telling you which build you have, not
that anything is wrong with the files. The ordinary build of this tool reads them.
README
  fi
  cat >> "$1" <<'README'

Licenses
--------

The bundled copy of htmx is unmodified. Its terms are served by the tool itself, at
/static/htmx-LICENSE.txt while it is running.
km-package-builder is MIT OR Apache-2.0, at your option; both texts are beside this file.
README
}

readme_km_admin() { # <file> <video: 1 or 0> <console twin: 1 or 0> <macOS bundle: 1 or 0>
  cat > "$1" <<'README'
km-admin
========

KM Admin: finds pictures and instrument banks for a karaoke machine, and sends them to
it.

The machine can be given files and cannot go and find any. It may have no way onto the internet, it
should not be holding your accounts with anybody, and looking through a few hundred photographs is
not what a box under a television should be doing while it is meant to be playing a song. This is
the other half of that: it runs on a desktop, does the looking, and hands over the result.

Everything it makes is written into its own folder **and** offered to the machine, so a machine that
is switched off is a delay rather than a wasted afternoon.
README
  if [ "${3:-0}" -eq 1 ]; then
    cat >> "$1" <<'README'

If there is a km-admin-console.exe beside km-admin.exe, that is the same program with somewhere to
print, for when you need to see why something did not start. Every command below works with either
name.
README
  fi
  if [ "${4:-0}" -eq 1 ]; then
    cat >> "$1" <<'README'

The application beside this folder
----------------------------------

    KM Admin.app

That bundle is this same program with a Dock icon and a window of its own. Double-click it; there is
nothing to run first and nothing to register.
README
    if ! dist_signing; then
      cat >> "$1" <<'README'

It is signed only ad-hoc, so on any Mac other than the one that built it macOS will refuse to open
it until you clear the quarantine flag:

    xattr -dr com.apple.quarantine "KM Admin.app"

The first launch is also slow while macOS assesses it, and immediate afterwards.
README
    fi
    cat >> "$1" <<'README'

The km-admin executable in this folder is the same program for a terminal, and every command below
is typed against it.
README
  fi
  cat >> "$1" <<'README'

Running it
----------

    km-admin

A build with a window opens one. Otherwise, open the address it prints -- http://127.0.0.1:8180/ --
in a browser.

    km-admin --machine 192.168.1.5

names the machine to send things to. You can also type it on the Machine page, and it is remembered.

Pictures
--------

Photographs that two lines of lyrics stay readable over. That is measured rather than judged by eye:
the contrast is computed in the exact band of the screen the words occupy, against the same
near-white the lyrics are drawn in and the same dimming the machine already lays over every picture.
Most photographs fail, which is the point -- a picture that looks fine in a browser is often
unreadable behind moving text.

Three sources, and **which one you use decides what you may do with the result**:

    Openverse    no account needed. Only CC0, Public Domain Mark and CC BY, so a pack built from
                 here may be passed on to somebody else.
    Pixabay      needs a key of your own. Their license bars distributing content "on a Standalone
                 basis" and names wallpaper as one of the forms, so a pack built from here is for
                 the machine that built it.
    Pexels       needs a key of your own. Their API guidelines bar making Pexels content available
                 as a wallpaper app, with the same consequence.

Using either of the last two under your own key, for your own machine, is what those terms permit.
This program mirrors nothing and passes nothing on; every picture is fetched from the source to you.
The page says which kind of pack you have built, on the result.

Keys are kept in memory unless you tick the box that writes one down, and then only into this
program's own folder -- never into anything you would send somebody. On macOS and Linux that file is
readable only by you; on Windows there is no per-file setting this program can make, and the page
says so rather than implying otherwise.

Sound
-----

Sixty-three General MIDI instrument banks, with what each one costs to download, what its license
says, and one line on how it sounds. Every download is checked against a digest before anything is
installed, and a file that does not match is deleted rather than kept.

A machine with its own internet connection can fetch these itself. This is for the one that cannot
-- and for a television box, where there is no other way to get a file onto it.

Options
-------

    --machine <address>     the karaoke machine to send things to
    --data-dir <folder>     where downloads, packs, keys and settings are kept
    --port <number>         the port to serve on (default 8180)
    --lan                   serve to the whole network rather than only this device
    --open                  open a browser once it is up
    --browser               show the page in your browser rather than a window of its own
    --log-file              also write this run's log to a file under the data folder

--lan
-----

Rarely what you want. This program has no password, it holds whatever keys you have given it, and it
writes files as you. It prints a warning saying so when you ask for it.

If the machine has a password
-----------------------------

Most do not, and nothing here will ask for one until something is refused. When it is, the Machine
page asks for the machine's admin password -- the one its own /admin/ page uses -- and exchanges it
for a token that lives in memory until this program stops. The password itself is never written
down.
README
}

readme_km_wallpaper_pack() { # <file> <video: 1 or 0>
  cat > "$1" <<'README'
km-wallpaper-pack
=================

Builds a wallpaper pack for karaokemachine from royalty-free stock photography, keeping only the
images lyrics stay readable over. Every candidate is measured -- a WCAG contrast ratio for white-ish
text over the band the words are drawn in, at the darkening the app already applies -- so a hundred
wallpapers are accepted or rejected without anybody squinting at them.

This one is NOT a working install on its own
--------------------------------------------

Unlike the other tools in dist/, this executable needs two things before it will do anything:

  1. A config file. Copy config.example.toml from the repository to config.toml beside the exe and
     edit the search terms. `--config PATH` points at another one.

  2. API keys, in the ENVIRONMENT and never in the config file:

         PIXABAY_API_KEY=...    https://pixabay.com/api/docs/
         PEXELS_API_KEY=...     https://www.pexels.com/api/

     A key-shaped entry in the config file is a hard error, so that a config is always safe to
     commit or to send to somebody.

Running it
----------

    km-wallpaper-pack all --config config.toml

does the three phases in order. Only the first touches the network:

    fetch      search the providers and download originals into the cache
    analyze    measure, deduplicate and select, writing analysis.json
    build      process the selection and write the pack
    verify     re-check a built pack against its own contrast gate; exits 2 if any image fails

`analyze` and `build` are pure functions of the cache, so thresholds can be re-tuned without
spending any API quota -- which is the point of the cache, given quotas measured in hundreds of
requests an hour.

Useful options
--------------

    --out DIR           where the pack is built, default ./out
    --cache-dir DIR     the cache, default ./.wpcache
    --zip-dest DIR      where the finished zip is copied; point it at --out for no copy
    --remeasure         re-decode everything instead of reusing cached measurements. Only needed
                        when the tool's own measuring code changes -- a config change invalidates
                        what it needs to by itself
    --jobs N            how many images are measured at once, default one per core
    --dry-run           report what would happen and write nothing anywhere
    --json              machine-readable output on stdout

What it produces
----------------

The deliverable is the zip, not this executable. karaokemachine reads a zip sitting in its wallpaper
folder as if it were a folder of images, so a pack is one file to drop in and one file to remove.
`--out` also holds the loose images, a manifest, and ATTRIBUTION.md -- which is not optional: both
providers require credit, and the pack carries it.

A note on resolution
--------------------

Ask for the size the display is, not the largest number available. Pixabay's API serves images
capped at 1280 pixels on the longest side regardless of how large the original photograph is, so
configuring a 4K output there upscales every image rather than sharpening it. `build` warns per
image when the source is narrower than the output size.

Licenses
--------

km-wallpaper-pack is MIT OR Apache-2.0, at your option; both texts are beside this file. The photographs
it downloads are not: each provider's license applies, and ATTRIBUTION.md in every pack records who
took what.
README
}

# -- what every selected tool has to have before any of them is built --------------------------------
#
# Resolved here for the reason the ffmpeg question above is: a missing piece should cost a second
# rather than a release build. This one is later in the file only because it asks about functions,
# which have to have been defined first; it still runs before the first `cargo build` below.
#
# **What it catches is a rename that moved a string and not a definition.** The staging loop spells
# each tool's README function by turning its hyphens into underscores, so the name never appears in
# this file as itself -- `bash -n` sees nothing, shellcheck sees nothing, and every tool staged
# before the broken one succeeds. `km-wallpaper-pack` is the whole argument: it is staged last, and
# when `wallpaper-pack` took the family name everywhere except its own `readme_` definition, a
# `task dist:setup` built every tool ahead of it and the machine's application bundle and then
# stopped on `readme_km_wallpaper_pack: command not found`.
#
# The bundle tables are checked here too, and for the same reason: `bundle_plist` and `bundle_icns`
# are read in argument position, where their own default arm cannot stop anything.
readme_fn() { printf 'readme_%s' "$(echo "$1" | tr - _)"; }

tool_preconditions() {
  local t fn missing=0
  for t in "${TOOLS[@]}"; do
    fn="$(readme_fn "$t")"
    if ! declare -F "$fn" >/dev/null; then
      echo "dist-tools: $t has no README function -- expected $fn" >&2
      missing=1
    fi
    bundle_capable "$t" || continue
    if [ -z "$(bundle_name "$t")" ] \
       || [ ! -f "$(bundle_plist "$t")" ] \
       || [ ! -f "$(bundle_icns "$t")" ]; then
      echo "dist-tools: $t is staged as a macOS bundle, but its name, plist or icon is missing" >&2
      missing=1
    fi
  done
  [ "$missing" -eq 0 ] || exit 2
}
tool_preconditions

# -- the tools -------------------------------------------------------------------------------------

staged=()
# The macOS bundles, which are a second artifact for the same tool rather than another entry above.
staged_apps=()

for tool in "${TOOLS[@]}"; do
  FEATURES=()
  features=""
  suffix=""
  video=0
  desktop=0
  # Whether a `.app` is staged beside this folder. Decided here so the staged README can say so; the
  # bundle itself is built at the end of the loop, after the folder is finished.
  bundle=0
  if video_capable "$tool"; then
    if [ "$VIDEO" -eq 1 ]; then
      features="video"
      video=1
    else
      # The two are different products in the same version -- one handles a kind of song the other
      # cannot -- so they cannot share a folder: the second staging would silently replace the first,
      # and `--zip` would overwrite a zip that had already been sent to somebody. **The marker is on
      # the declined build**, because video is the default and the plain name should name the thing
      # the plain command produces. It spells the flag exactly, so the folder is guessable from the
      # command that made it.
      suffix="-no-video"
    fi
  fi
  # km-lyrics and km-wallpaper-pack get no marker either way: they have no such feature, so naming one
  # would describe a choice they were never offered.

  if desktop_capable "$tool"; then
    if [ "$DESKTOP" -eq 1 ]; then
      features="${features:+$features,}desktop"
      desktop=1
    else
      # Same rule as `-no-video`, and for the same reason: the two are different products and must
      # not share a folder. A tool asked for both declines lands in `-no-video-no-desktop`, which is
      # ugly and is exactly what was asked for.
      suffix="$suffix-no-desktop"
    fi
  fi
  # On Linux `desktop_capable` is false, so no marker appears there whichever way the flag went --
  # naming a choice that platform was never offered would be a lie in a folder name.

  # A bundle needs both: the platform, and a window to receive an Apple Event in. Decided here rather
  # than beside `bundle_capable` because `desktop` is not known until now, and the staged README says
  # which of the two artifacts it is sitting in.
  if bundle_capable "$tool" && [ "$desktop" -eq 1 ]; then bundle=1; fi

  [ -n "$features" ] && FEATURES=(--features "$features")

  dist_step "build $tool"
  build_started=$SECONDS
  # Word-split on purpose: `tool_build_args` prints either `-p <name>` or `--manifest-path <path>`,
  # and `dist_cargo_quiet` prints one flag or nothing at all.
  # shellcheck disable=SC2046
  cargo build --release $(dist_cargo_quiet) $(tool_build_args "$tool") "${FEATURES[@]+"${FEATURES[@]}"}"
  # Per tool rather than once at the end, and this is the loop that most needs it: six quiet builds
  # in a row is otherwise several silent minutes with nothing saying which one is running.
  printf '   built in %s\n' "$(dist_elapsed "$build_started")"

  EXE="$(tool_exe "$tool" "$EXT")"
  if [ ! -f "$EXE" ]; then
    echo "dist-tools: $EXE was not produced" >&2
    exit 1
  fi

  VERSION="$(dist_version "$EXE")"

  NAME="$tool-$VERSION-$TARGET$suffix"
  PARENT="$(dist_dir "$tool" "$PLATFORM")"
  DEST="$PARENT/$NAME"

  dist_clear "$DEST"
  cp "$EXE" "$DEST/$tool$EXT"

  # **The console twin, and only on Windows.** With a window, `km-package-builder` is built
  # GUI-subsystem there -- which is what stops a console flashing past on the way to the window, and
  # is also what stops it answering `--help`, printing a version, or saying why it refused to start.
  # `km-package-builder-console` is the same program with none of that taken away and no window; it is
  # what a person at a terminal should run. See the `Two executables on Windows` row in docs/decisions/.
  #
  # macOS gets one file, deliberately: it has no subsystem to choose, so the windowed executable there
  # still prints to a Terminal that started it and a twin would be a second identical program. Linux
  # never has the feature at all, so the question does not arise.
  twin=0
  if [ "$desktop" -eq 1 ] && [ "$PLATFORM" = "windows" ]; then
    TWIN="$(tool_exe "$tool-console" "$EXT")"
    if [ ! -f "$TWIN" ]; then
      echo "dist-tools: $TWIN was not produced" >&2
      exit 1
    fi
    cp "$TWIN" "$DEST/$tool-console$EXT"
    twin=1
  fi

  dlls=0
  dylibs=0
  if [ "$video" -eq 1 ] && [ "$PLATFORM" = "windows" ]; then
    dlls="$(dist_stage_ffmpeg "$DEST" "$FFMPEG_DIR")"
  fi
  # The macOS equivalent, and it takes more than a copy: the load commands are absolute Homebrew paths
  # and have to be rewritten to @rpath, or the folder plays video only on the machine that built it.
  # A subfolder rather than beside the exe, because it is thirteen libraries and the exe is one file.
  if [ "$video" -eq 1 ] && [ "$PLATFORM" = "macos" ]; then
    dylibs="$(dist_stage_ffmpeg_macos "$DEST/$tool$EXT" "$DEST/lib" "@executable_path/lib")"
  fi

  "$(readme_fn "$tool")" "$DEST/README.txt" "$video" "$twin" "$bundle"

  # Every folder, whether or not it carries ffmpeg: these are the *application's* terms, and every
  # README above already names them. See `dist_stage_app_licenses` for why naming is not enough.
  # **A tool with no video feature is never signed by anything else**, and that is the gap this
  # closes: on macOS the executable only ever reached `codesign` as the last step of
  # `dist_stage_ffmpeg_macos`, which runs only for the three tools that link ffmpeg. km-lyrics,
  # km-wallpaper-pack and km-remote were staged by a plain `cp` and kept whatever the linker gave
  # them -- fine while everything was ad-hoc, and a mixed folder the moment an identity is set.
  # Only when signing, so an unsigned build stages exactly the bytes it always did.
  if [ "$PLATFORM" = macos ] && dist_signing; then
    dist_codesign "$DEST/$tool$EXT"
  fi

  dist_stage_app_licenses "$DEST"

  # The LGPL notice goes on the end of whichever README was just written, so the three do not each
  # have to remember it.
  if [ "$dlls" -gt 0 ]; then
    dist_ffmpeg_license_note "$FFMPEG_DIR" >> "$DEST/README.txt"
  fi
  # A different note, because a different ffmpeg: see dist_stage_ffmpeg_license_macos on why the one
  # above cannot be reused here. The terms go in lib/, beside the libraries they cover -- there is no
  # bundle here, so nothing objects to a text file sitting among them.
  if [ "$dylibs" -gt 0 ]; then
    dist_stage_ffmpeg_license_macos "$FFMPEG_DIR" "$DEST/lib" "$DEST/lib" >> "$DEST/README.txt"
  fi

  total="$(dist_bytes "$DEST")"
  printf 'staged %s\n' "$DEST"
  printf '  %s%s %s\n' "$tool" "$EXT" "$VERSION"
  if [ "$twin" -eq 1 ]; then
    printf '  %s-console%s -- the same program for a terminal: --help, --version, no window\n' \
      "$tool" "$EXT"
  fi
  if [ "$dlls" -gt 0 ]; then
    printf '  %s ffmpeg DLL(s) + ffmpeg-LICENSE.txt\n' "$dlls"
  fi
  if [ "$dylibs" -gt 0 ]; then
    printf '  %s ffmpeg dylib(s) in lib/, + their terms\n' "$dylibs"
  fi
  # Said for every tool that *could* have taken the feature, including off Windows where no DLL count
  # gives it away. Which of the two builds you are holding is the thing this script is now most able
  # to get wrong quietly.
  if video_capable "$tool"; then
    if [ "$video" -eq 1 ]; then
      printf '  video yes\n'
    else
      printf '  video no  -- video files are listed and refused, not read\n'
    fi
  fi
  # The same reasoning as the video line above: which of the two you are holding is invisible from the
  # outside, so it is said rather than left to be discovered. On Linux nothing is printed, because
  # nothing was offered.
  if desktop_capable "$tool"; then
    if [ "$desktop" -eq 1 ]; then
      printf '  window yes -- opens in a window of its own; --browser for a browser tab\n'
    else
      printf '  window no  -- serves its page to your browser, as it always has\n'
    fi
  fi
  printf '  %s bytes total (~%s MiB)\n' "$total" "$((total / 1024 / 1024))"

  # The claim a Windows video build makes is that the folder is self-contained, and that is worth
  # *testing* rather than asserting: the whole point of staging the DLLs is that the exe starts with
  # no ffmpeg anywhere else on the machine. So run it with a PATH stripped of everything that could be
  # hiding the failure. `--version` prints and exits, which makes it a cheap way to prove the process
  # got as far as running its own code.
  if [ "$dlls" -gt 0 ]; then
    # Both executables when there are two: they link the same DLLs, so staging that satisfied one and
    # not the other would be a folder half of which works. `--version` is answered by the
    # GUI-subsystem one as well, because a redirected standard output is a handle it inherits
    # whatever its subsystem -- see `stdout_goes_nowhere` in crates/platform/km-console.
    exes=("$tool")
    if [ "$twin" -eq 1 ]; then
      exes+=("$tool-console")
    fi
    ok=1
    for exe in "${exes[@]}"; do
      ( cd "$DEST" && PATH="/c/Windows/System32:/c/Windows" "./$exe$EXT" --version >/dev/null 2>&1 ) || ok=0
    done
    if [ "$ok" -eq 1 ]; then
      echo "  verified: starts with no ffmpeg on PATH, so the folder is self-contained."
    else
      echo "dist-tools: the staged $tool would not start with a bare PATH." >&2
      echo "            Something it links is missing from the folder; it will fail elsewhere." >&2
      exit 1
    fi
  fi
  # A desktop build with no DLLs to prove staged still has something worth proving, and it is not a
  # formality: that the **GUI-subsystem** executable answers `--version` down a redirected handle.
  # `dist_version` above stages every release by relying on exactly that, and it is the case
  # `stdout_goes_nowhere` in crates/platform/km-console exists to get right -- a process with no console still
  # has a standard output when a shell gave it a pipe, and a build that got that wrong would print
  # nothing here while working perfectly when double-clicked. km-remote is the first tool to
  # reach this arm: it has a window but stages no libraries, so before this it was checked by nothing.
  if [ "$dlls" -eq 0 ] && [ "$dylibs" -eq 0 ] && [ "$twin" -eq 1 ]; then
    ok=1
    for exe in "$tool" "$tool-console"; do
      ( cd "$DEST" && "./$exe$EXT" --version >/dev/null 2>&1 ) || ok=0
    done
    if [ "$ok" -eq 1 ]; then
      echo "  verified: both executables start and answer --version."
    else
      echo "dist-tools: a staged $tool executable would not answer --version." >&2
      echo "            A GUI-subsystem build that cannot print breaks dist_version too." >&2
      exit 1
    fi
  fi

  # The same claim on macOS, proved differently, because PATH has nothing to do with it here: a dylib
  # is found by the path in the load command, so what has to be shown is that none of those paths
  # leads outside the folder -- and then that the thing actually starts.
  if [ "$dylibs" -gt 0 ]; then
    dist_verify_macho_portable "$DEST"
    if ( cd "$DEST" && "./$tool$EXT" --version >/dev/null 2>&1 ); then
      echo "  verified: nothing loads by absolute path, and it starts. The folder is self-contained."
    else
      echo "dist-tools: the staged $tool would not start." >&2
      echo "            Something it links is missing from lib/; it will fail elsewhere too." >&2
      exit 1
    fi
  fi

  if [ "$ZIP" -eq 1 ]; then
    dist_zip "$PARENT" "$NAME"
  fi

  # -- and, on macOS, the same executable again as an application bundle ------------------------------
  #
  # **Staged from `$EXE`, never from `$DEST/$tool`.** That copy has already had its load commands
  # rewritten to `@executable_path/lib` a few lines above, so handing it to `dist_stage_ffmpeg_macos` a
  # second time would have it hunting for `@rpath/...` dependencies and finding none. Two copies of one
  # build, two different rpaths, both starting from the untouched build output.
  #
  # No second `cargo build`: this is the same binary the folder holds, in the layout macOS needs.
  if [ "$bundle" -eq 1 ]; then
    # Separated from the folder's report above rather than from the next tool below: the loop already
    # ends every tool with a blank line, so one here too gave the bundle two and the two reports for
    # this tool none between them.
    echo
    APP="$PARENT/$(bundle_name "$tool").app"
    APP_CONTENTS="$APP/Contents"

    dist_stage_macos_bundle "$APP" "$(bundle_plist "$tool")" \
                            "$EXE" "$tool" \
                            "$(bundle_icns "$tool")" "$VERSION"

    # Contents/Frameworks and `@executable_path/../Frameworks`, where the folder above uses `lib/` and
    # `@executable_path/lib`. Apple's layout rather than a preference -- it is what anything that later
    # signs or notarizes this will expect to find.
    bundle_dylibs=0
    if [ "$video" -eq 1 ]; then
      bundle_dylibs="$(dist_stage_ffmpeg_macos "$APP_CONTENTS/MacOS/$tool" \
                                               "$APP_CONTENTS/Frameworks" \
                                               "@executable_path/../Frameworks")"
      # Terms in Resources, libraries in Frameworks: `codesign` treats Frameworks as code and refuses
      # to seal a bundle with a text file in it. The folder above has no such rule and keeps both in
      # `lib/`. The directory is made here because the shell opens the redirect before the function runs.
      mkdir -p "$APP_CONTENTS/Resources/ffmpeg"
      dist_stage_ffmpeg_license_macos "$FFMPEG_DIR" "$APP_CONTENTS/Frameworks" \
                                      "$APP_CONTENTS/Resources/ffmpeg" \
        > "$APP_CONTENTS/Resources/ffmpeg/README.txt"
    fi

    # The application's own terms, on the same footing as ffmpeg's above and in the same place, for
    # the same `codesign` reason. Before the seal, so they are signed rather than added to a sealed
    # bundle. A bundle is a copy of the software like any other, and MIT asks the notice to travel in
    # every copy -- see `dist_stage_app_licenses`.
    dist_stage_app_licenses "$APP_CONTENTS/Resources"

    # Last, and it checks as well as signs -- see `dist_seal_macos_bundle`.
    dist_seal_macos_bundle "$APP"

    app_total="$(dist_bytes "$APP")"
    printf 'staged %s\n' "$APP"
    printf '  the same %s %s, as an application: a Dock icon and a double-click.\n' "$tool" "$VERSION"
    # Only the builder declares a document type; saying it of the remote would name a file type that
    # bundle knows nothing about.
    if [ "$tool" = km-package-builder ]; then
      printf '  It is what makes a .kmbuild open its corpus.\n'
    fi
    printf '  The folder above is still the command line.\n'
    if [ "$bundle_dylibs" -gt 0 ]; then
      printf '  %s ffmpeg dylib(s) in Contents/Frameworks, + their terms in Contents/Resources\n' \
        "$bundle_dylibs"
    fi
    printf '  %s bytes total (~%s MiB)\n' "$app_total" "$((app_total / 1024 / 1024))"
    # Said here rather than left to be discovered, exactly as tools/platform/macos/app-bundle.sh says it --
    # and only for the build it is true of. Telling somebody to strip quarantine off a
    # Developer-ID-signed bundle teaches a habit that defeats the point of signing it.
    if dist_signing; then
      printf '  signed %s\n' "$(dist_signing_note)"
    else
      printf '  on another Mac: xattr -dr com.apple.quarantine "%s"\n' "$APP"
    fi

    if [ "$ZIP" -eq 1 ]; then
      dist_zip_macos_bundle "$APP" "$PARENT/$tool-$VERSION-macos$suffix.zip"
    fi

    staged_apps+=("$APP")
  fi

  staged+=("$DEST")
  echo
done

# -- report ------------------------------------------------------------------------------------------

# A runnable line per tool rather than just a path. `tools/cmd/km-package-builder/src/lib.rs` has a
# test asserting that the km-package-builder line below actually parses -- `--open` was once
# documented in three places and implemented in none of them, so the first command the README told
# anybody to run failed. A flag that exists only in prose is worse than no flag at all.
echo "staged ${#staged[@]} tool(s):"
for d in "${staged[@]}"; do
  tool="$(basename "$(dirname "$(dirname "$d")")")"
  printf '  %s\n' "$d"
  case "$tool" in
    km-pack)
      # Two lines, because `build` takes a description and nothing else -- never a folder. This said
      # `build <your karaoke folder> --out vol1.kmpkg` until M19 made a package a document, and it
      # went on saying it after the staged README had been rewritten. It **parses** either way --
      # `<SPEC>` is a path and `--out` is a real option -- so it failed at run time with `Is a
      # directory`, on the first command a fresh release told anybody to type. km-pack now has a
      # test holding these two lines to what the binary accepts.
      printf '    cd %s && ./km-pack%s spec <your karaoke folder> --out vol1.kmspec.yaml\n' "$d" "$EXT"
      printf '    cd %s && ./km-pack%s build vol1.kmspec.yaml\n' "$d" "$EXT" ;;
    km-lyrics) printf '    cd %s && ./km-lyrics%s scan <your karaoke folder>\n' "$d" "$EXT" ;;
    km-package-builder)
      # **The console executable where there is one**, because these are lines to type: the windowed
      # one is GUI-subsystem on Windows and would run perfectly while appearing to say nothing at all,
      # which from a prompt reads as a program that failed. Where there is no twin the plain name is
      # already the one that prints.
      exe="km-package-builder"
      if [ "$DESKTOP" -eq 1 ] && [ "$PLATFORM" = "windows" ]; then
        exe="km-package-builder-console"
      fi
      printf '    cd %s && ./%s%s <your karaoke folder> --init --scan --open\n' "$d" "$exe" "$EXT"
      # Said once here rather than left in the README, because it is the step that turns this from a
      # program you run into one you open a corpus with, and it is easy to never find. It registers
      # the *windowed* executable whichever of the two you run it from -- a corpus double-clicked in
      # Explorer should open a window, not a browser tab. See `windowed_twin_of` in src/register.rs.
      #
      # **Not on macOS, where it would be misleading.** There the file type is declared by the bundle's
      # own Info.plist and LaunchServices reads it when the `.app` is placed, so what makes double-click
      # work is the bundle rather than a command -- and `--register` only nudges `lsregister`, which
      # matters while developing and not to somebody who has just unzipped this. The bundle's line is
      # printed below instead.
      if [ "$PLATFORM" != "macos" ]; then
        printf '    cd %s && ./%s%s --register   # double-click .kmbuild from then on\n' \
          "$d" "$exe" "$EXT"
      fi ;;
    km-remote)
      # The console executable where there is one, for the reason given above the km-package-builder
      # arm: a GUI-subsystem exe run from a prompt appears to say nothing at all.
      #
      # And **no `--open` where there is a window**, which is not a cosmetic difference: a build with
      # one opens it unasked, so printing the flag would suggest it is needed and invite somebody to
      # conclude the window was what `--open` did. Where there is no window the flag is exactly how
      # you get the page in front of you, so it stays.
      # The two decisions are tied together, and getting them independently is how you produce a line
      # that is wrong: the exe named here is the console twin on Windows, and **the console twin
      # never opens a window whatever the build** -- that is what it is for. So it is precisely the
      # case that still needs `--open`. The only line without the flag is the one naming a windowed
      # executable, which is a desktop build anywhere except Windows.
      exe="km-remote"
      open=" --open"
      if desktop_capable km-remote && [ "$DESKTOP" -eq 1 ]; then
        if [ "$PLATFORM" = "windows" ]; then
          exe="km-remote-console"
        else
          open=""
        fi
      fi
      printf '    cd %s && ./%s%s%s\n' "$d" "$exe" "$EXT" "$open" ;;
    km-admin)
      # The same two decisions as the remote's arm above, for the same two reasons: the console twin
      # is what a person at a prompt should type on Windows, and it is precisely the executable that
      # never opens a window — so it is the one that still needs `--open`.
      exe="km-admin"
      open=" --open"
      if desktop_capable km-admin && [ "$DESKTOP" -eq 1 ]; then
        if [ "$PLATFORM" = "windows" ]; then
          exe="km-admin-console"
        else
          open=""
        fi
      fi
      printf '    cd %s && ./%s%s%s\n' "$d" "$exe" "$EXT" "$open" ;;
  esac
done

# The macOS bundles, listed separately because they are a second artifact for a tool already named
# above rather than a seventh tool -- and because what you do with one is open it, not type it.
if [ "${#staged_apps[@]}" -gt 0 ]; then
  echo
  echo "and ${#staged_apps[@]} application bundle(s):"
  for a in "${staged_apps[@]}"; do
    printf '  %s\n' "$a"
    case "$a" in
      *"KM Package Builder.app")
        printf '    open "%s"   # ...and .kmbuild files open in it from now on\n' "$a" ;;
      *)
        printf '    open "%s"\n' "$a" ;;
    esac
  done
fi
