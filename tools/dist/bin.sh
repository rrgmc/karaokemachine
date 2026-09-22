#!/usr/bin/env bash
#
# Stages one folder holding every executable this platform can build.
#
#   tools/dist/bin.sh                  # stage everything, then gather it into dist/bin{,-console}/
#   tools/dist/bin.sh --no-build       # gather what is already staged; build nothing
#   tools/dist/bin.sh --zip            # also produce a versioned .zip of each folder
#   tools/dist/bin.sh --no-video       # the smaller build, throughout
#   tools/dist/bin.sh --no-desktop     # km-package-builder and km-remote without their windows
#   tools/dist/bin.sh -v               # watch the builds; quiet is the default
#
#   dist/bin/<platform>/           the GUI form of anything that has one, the plain form of the rest
#   dist/bin-console/<platform>/   the console form of anything that has one, the same plain rest
#
# **This is a second carrier, not a new layout.** `dist/<app>/<platform>/<app>-<version>-<triple>/`
# is untouched and stays the answer to what a release *is*: a release is a thing you hand to
# somebody, and somebody who wants the curation tool has no use for the other six. What this answers
# is the other question people ask -- *give me one folder with all of it in it* -- which today means
# unzipping seven folders and merging them by hand, and getting that merge wrong in the two places it
# can go wrong. The ffmpeg libraries appear in five of those folders, and on Windows three products
# ship a GUI executable and a console twin under different names. It is the same move
# `A second Linux carrier` made for the tarball; see `A folder with everything in it` in docs/decisions/distribution.md.
#
# **Executables only.** No `.deb`, no installer, no `.tar.gz` -- those are carriers of their own and a
# folder of programs is not the place for one. On Linux this therefore gathers the *tarball*, which is
# the portable folder, and never the package.
#
# **It gathers rather than builds, and that is the property to preserve.** Every fact about which
# crate takes `video`, which takes `desktop`, which four DLLs get staged, how a macOS load command is
# rewritten and what each README says already lives in tools/dist/cmd.sh and its three siblings.
# None of it is restated here: this script runs those scripts and copies what they produced, so the
# two cannot drift. What it knows by itself is three rules about *shape*, below.
#
# What lands in each folder, given a staged product folder:
#
#   1. A `*.app` beside the folder is that product's GUI form                  -> bin/ only
#      ...and the bare executable in the folder is therefore the console form  -> bin-console/ only
#   2. A file `<x>-console<EXT>` is the console form                           -> bin-console/ only
#      ...and its sibling `<x><EXT>` is therefore the GUI form                 -> bin/ only
#   3. Every other executable is single-form                                   -> both
#
# **Rules 1 and 2 are the same rule, said from each end.** A product with two forms puts the one you
# double-click in `bin/` and the one with somewhere to print in `bin-console/`; what differs is only
# how the platform spells the pair. Windows spells it as two files under two names, `km-remote.exe`
# beside `km-remote-console.exe`. macOS spells it as a bundle beside the executable it wraps -- so
# `KM Remote.app` is the GUI form and the `km-remote` in the staged folder is the console
# one, and copying that bare binary into `bin/` as well put a second, worse way to start the same
# program next to the icon: a browser tab with no Dock entry behind it, in the folder whose own README
# says it holds the one you double-click. `karaokemachine` on macOS has always been gathered this way
# -- see the extraction below, which takes its console copy out of the bundle -- and this is the other
# three products agreeing with it.
#
# Support files -- `assets/`, `lib/`, `LICENSES/`, `install.sh`, the ffmpeg libraries and
# their terms -- go to both. `README.txt` is renamed `README-<app>.txt`, because seven of them collide
# on one name and each is a document somebody actually reads; a short `README.txt` written here says
# what the folder is and points at them.
#
# **Rules and not a table, so that an eighth product needs no edit here.** The alternative was a list
# naming each product's GUI and console executable, which is the same list tools/dist/cmd.sh already
# keeps in a form it can act on, spelled a second time in a form it cannot.
#
# **Nothing is dropped silently.** Anything a staged folder holds that matches none of the rules is
# reported by name. A carrier that quietly loses a file is the one failure this must not have, and the
# case that proves it is real: `km-package-builder.exe.WebView2/` appears in that tool's staged folder
# the first time anybody runs the exe from it. It is the webview's own scratch directory, not
# something a staging script produced, so it is skipped -- by name and out loud, not by accident.
#
# **The folders carry no version, and the zips do.** `dist/bin/windows` is a place you keep the
# current build, in the way `Karaoke Machine.app` is; the number belongs on the thing you hand over,
# which is the archive. One consequence worth knowing: `tools/dist/clean.sh --old` can never remove
# one of these folders, exactly as it can never remove a `.app`. `--all` is what does.

set -euo pipefail

cd "$(dirname "$0")/../.."

. tools/dist/common.sh
DIST_SCRIPT=dist-bin

# Every product with an executable in it. The order is the order they are reported in: the machine
# first, then the tools in the order tools/dist/cmd.sh names them.
ALL_APPS=(karaokemachine km-pack km-lyrics km-package-builder km-package-simple km-remote km-admin km-wallpaper-pack)

# Is this the name of a program somebody runs, as opposed to a support file that happens to be marked
# executable?
#
# **The rule is the name, not the mode bit**, and that is not a shortcut. On Windows every file in a
# staged folder reports as executable to a Git Bash `-x`, so the mode says nothing at all there. Off
# Windows it says too much: `install.sh` in the Linux tarball is 0755, and a rule that trusted the
# mode would have copied it in as a product, counted it, and then *run it* during the checks at the
# bottom -- which would put a menu entry in the menus of whoever staged the build.
#
# So: `.exe` is the whole answer on Windows, and off Windows a dotted name is a script or a document
# and an undotted one is a program. Every executable this workspace produces is undotted
# (`karaokemachine`, `km-pack`, `km-wallpaper-pack`), and everything else a staging script puts beside
# them has an extension.
is_program() { # <basename>
  case "$1" in
    *.exe) [ -n "$EXT" ] ;;
    *.*)   return 1 ;;
    *)     [ -z "$EXT" ] ;;
  esac
}

# Does this folder carry the libraries that read video?
#
# **Asked of the folder, never of the flag**, and that distinction was not academic: `--no-video`
# with `--no-build` cheerfully gathered the video build staged an hour earlier and then wrote a
# README over it saying video songs would not play. The suffix list below narrows *which* folder is
# chosen, but it cannot settle this on its own, because a product with no `video` feature to decline
# never carries a marker either -- so an empty suffix has to stay acceptable, and an empty suffix
# matches a video build too.
#
# The libraries being present is the thing that is actually true or false, and it is one `[ -f ]` per
# name. It decides both the refusal below and what the README claims, so the README cannot lie about
# a folder whatever flags produced it.
has_ffmpeg() { # <dir>
  local dir="$1" dll
  for dll in "${DIST_FFMPEG_DLLS[@]}"; do
    if [ -f "$dir/$dll" ]; then return 0; fi
  done
  # The other two platforms carry them in `lib/`, under an soname or a versioned dylib rather than
  # the Windows names above.
  for dll in "$dir"/lib/libav*; do
    if [ -e "$dll" ]; then return 0; fi
  done
  return 1
}

# Did this product stage a `.app`?
#
# **Asked of the directory, never of a list of products**, for the reason `has_ffmpeg` gives about
# flags: a name here would be tools/dist/cmd.sh's `bundle_capable` spelled a second time, in a form
# this script cannot check, and it would go stale the moment a fifth bundle appears. A bundle being
# there is the thing that is actually true or false, and it is one glob.
#
# It also gets `--no-desktop` right for free. That flag stages no bundles, so this is false, so the
# bare executable is single-form again and goes to both folders -- which is correct, because with no
# bundle there is no other form of it anywhere.
has_bundle() { # <app>
  local bundle
  for bundle in "$(dist_dir "$1" "$PLATFORM")"/*.app; do
    if [ -d "$bundle" ]; then return 0; fi
  done
  return 1
}

# Directories a staged folder may hold that are content rather than clutter. Everything here is
# copied into both folders and merged across products -- `lib/` in particular arrives from every
# video-capable tool on macOS, holding the same closure each time.
#
# `Contents` is deliberately absent: a `.app` is copied whole, as one thing, rather than walked.
SUPPORT_DIRS=(assets lib share LICENSES)

# -- what was asked for ------------------------------------------------------------------------------

ZIP=0
VIDEO=1
DESKTOP=1
BUILD=1
for arg in "$@"; do
  case "$arg" in
    --zip) ZIP=1 ;;
    --no-video) VIDEO=0 ;;
    --no-desktop) DESKTOP=0 ;;
    # Gather what is already there. The fast path while working on this script, and the honest answer
    # for somebody who has just run `task dist` and does not want six release builds again.
    --no-build) BUILD=0 ;;
    -v|--verbose) DIST_VERBOSE=1 ;;
    -h|--help)
      echo "usage: tools/dist/bin.sh [--no-build] [--no-video] [--no-desktop] [--zip] [-v]"
      exit 0
      ;;
    *) echo "dist-bin: unknown option $arg" >&2; exit 2 ;;
  esac
done

TARGET="$(dist_host_triple)"
PLATFORM="$(dist_platform "$TARGET")"
EXT="$(dist_exe_ext "$TARGET")"

# From the manifest, not from an artifact -- this script is looking for the artifact and so has
# nothing to run. The same two-strip idiom and the same reason as tools/dist/clean.sh: `cargo pkgid`
# prints `...#karaokemachine@1.2.0` when the package name differs from its directory and `...#1.2.0`
# when it does not.
pkg_version() { # <cargo pkgid arguments>  -> prints the version
  local p
  p="$(cargo pkgid "$@")"
  p="${p##*#}"
  printf '%s' "${p##*@}"
}

# **One number for every product here**, which is why this is a variable and not a function taking an
# app. `km-admin` and `km-wallpaper-pack` are in the excluded workspace under `tools/cmd/assets`; a
# version of their own would make this a `case` with an arm per program, and the arm nobody adds is
# the bug -- `task dist:bin` staging km-admin under a number of its own and then reporting nothing was
# staged for it, having looked for the machine's. Both follow it; see that workspace's
# `[workspace.package]`, and
# `tools/dev/check-version-pin.sh` for what keeps the two roots agreeing.
VERSION="$(pkg_version -p karaokemachine)"

# -- staging, unless it was declined -----------------------------------------------------------------

# Exactly what `task dist` runs, with one deliberate omission: `tools/platform/linux/deb.sh`. A `.deb` is a
# carrier, not an executable, and nothing in it would be gathered.
#
# The flags are forwarded rather than reinterpreted, so a `--no-video` run here produces and gathers
# the same folders a `--no-video` run of those scripts produces on its own.
#
# Both of these are `if` rather than `&&`, and that is the same trap `dist_detail` documents: a
# `cond && thing` whose condition is false is a statement that returned 1, and every script here runs
# under `set -e`, so the quiet non-video case would take the script down before it built anything.
FLAGS=()
if [ "$VIDEO" -ne 1 ]; then FLAGS+=(--no-video); fi
if dist_verbose; then FLAGS+=(--verbose); fi

if [ "$BUILD" -eq 1 ]; then
  dist_step "staging (this is where the minutes go)"
  case "$PLATFORM" in
    windows) tools/platform/windows/dist.sh "${FLAGS[@]+"${FLAGS[@]}"}" ;;
    macos)   tools/platform/macos/app-bundle.sh "${FLAGS[@]+"${FLAGS[@]}"}" ;;
    linux)   tools/platform/linux/tarball.sh "${FLAGS[@]+"${FLAGS[@]}"}" ;;
    *)       echo "dist-bin: no staging script for $PLATFORM" >&2; exit 1 ;;
  esac
  # `--no-desktop` goes only to the tools: it is km-package-builder's and km-remote's flag, and
  # the machine's staging scripts would exit 2 on an option they do not know.
  TOOL_FLAGS=("${FLAGS[@]+"${FLAGS[@]}"}")
  if [ "$DESKTOP" -ne 1 ]; then TOOL_FLAGS+=(--no-desktop); fi
  tools/dist/cmd.sh "${TOOL_FLAGS[@]+"${TOOL_FLAGS[@]}"}"
  echo
fi

# -- the two folders ---------------------------------------------------------------------------------

BIN="dist/bin/$PLATFORM"
CONSOLE="dist/bin-console/$PLATFORM"

# The folder-name suffixes this run will accept, most specific first. `dist_staged_dir` takes the
# first of them that exists, and refuses if none does.
#
# **This is what stops `--no-video` gathering a video build.** The marker goes on the *declined*
# build, so what a run should accept is decided entirely by which flags it was given -- and the empty
# suffix has to be in the list every time, because a product with no such feature to decline never
# carries a marker. km-lyrics has no `video` feature and km-pack no `desktop` one; neither is a
# special case here, which is the point.
#
# Without it, `tools/dist/bin.sh --no-video --no-build` found the video folder staged an hour
# earlier, gathered it, and wrote a README over it saying video songs would not play.
WANTED=()
if [ "$VIDEO" -ne 1 ] && [ "$DESKTOP" -ne 1 ]; then WANTED+=(-no-video-no-desktop); fi
if [ "$VIDEO" -ne 1 ];   then WANTED+=(-no-video); fi
if [ "$DESKTOP" -ne 1 ]; then WANTED+=(-no-desktop); fi
WANTED+=("")

dist_step gathering
dist_clear "$BIN"
dist_clear "$CONSOLE"

# Counters and notes accumulated across every product, reported at the end.
GUI_COUNT=0
CONSOLE_COUNT=0
BOTH_COUNT=0
SKIPPED=()
GATHERED=()

# Copy one path into a destination, refusing to disagree with a copy that is already there.
#
# **A collision on identical bytes is a no-op and a collision on different bytes is an error**, which
# is the whole point of doing this with a comparison rather than a `cp -n` or a `cp -f`. The four
# ffmpeg DLLs arrive from five staged folders; they are the same file each time, and that is a claim
# worth testing once per file rather than assuming. Two products shipping *different* builds of one
# library is precisely the merge fault this carrier exists to stop somebody making by hand.
place() { # <source path> <destination dir> [destination name]
  local src="$1" dest="$2" name="${3:-$(basename "$1")}"
  local at="$dest/$name"
  if [ -e "$at" ]; then
    if [ -d "$src" ]; then
      # A directory arriving twice is merged file by file, so the check applies where it means
      # something -- `assets/` from the machine and `lib/` from four tools are the real cases.
      local rel
      while IFS= read -r rel; do
        [ -n "$rel" ] || continue
        mkdir -p "$at/$(dirname "$rel")"
        place "$src/$rel" "$at/$(dirname "$rel")"
      done < <(cd "$src" && find . -type f | sed 's|^\./||' | sort)
      return 0
    fi
    if cmp -s "$src" "$at"; then return 0; fi
    # **A signed Mach-O is allowed to differ byte for byte and still be the same file**, and this is
    # the one place that premise breaks. `--timestamp` asks Apple's timestamp authority at signing
    # time, so two signings of identical input embed different CMS blobs -- and the four ffmpeg
    # dylibs are signed once per staged folder, five folders over. The bytes differ; the code does
    # not. `CDHash` covers the code directory, which hashes the code pages and the identifier and
    # nothing about when it was signed, so comparing that asks the question this check means to ask.
    # Found by turning signing on and watching the gather refuse its own output.
    if dist_signing && [ "$(dist_cdhash "$src")" = "$(dist_cdhash "$at")" ] \
       && [ -n "$(dist_cdhash "$src")" ]; then
      return 0
    fi
    echo "dist-bin: two staged folders disagree about $name." >&2
    echo "      one of them is $src; they are not the same file." >&2
    return 1
  fi
  cp -R "$src" "$at"
  return 0
}

# The three rules, applied to one staged product folder.
gather_app() { # <app> <staged dir>
  local app="$1" dir="$2" entry base
  local gui=() twin=() both=()

  for entry in "$dir"/*; do
    [ -e "$entry" ] || continue
    base="$(basename "$entry")"

    if [ -d "$entry" ]; then
      case " ${SUPPORT_DIRS[*]} " in
        *" $base "*) both+=("$entry"); continue ;;
      esac
      # Rule 3's directory case does not exist -- nothing a staging script produces is a directory
      # that is also a program, except a `.app`, which is handled where the bundles are. So anything
      # else here is somebody's or something's leftover, and is named rather than swept along.
      SKIPPED+=("$app: $base/ (a directory no staging script produces)")
      continue
    fi

    case "$base" in
      README.txt)
        # Renamed rather than merged: seven of them collide on one name, each is a document somebody
        # reads, and the ffmpeg license note at the end of each covers that folder's own libraries.
        place "$entry" "$BIN" "README-$app.txt"
        place "$entry" "$CONSOLE" "README-$app.txt"
        continue
        ;;
      *-console"$EXT")
        twin+=("$entry")
        continue
        ;;
    esac

    if is_program "$base"; then
      # Whether it is the GUI half of a pair is decided below, once every entry is known: `<x>` is
      # the GUI form only if `<x>-console` also turned up in this folder.
      gui+=("$entry")
    else
      # Everything else a staging script leaves beside a program: the ffmpeg DLLs, their terms,
      # install.sh, a license text. All of it belongs in both folders.
      both+=("$entry")
    fi
  done

  # The second half of rules 2 and 1, in that order, and this is why the loop above sorts rather
  # than places: the answer for `<x>` is not in `<x>` -- it is in whether `<x>-console` turned up
  # beside it, and in whether this product staged a bundle.
  #
  #   a console twin beside it   -> `<x>` is the GUI form of a pair       -> bin/ alone
  #   a `.app` beside the folder -> `<x>` is the console form of a pair   -> bin-console/ alone
  #   neither                    -> single-form                           -> both
  #
  # The two cannot both hold: a `.app` is built only on macOS and a `-console` twin only on Windows.
  # Asked in this order anyway rather than as an `elif` on an assumption, because the order is free
  # and the assumption is the kind that stops being true quietly.
  local exe twin_path
  local bundled=0
  if has_bundle "$app"; then bundled=1; fi
  for exe in ${gui[@]+"${gui[@]}"}; do
    base="$(basename "$exe")"
    twin_path="$dir/${base%"$EXT"}-console$EXT"
    if [ -f "$twin_path" ]; then
      place "$exe" "$BIN"
      GUI_COUNT=$((GUI_COUNT + 1))
    elif [ "$bundled" -eq 1 ]; then
      place "$exe" "$CONSOLE"
      CONSOLE_COUNT=$((CONSOLE_COUNT + 1))
    else
      place "$exe" "$BIN"
      place "$exe" "$CONSOLE"
      BOTH_COUNT=$((BOTH_COUNT + 1))
    fi
  done
  for exe in ${twin[@]+"${twin[@]}"}; do
    place "$exe" "$CONSOLE"
    CONSOLE_COUNT=$((CONSOLE_COUNT + 1))
  done
  for entry in ${both[@]+"${both[@]}"}; do
    place "$entry" "$BIN"
    place "$entry" "$CONSOLE"
  done
}

# Rule 1: a bundle sits *beside* the staged folder, at the platform level, and is the GUI form of
# whatever it wraps. tools/platform/macos/app-bundle.sh puts `Karaoke Machine.app` there and
# tools/dist/cmd.sh puts `KM Package Builder.app` there.
gather_bundles() { # <app>
  local app="$1" parent bundle
  parent="$(dist_dir "$app" "$PLATFORM")"
  for bundle in "$parent"/*.app; do
    [ -d "$bundle" ] || continue
    # `ditto` rather than `cp -R` or `place`, for the reason `dist_zip_macos_bundle` exists: a bundle
    # is made of symlinks, resource forks and a signature sealed over all of it, and the copy has to
    # preserve every one or `codesign --verify` stops agreeing that it will load elsewhere. It was
    # already sealed by whoever staged it, so nothing here re-signs it.
    ditto "$bundle" "$BIN/$(basename "$bundle")"
    GUI_COUNT=$((GUI_COUNT + 1))
  done
}

for app in "${ALL_APPS[@]}"; do
  # The machine on macOS is staged only as a bundle -- there is no folder at all -- so the lookup is
  # allowed to come up empty there and nowhere else.
  if STAGED="$(dist_staged_dir "$app" "$PLATFORM" "$VERSION" "${WANTED[@]}" 2>/dev/null)"; then
    # The flag has to mean something, and with `--no-build` the only evidence of what was staged is
    # what is in the folder. A `--no-video` run that gathered a video build would produce exactly the
    # folder the flag exists to avoid.
    if [ "$VIDEO" -ne 1 ] && has_ffmpeg "$STAGED"; then
      echo "dist-bin: --no-video was given, but $STAGED is a video build." >&2
      echo "      It carries ffmpeg's libraries. Stage that product with --no-video first, or drop" >&2
      echo "      the flag; tools/dist/bin.sh --no-video without --no-build does both." >&2
      exit 1
    fi
    gather_app "$app" "$STAGED"
    GATHERED+=("$app")
  elif [ "$PLATFORM" != "macos" ] || [ "$app" != "karaokemachine" ]; then
    # **The version is in the message on purpose.** `dist_staged_dir` says which folder name it wanted
    # and that line is discarded by the redirect above, which has to stay -- the macOS machine is
    # expected to have no folder and that is not an error. Without the number, a product that really
    # had just been staged, under a name this script did not look for, read exactly like one that had
    # never been built.
    echo "dist-bin: nothing staged for $app $VERSION." >&2
    if [ "$BUILD" -eq 0 ]; then
      echo "      --no-build was given, so nothing was staged for it here either. Run" >&2
      echo "      tools/dist/bin.sh without it, or stage that product on its own first." >&2
    fi
    exit 1
  fi
  # `if` rather than `&&`, for the `set -e` reason given where the flags are assembled.
  if [ "$PLATFORM" = "macos" ]; then gather_bundles "$app"; fi
done

# -- the macOS machine, which has no folder to gather from --------------------------------------------

# `tools/platform/macos/app-bundle.sh` produces a bundle and nothing else, so on that platform there is no bare
# `karaokemachine` anywhere -- and `bin-console/` is exactly the folder that wants one. Taken out of
# the bundle rather than built a second time, which keeps this script a gatherer.
#
# Three things travel with it and the third is the one that is easy to miss. The dylibs are already
# `@rpath`-identified by `dist_stage_ffmpeg_macos`, so they only have to move; `assets/` has to sit
# *beside* the executable, because `Paths::discover_asset_dir` looks next to the binary and the
# bundle keeps them under Contents/Resources; and the rpath itself has to be repointed, from the
# bundle's `@executable_path/../Frameworks` to this folder's `@executable_path/lib`. The old one is
# deleted rather than left as a dead entry -- dyld would ignore it, but a load command naming a
# directory that is not there is a thing somebody has to work out later.
#
# `install_name_tool` invalidates whatever signature the file carried and dyld on Apple silicon
# refuses a Mach-O whose signature does not match, so the re-sign is mandatory rather than tidy.
# Which identity it uses is `KM_SIGN_IDENTITY`'s answer -- see `dist_codesign`. This is the same
# sequence `dist_stage_ffmpeg_macos` ends with, and `dist_verify_macho_portable` below covers the
# result exactly as it covers everything else.
if [ "$PLATFORM" = "macos" ]; then
  APP="$(dist_dir karaokemachine macos)/Karaoke Machine.app"
  if [ ! -d "$APP" ]; then
    echo "dist-bin: $APP is missing -- tools/platform/macos/app-bundle.sh has not run." >&2
    exit 1
  fi
  cp "$APP/Contents/MacOS/karaokemachine" "$CONSOLE/karaokemachine"
  chmod 755 "$CONSOLE/karaokemachine"
  if [ -d "$APP/Contents/Resources/assets" ]; then
    place "$APP/Contents/Resources/assets" "$CONSOLE"
  fi
  # Only where there are libraries to point at. A `--no-video` bundle has no Frameworks and no rpath
  # naming one, and adding an `@executable_path/lib` that is not there would be a load command
  # somebody has to work out the absence of later.
  if [ -d "$APP/Contents/Frameworks" ]; then
    mkdir -p "$CONSOLE/lib"
    for dylib in "$APP/Contents/Frameworks"/*.dylib; do
      [ -f "$dylib" ] || continue
      place "$dylib" "$CONSOLE/lib"
    done
    install_name_tool -delete_rpath "@executable_path/../Frameworks" "$CONSOLE/karaokemachine" 2>/dev/null || true
    if ! otool -l "$CONSOLE/karaokemachine" | grep -q "path @executable_path/lib "; then
      install_name_tool -add_rpath "@executable_path/lib" "$CONSOLE/karaokemachine"
    fi
    # **The dylibs beside it are re-signed too, and that is not redundant.** `place` above dedups on
    # `cmp -s`, so a folder staged under one identity and re-gathered under another keeps the old
    # libraries and says nothing -- an executable and its libraries disagreeing about who signed
    # them, which is the one shape notarization refuses without naming a file. Signing them here
    # costs milliseconds and removes the whole class.
    for dylib in "$CONSOLE/lib"/*.dylib; do
      [ -f "$dylib" ] || continue
      dist_codesign "$dylib" 2>/dev/null
    done
    dist_codesign "$CONSOLE/karaokemachine"
  fi
  BOTH_COUNT=$((BOTH_COUNT + 1))
fi

# -- the folder's own README --------------------------------------------------------------------------

# Short, and deliberately not a summary of the seven beside it. What somebody opening this folder
# needs is which file to start and where the real documentation is; each product's own README says
# everything else, including the license terms for whatever libraries it brought with it.
# **This document is about a folder, and only about a folder.** It says the folder holds every
# executable this platform can build, it lists them by scanning the staged directory, and it says to
# get rid of it by deleting it because nothing was installed elsewhere and nothing was registered.
# Every word of that is true here and none of it survives being installed, which is why both setup
# programs stopped shipping it: an installed build gets `dist_installed_readme` in
# tools/dist/common.sh instead. So do not "fix" this to hedge across both carriers -- the hedging
# version was considered and rejected, because it would have to straddle folder-versus-installed and
# Windows-versus-macOS at once, which is four situations in one document. See the `What an installed
# build contains` decision in docs/decisions/.
write_readme() { # <dir> <kind: gui|console>
  local dir="$1" kind="$2" exe base
  {
    if [ "$kind" = gui ]; then
      cat <<'HEAD'
KaraokeMachine -- every program, in one folder
===============================================

This folder holds every executable this platform can build, with everything they need beside them.
It is the same build as the per-product folders a release is normally handed out as; what is
different is only that it is one folder rather than seven.
HEAD
    else
      cat <<'HEAD'
KaraokeMachine -- every program, in one folder (for diagnosing)
===============================================================

This folder holds every executable this platform can build, with everything they need beside them.
Where a program comes in two forms, this folder has the one with somewhere to print: --help,
--version and the reason it would not start all reach you, which is what makes this the copy to
reach for when something is wrong. The folder beside this one, bin/, holds the ordinary form of
each -- the one you double-click, which opens a window and no console with it, and the one to
use for anything other than diagnosing.
HEAD
    fi

    echo
    echo "What is here"
    echo "------------"
    echo
    for exe in "$dir"/*; do
      [ -e "$exe" ] || continue
      base="$(basename "$exe")"
      # The programs, and the macOS bundles, which are programs in the way that matters here.
      case "$base" in *.app) printf '    %s\n' "$base"; continue ;; esac
      if is_program "$base"; then printf '    %s\n' "$base"; fi
    done

    cat <<'BODY'

Each of them has a README of its own here, named for it -- README-karaokemachine.txt,
README-km-pack.txt and so on. Those are the documents to read: they say what each program is for,
how to run it, and what the libraries beside it are licensed under.

The one rule about this folder
------------------------------

Keep it together. assets/ holds the instrument bank and the wallpapers and has to stay beside
karaokemachine, which looks for it there; lib/ (where there is one) holds the libraries that read
video and has to stay beside the programs that read it. Copy the whole folder, not files out of it.

Getting rid of it
-----------------

Delete the folder. Nothing was installed anywhere else and nothing was registered, so there is
nothing left behind to find.

Your songs, settings and catalog are not in here -- they are under your home directory, so
deleting this folder loses nothing but the programs. Start the machine with --show-paths before you
delete it if you want to know where they are.
BODY

    # From the folder rather than from `$VIDEO`, so this sentence is true whatever flags produced the
    # folder -- see `has_ffmpeg`, and the run that made this necessary.
    if has_ffmpeg "$dir"; then
      cat <<'BODY'

This build plays video songs.
BODY
    else
      cat <<'BODY'

This build was staged with --no-video: video songs are catalogd and queued, and will not play.
BODY
    fi

    # **This folder gathers signed bundles and said nothing about it.** Which kind of build they are
    # decides whether the recipient has to do anything before opening one, so it belongs in the one
    # document they get. macOS only: it is the only platform here where a signature changes what
    # somebody has to do.
    #
    # It said "the three .app bundles" until a fourth was added and nobody came back here. The count
    # told the reader nothing they could not see in the folder, so it is gone rather than derived --
    # a number kept correct by hand in a third place is the same fault waiting for a fifth.
    if [ "$PLATFORM" = macos ]; then
      if dist_signing; then
        cat <<'BODY'

The .app bundles here are signed with a Developer ID and run under the hardened runtime, so they
open normally. They are not notarized -- that is done to the installer package rather than to a
folder -- so a copy that arrives by download may still need one right-click -> Open.
BODY
      else
        cat <<'BODY'

The .app bundles here are signed only ad-hoc, so on any Mac other than the one that built them
macOS refuses to open them until the quarantine flag is cleared:

    xattr -dr com.apple.quarantine "Karaoke Machine.app"

The first launch is slow while macOS assesses the bundle, and immediate afterwards.
BODY
      fi
    fi
  } > "$dir/README.txt"
}

write_readme "$BIN" gui
write_readme "$CONSOLE" console

# -- report -------------------------------------------------------------------------------------------

echo
# Once each, into a variable, the way every sibling script does it: `dist_bytes` walks the whole tree
# and these folders are the largest thing staged anywhere here.
bin_bytes="$(dist_bytes "$BIN")"
console_bytes="$(dist_bytes "$CONSOLE")"
printf 'staged %s\n' "$BIN"
printf '  %s bytes total (~%s MiB)\n' "$bin_bytes" "$((bin_bytes / 1024 / 1024))"
printf 'staged %s\n' "$CONSOLE"
printf '  %s bytes total (~%s MiB)\n' "$console_bytes" "$((console_bytes / 1024 / 1024))"
echo
printf '  %s product(s) gathered: %s\n' "${#GATHERED[@]}" "${GATHERED[*]}"
printf '  %s with a window, %s with a console, %s with only one form\n' \
       "$GUI_COUNT" "$CONSOLE_COUNT" "$BOTH_COUNT"

# On Linux nothing has two forms -- the machine has no console twin off Windows, and
# km-package-builder and km-remote never carry the `desktop` feature there, because wry links
# libwebkit2gtk at load time. Both folders are still produced, because "bin-console/<platform> is
# there on every platform" is worth more than the copy it saves; saying so is what stops it reading
# as a bug.
if [ "$GUI_COUNT" -eq 0 ] && [ "$CONSOLE_COUNT" -eq 0 ]; then
  echo
  echo "  note: on $PLATFORM nothing has two forms, so these two folders are the same folder."
fi

if [ "${#SKIPPED[@]}" -gt 0 ]; then
  echo
  echo "  not gathered, because no rule here recognizes them:"
  for note in "${SKIPPED[@]}"; do printf '    %s\n' "$note"; done
fi

# -- what this folder claims, tested rather than asserted ----------------------------------------------

# The claim is that either folder runs when copied to another machine, and a merged folder is exactly
# where a missing library first shows up -- so it is checked the same way every sibling script checks
# its own: by starting what was staged with the environment stripped of anything that could be hiding
# the failure.
#
# `--version` for everything except the machine, which is asked for `--show-paths` for the same reason
# tools/platform/windows/dist.sh asks it: it prints and exits, which proves the process got as far as running
# its own code.
ok=1
probe() { # <exe basename, without extension>  -> prints the argument to check it with
  case "$1" in karaokemachine|karaokemachine-console) printf -- '--show-paths' ;; *) printf -- '--version' ;; esac
}

echo
case "$PLATFORM" in
  windows)
    for dir in "$BIN" "$CONSOLE"; do
      for exe in "$dir"/*; do
        [ -f "$exe" ] || continue
        is_program "$(basename "$exe")" || continue
        base="$(basename "$exe" .exe)"
        ( cd "$dir" && PATH="/c/Windows/System32:/c/Windows" "./$base.exe" "$(probe "$base")" >/dev/null 2>&1 ) \
          || { echo "warning: $exe would not start with a bare PATH." >&2; ok=0; }
      done
    done
    if [ "$ok" -eq 1 ]; then
      echo "verified: every executable in both folders starts with nothing on PATH but Windows itself,"
      echo "          so each folder is self-contained."
    fi
    ;;
  macos)
    for dir in "$BIN" "$CONSOLE"; do
      dist_verify_macho_portable "$dir" || ok=0
      for exe in "$dir"/*; do
        [ -f "$exe" ] || continue
        is_program "$(basename "$exe")" || continue
        base="$(basename "$exe")"
        ( cd "$dir" && "./$base" "$(probe "$base")" >/dev/null 2>&1 ) \
          || { echo "warning: $exe would not start." >&2; ok=0; }
      done
    done
    if [ "$ok" -eq 1 ]; then
      echo "verified: nothing loads by absolute path and every executable starts. Both folders are"
      echo "          self-contained. The bundles carry their own libraries and were sealed already."
    fi
    ;;
  linux)
    for dir in "$BIN" "$CONSOLE"; do
      for exe in "$dir"/*; do
        [ -f "$exe" ] || continue
        is_program "$(basename "$exe")" || continue
        base="$(basename "$exe")"
        ( cd "$dir" && env -i HOME=/tmp PATH=/usr/bin:/bin "./$base" "$(probe "$base")" >/dev/null 2>&1 ) \
          || { echo "warning: $exe would not start with nothing inherited from this shell." >&2; ok=0; }
      done
    done
    if [ "$ok" -eq 1 ]; then
      echo "verified: every executable starts with nothing inherited from this shell."
    fi
    ;;
esac

if [ "$ok" -ne 1 ]; then
  echo "         Something a staged folder needed is missing here; it will fail elsewhere too." >&2
  exit 1
fi

# A rule that matched nothing produces a smaller folder and no error, which is the failure this
# carrier is least able to notice. Reconciling the count against the products gathered is what makes
# it an error instead.
if [ "$((GUI_COUNT + CONSOLE_COUNT + BOTH_COUNT))" -lt "${#GATHERED[@]}" ]; then
  echo "dist-bin: fewer executables than products gathered -- a rule matched nothing." >&2
  exit 1
fi

# -- the zips, if asked for ----------------------------------------------------------------------------

# The folders carry no version and the archives do, which is where a version is actually useful: a
# folder is a place you keep the current build, an archive is a thing you hand over. `dist_zip_as`
# is what makes the name inside the archive match the name on it -- without it this would unpack to a
# directory called `windows`.
if [ "$ZIP" -eq 1 ]; then
  echo
  SUFFIX=""
  [ "$VIDEO" -eq 1 ]   || SUFFIX="$SUFFIX-no-video"
  [ "$DESKTOP" -eq 1 ] || SUFFIX="$SUFFIX-no-desktop"
  dist_zip_as "$BIN"     "dist/bin/karaokemachine-bin-$VERSION-$TARGET$SUFFIX"
  dist_zip_as "$CONSOLE" "dist/bin-console/karaokemachine-bin-console-$VERSION-$TARGET$SUFFIX"
fi

echo
case "$PLATFORM" in
  windows) echo "now:  cd $CONSOLE && ./karaokemachine-console.exe --show-paths" ;;
  macos)   echo "now:  open \"$BIN/Karaoke Machine.app\"" ;;
  *)       echo "now:  cd $BIN && ./karaokemachine --show-paths" ;;
esac
