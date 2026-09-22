#!/usr/bin/env bash
#
# Builds the Windows setup program for the remote alone.
#
#   tools/platform/windows/installer-remote.sh              # stage the remote, compile, test it
#   tools/platform/windows/installer-remote.sh --no-build   # compile from what is already staged
#   tools/platform/windows/installer-remote.sh -v           # watch the staging and the compile
#
#   -> dist/setup/windows/km-remote-setup-<version>-windows-x86_64.exe
#
# **It gathers; it does not build.** The payload is the folder tools/dist/cmd.sh already stages for
# the remote, `dist/km-remote/windows/km-remote-<version>-<triple>` -- five files, about 21 MB. The
# all-in-one gathers `dist/bin/windows` instead, which is every product and a six-minute staging run
# for a carrier that installs one of them.
#
# **There is no --no-video**, and here there is nothing for one to mean: the remote links no ffmpeg
# at all. The all-in-one refuses a payload without it because the machine it carries promises to play
# video; this carrier promises nothing of the kind, and the check it makes is the opposite one --
# that the .iss has not grown a line naming a library nothing here links.
#
# Prerequisite: Inno Setup 6. `winget install JRSoftware.InnoSetup`.

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/dist/common.sh
. tools/platform/windows/inno.sh
DIST_SCRIPT=installer-remote

TARGET="x86_64"
ISS="tools/platform/windows/installer-remote.iss"
ALL_IN_ONE_ISS="tools/platform/windows/installer.iss"
BUILD=1

for arg in "$@"; do
  case "$arg" in
    --no-build) BUILD=0 ;;
    -v|--verbose) DIST_VERBOSE=1 ;;
    -h|--help)
      echo "usage: tools/platform/windows/installer-remote.sh [--no-build] [-v]"
      exit 0 ;;
    *) echo "installer-remote: unknown option $arg" >&2; exit 2 ;;
  esac
done

if [ "$(dist_platform)" != "windows" ]; then
  echo "installer-remote: this builds a Windows setup program and has to run on Windows." >&2
  echo "                  The macOS one is tools/platform/macos/installer-remote.sh; Linux has" >&2
  echo "                  none, because the remote travels in the .deb and the tarball." >&2
  exit 1
fi

inno_require_compiler

# -- the payload ----------------------------------------------------------------------------------

if [ "$BUILD" -eq 1 ]; then
  dist_step "staging the remote"
  staging_started=$SECONDS
  args=()
  if dist_verbose; then args=(-v); fi
  dist_run "dist-cmd.sh" tools/dist/cmd.sh "${args[@]+"${args[@]}"}" km-remote
  printf '   staged in %s\n' "$(dist_elapsed "$staging_started")"
fi

# **Found rather than named, and exactly one.** `dist_staged_dir` takes the version, and the version
# comes out of the binary inside the folder, so the folder has to be resolved first. Two matches is
# an error rather than a choice -- the shape `one_match` in tools/dist/release.sh uses, and for the
# same reason: picking whichever sorted first would package a build nobody asked for.
#
# **The plain name only.** tools/dist/cmd.sh marks a *declined* build, so `-no-desktop` is a remote
# with no window -- which is the whole of what this carrier installs. Packaging it would be the same
# silent downgrade the all-in-one refuses when ffmpeg is missing.
PARENT="$(dist_dir km-remote windows)"
PAYLOAD=""
for candidate in "$PARENT/km-remote-"*"-$(dist_host_triple)"; do
  [ -d "$candidate" ] || continue
  if [ -n "$PAYLOAD" ]; then
    echo "installer-remote: two staged folders under $PARENT/:" >&2
    echo "                  $(basename "$PAYLOAD") and $(basename "$candidate")" >&2
    echo "                  Remove the one you do not want, or run tools/dist/clean.sh." >&2
    exit 1
  fi
  PAYLOAD="$candidate"
done

if [ -z "$PAYLOAD" ]; then
  echo "installer-remote: nothing staged under $PARENT/." >&2
  if [ "$BUILD" -eq 0 ]; then
    echo "                  --no-build was given, so nothing was staged for it here either. Run" >&2
    echo "                  this script without it, or tools/dist/cmd.sh km-remote first." >&2
  fi
  exit 1
fi

if [ ! -f "$PAYLOAD/km-remote-console.exe" ]; then
  echo "installer-remote: $(basename "$PAYLOAD") has no console twin, so it was staged without a" >&2
  echo "                  window -- tools/dist/cmd.sh builds both or neither. A setup program" >&2
  echo "                  installing a remote that cannot open one is not what it promises." >&2
  echo "                  Run tools/dist/cmd.sh km-remote without --no-desktop." >&2
  exit 1
fi

dist_detail "payload    $PAYLOAD"

# **The drift check, in its inverted form.** The all-in-one asserts that every library
# DIST_FFMPEG_DLLS names is installed, because the machine it carries reads video. Here the same list
# is asserted *absent*: the remote links none of them, and a line naming one would turn a 5 MB
# download into a 40 MB one without anybody deciding to.
for dll in "${DIST_FFMPEG_DLLS[@]}"; do
  if grep -qF "\\$dll\"" "$ISS"; then
    echo "installer-remote: $ISS installs $dll, which the remote does not link." >&2
    echo "                  Nothing here reads video; the machine's own installer is where" >&2
    echo "                  those libraries belong." >&2
    exit 1
  fi
done

# -- what the .iss says it installs, against what is actually there --------------------------------
#
# `inno_coverage_check` reads the [Files] section back out and expands it against the payload, so
# what is staged and what is installed cannot come to disagree.
#
# **One exclusion, and it is the console twin.** `km-remote-console.exe` is the same program with
# somewhere to print, and it is there so that a *folder* gives somebody something to type. An
# installed program has a Start Menu entry, and a second executable differing from the first only in
# its subsystem is exactly the confusion an installer exists to remove -- the same judgment the
# all-in-one makes about its own three twins.
#
# The payload's `README.txt` is *not* excluded here, unlike the all-in-one's: that one is a folder
# document about a folder holding every product, and this one is the remote's own document, which is
# right wherever it is read. It is installed as `README-km-remote.txt`, beside the shorter README
# written for an installed build.
inno_coverage_check "$ISS" "$PAYLOAD" km-remote-console.exe || exit 1

skipped_profiles="$(inno_skipped_profiles "$PAYLOAD")"

# -- two carriers, two identities ------------------------------------------------------------------
#
# **Asserted at build time, because the failure is a copy-paste and the symptom is somebody else's
# install disappearing.** Inno decides upgrade-versus-second-copy by AppId and finds an uninstaller by
# it, so sharing one would mean installing the remote removed the karaoke machine. AppName is the
# second of the two because the install folder and the Start Menu group both follow it in each script,
# and Inno removes a directory it created when the uninstall leaves it empty.
differs() { # <label> <pattern> -- fails if the two scripts give the same value
  local label="$1" pattern="$2" mine theirs
  mine="$(sed -n "$pattern" "$ISS" | head -n 1)"
  theirs="$(sed -n "$pattern" "$ALL_IN_ONE_ISS" | head -n 1)"
  if [ -z "$mine" ] || [ -z "$theirs" ]; then
    echo "installer-remote: no $label in $ISS or $ALL_IN_ONE_ISS." >&2
    exit 1
  fi
  if [ "$mine" = "$theirs" ]; then
    echo "installer-remote: $label is $mine in both $ISS and $ALL_IN_ONE_ISS." >&2
    echo "                  The two carriers have to differ in both, or installing one" >&2
    echo "                  upgrades or removes the other." >&2
    exit 1
  fi
}
differs AppId   's/^AppId=\(.*\)$/\1/p'
differs AppName 's/^#define AppName \(.*\)$/\1/p'

# -- the version ----------------------------------------------------------------------------------
#
# From the binary, never the manifest: every shipped crate says `version.workspace = true`, so
# reading it means picking the right one of many `version =` lines, and asking the binary cannot
# disagree with the binary. The GUI-subsystem executable answers `--version` down a pipe perfectly
# well -- standard handles are inherited whatever the subsystem.
VERSION="$(dist_version "$PAYLOAD/km-remote.exe")"

OUTDIR="$(dist_dir setup windows)"
OUTBASE="km-remote-setup-$VERSION-windows-$TARGET"
mkdir -p "$OUTDIR"
rm -f "$OUTDIR/$OUTBASE.exe"

# -- the README an installed build gets ------------------------------------------------------------
#
# **Not the payload's own README.txt**, which describes a folder somebody unpacked: it says the
# console twin is beside the program and tells you to keep the folder together, neither of which is
# true once the windowed program is installed on its own with a Start Menu group and an uninstaller.
# `dist_installed_readme windows km-remote` writes the one that goes in instead -- shared with the
# macOS package so the two cannot come to describe removing the same product differently.
#
# **A generated directory of its own**, not the all-in-one's: that script clears
# `dist/setup/windows/generated` on every run, so sharing it would mean whichever installer built
# last owned the other's README.
GENDIR="$OUTDIR/km-remote-generated"
dist_clear "$GENDIR"
dist_installed_readme windows km-remote > "$GENDIR/README.txt"

# -- compile ---------------------------------------------------------------------------------------
#
# MSYS2_ARG_CONV_EXCL is not optional and not cosmetic. Git Bash rewrites any argument that looks
# like a POSIX path, so `/DPayload=...` arrives at a Windows program as `C:/Program Files/Git/DPayload
# =...`. Every value is additionally converted to a Windows path, because ISCC is a Windows program
# and has never heard of /c/prog.
dist_step "compiling the installer"
compile_started=$SECONDS
MSYS2_ARG_CONV_EXCL='*' dist_run "iscc" "$ISCC" \
  "/DPayload=$(host_path "$PWD/$PAYLOAD")" \
  "/DGenerated=$(host_path "$PWD/$GENDIR")" \
  "/DVersion=$VERSION" \
  "/DOutDir=$(host_path "$PWD/$OUTDIR")" \
  "/DOutBase=$OUTBASE" \
  "$(host_path "$PWD/$ISS")"
printf '   compiled in %s\n' "$(dist_elapsed "$compile_started")"

SETUP="$OUTDIR/$OUTBASE.exe"
if [ ! -f "$SETUP" ]; then
  echo "installer-remote: $SETUP was not produced" >&2
  exit 1
fi

# -- report ----------------------------------------------------------------------------------------

payload_bytes="$(dist_bytes "$PAYLOAD")"
setup_bytes="$(wc -c < "$SETUP" | tr -d ' ')"

echo
printf 'built %s\n' "$SETUP"
printf '  km remote %s\n' "$VERSION"
printf '  the windowed program alone: no PATH entry, no file association, no ffmpeg\n'
printf '  %s bytes (~%s MiB), from a %s MiB payload\n' \
  "$setup_bytes" "$((setup_bytes / 1024 / 1024))" "$((payload_bytes / 1024 / 1024))"
if [ "$skipped_profiles" -gt 0 ]; then
  printf '  skipped %s WebView2 profile folder(s) in the payload -- runtime state, not ours to ship\n' \
    "$skipped_profiles"
fi

# -- the round trip ---------------------------------------------------------------------------------
#
# **Tested rather than asserted**, which is this repository's rule for a carrier: install it, run what
# it installed with nothing on PATH to rescue it, uninstall it, and look. The harness is
# tools/platform/windows/inno.sh's, shared with the all-in-one -- the uninstaller runs before anything
# is deleted, because deleting the folder leaves the registration behind and `unins000.exe` was inside
# the folder.

echo
dist_step "round trip"

scratch_root="$(mktemp -d)"
trap 'inno_cleanup_scratch "$scratch_root"' EXIT

fail() { echo "installer-remote: $*" >&2; exit 1; }

lean="$scratch_root/remote"
inno_install_to "$SETUP" "$lean" || fail "the silent install failed"
[ -f "$lean/unins000.exe" ] || fail "no uninstaller in $lean -- the install did not complete"

# The exit status, not the output: what is being proved is that the process got as far as running its
# own code, which means every library it imports resolved with nothing else on PATH.
[ -f "$lean/km-remote.exe" ] || fail "km-remote.exe was not installed"
( cd "$lean" && PATH="/c/Windows/System32:/c/Windows" ./km-remote.exe --version >/dev/null 2>&1 ) \
  || fail "km-remote.exe would not start from an installed folder with a bare PATH"

# **What this carrier is, said as a list of what is not in it.** Each of these is a promise the
# report above makes out loud, and each would be broken by one careless [Files] line.
[ -f "$lean/km-remote-console.exe" ] \
  && fail "the console twin was installed; this installs the windowed program only"
for exe in karaokemachine km-package-builder km-package-simple km-admin km-pack km-lyrics km-wallpaper-pack; do
  [ -f "$lean/$exe.exe" ] && fail "$exe.exe was installed by the remote's own setup program"
done
[ -d "$lean/assets" ] && fail "the assets folder was installed -- the remote reads no bank"
for dll in "${DIST_FFMPEG_DLLS[@]}"; do
  [ -f "$lean/$dll" ] && fail "$dll was installed -- the remote links no ffmpeg"
done

# The obligation, under test rather than asserted: MIT asks that the notice be in every copy, and
# there is no component tick here that could have dropped it.
[ -f "$lean/LICENSE-MIT.txt" ]    || fail "LICENSE-MIT.txt was not installed"
[ -f "$lean/LICENSE-APACHE.txt" ] || fail "LICENSE-APACHE.txt was not installed"
[ -f "$lean/ffmpeg-LICENSE.txt" ] && fail "an ffmpeg notice was installed beside no ffmpeg"

# **Both READMEs, and each proved to be the one it should be.** The short one is written for an
# installed build; the long one is the program's own document. Checking both halves is what stops a
# Source line being swapped back without anybody noticing -- the `Read me first` Start Menu entry
# points at the first of them.
[ -f "$lean/README.txt" ] || fail "no README.txt installed -- the 'Read me first' entry points at nothing"
grep -qi 'Add or remove programs' "$lean/README.txt" \
  || fail "the installed README does not say how to remove an installed build"
grep -qi 'km-remote-console' "$lean/README.txt" \
  && fail "the installed README names the console twin; it is the folder document, not dist_installed_readme's"
[ -f "$lean/README-km-remote.txt" ] || fail "the program's own README was not installed"
grep -qi 'Running it' "$lean/README-km-remote.txt" \
  || fail "README-km-remote.txt is not the remote's own document"

# The [UninstallDelete] line, checked deterministically. **What is being tested is that the
# uninstaller removes a directory it did not install**, so the folder is planted rather than waited
# for -- a current build makes none, and where it came from is not the point.
mkdir -p "$lean/km-remote.exe.WebView2/EBWebView"
printf 'profile\n' > "$lean/km-remote.exe.WebView2/EBWebView/marker"

inno_uninstall_from "$lean"
[ -d "$lean" ] && fail "the uninstaller left $lean behind (WebView2 profile not removed?)"

echo "verified: installs, runs with a bare PATH, uninstalls clean, carries neither the"
echo "          console twin nor any other product, and both READMEs are the ones meant"
echo "          for an installed build."

echo
echo "now:  ./$SETUP"
