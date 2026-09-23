#!/usr/bin/env bash
#
# Builds the Windows setup program: one installer carrying every product this repository makes.
#
#   tools/platform/windows/installer.sh              # stage everything, compile the installer, test it
#   tools/platform/windows/installer.sh --no-build   # compile from what is already staged; build nothing
#   tools/platform/windows/installer.sh -v           # watch the staging and the compile; quiet is the default
#
#   -> dist/setup/windows/karaokemachine-setup-<version>-windows-x86_64.exe
#
# **It gathers; it does not build.** The payload is `dist/bin/windows`, which tools/dist/bin.sh
# produces -- so every fact about which crate takes `video`, which four DLLs are staged and what each
# README says stays in tools/dist/cmd.sh and tools/platform/windows/dist.sh, and none of it is restated
# here. This script runs that one, checks what came out, and hands it to Inno Setup.
#
# **There is no --no-video.** The installer is always the build that can play every kind of song a
# catalog can hold; a person double-clicking setup.exe is not choosing a feature matrix. The
# portable folder is still where `--no-video` means something. If ffmpeg is missing this stops rather
# than quietly producing a setup that lists video songs and refuses to read them. See the
# `What an installed build contains` decision in docs/decisions/.
#
# **The console twins are not installed.** dist-bin.sh's other folder, dist/bin-console/windows, is
# not opened at all: `karaokemachine-console.exe` and its two siblings exist so that a Windows
# *folder* gives somebody something to type, and an installed program has a Start Menu entry and a
# PATH instead.
#
# Prerequisite: Inno Setup 6. `winget install JRSoftware.InnoSetup`. Nothing else -- the payload is
# already built by the time this needs it.

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/dist/common.sh
. tools/platform/windows/inno.sh
DIST_SCRIPT=installer

TARGET="x86_64"
ISS="tools/platform/windows/installer.iss"
BUILD=1

for arg in "$@"; do
  case "$arg" in
    --no-build) BUILD=0 ;;
    -v|--verbose) DIST_VERBOSE=1 ;;
    -h|--help)
      echo "usage: tools/platform/windows/installer.sh [--no-build] [-v]"
      exit 0 ;;
    *) echo "installer: unknown option $arg" >&2; exit 2 ;;
  esac
done

if [ "$(dist_platform)" != "windows" ]; then
  echo "installer: this builds a Windows setup program and has to run on Windows." >&2
  echo "           On macOS and Linux the carriers are tools/platform/macos/app-bundle.sh and" >&2
  echo "           tools/platform/linux/deb.sh; see CLAUDE.md." >&2
  exit 1
fi

# -- the compiler ---------------------------------------------------------------------------------
#
# Resolved before anything is built, so a missing install fails in a second with the line that fixes
# it rather than after a six-minute staging run. Same discipline `dist_ffmpeg_dir` applies to ffmpeg.
# The search itself is in tools/platform/windows/inno.sh, shared with the remote's setup program.
inno_require_compiler

# -- the payload ----------------------------------------------------------------------------------

PAYLOAD="$(dist_dir bin windows)"

if [ "$BUILD" -eq 1 ]; then
  dist_step "staging every product"
  staging_started=$SECONDS
  args=()
  if dist_verbose; then args=(-v); fi
  dist_run "dist-bin.sh" tools/dist/bin.sh "${args[@]+"${args[@]}"}"
  printf '   staged in %s\n' "$(dist_elapsed "$staging_started")"
fi

if [ ! -d "$PAYLOAD" ]; then
  echo "installer: nothing staged at $PAYLOAD." >&2
  if [ "$BUILD" -eq 0 ]; then
    echo "           --no-build was given, so nothing was staged for it here either. Run" >&2
    echo "           tools/platform/windows/installer.sh without it, or tools/dist/bin.sh first." >&2
  fi
  exit 1
fi

# **Asked of the folder, never of a flag.** There is no --no-video here, so this is not choosing
# between two builds -- it is refusing to make an installer out of a payload that cannot play video,
# which would otherwise be a silent downgrade of what the setup promises.
missing_dlls=()
for dll in "${DIST_FFMPEG_DLLS[@]}"; do
  [ -f "$PAYLOAD/$dll" ] || missing_dlls+=("$dll")
done
if [ "${#missing_dlls[@]}" -gt 0 ]; then
  echo "installer: the staged folder has no ffmpeg -- missing ${missing_dlls[*]}." >&2
  echo "           The setup program is always a video build, so this cannot be packaged." >&2
  echo "           Run tools/setup/fetch-ffmpeg.sh once on this machine, then this script again." >&2
  echo "           (tools/dist/bin.sh --no-video stages the smaller folder; it is not installable.)" >&2
  exit 1
fi

# The same list, asserted against the .iss rather than trusted to have been kept in step. Two places
# name these DLLs and only one of them is bash; this is the line that notices when the other stops
# agreeing -- which is what happens the day ffmpeg's soname moves to avcodec-62.
for dll in "${DIST_FFMPEG_DLLS[@]}"; do
  if ! grep -qF "\\$dll\"" "$ISS"; then
    echo "installer: $ISS does not install $dll, which DIST_FFMPEG_DLLS says is linked." >&2
    echo "           tools/dist/common.sh and the [Files] section have drifted apart." >&2
    exit 1
  fi
done

# -- what the .iss says it installs, against what is actually there --------------------------------
#
# **Read out of the .iss rather than restated here**, by `inno_coverage_check` in
# tools/platform/windows/inno.sh: the whole point is that the two cannot disagree, so it parses the
# [Files] section for its `Source:` paths and expands them against the payload.
#
# **`README.txt` is the one exclusion, and it is deliberately not installed.** It is the folder
# document tools/dist/bin.sh writes for `dist/bin/<platform>`, and it is right about that folder and
# wrong about an installed one in almost every sentence -- see the block above the ISCC call. An
# installed build gets `dist_installed_readme`'s text instead, from `{#Generated}`. `README-*.txt`,
# the per-product ones, are still installed unconditionally.
inno_coverage_check "$ISS" "$PAYLOAD" README.txt || exit 1

skipped_profiles="$(inno_skipped_profiles "$PAYLOAD")"

# -- what the components are, read out of the .iss too ---------------------------------------------
#
# **Parsed rather than restated**, for the same reason the [Files] block above is. The round trip
# installs "everything" by naming the components, and a list of them written out in bash is a list
# that stops agreeing the day a product is added -- which is not a hypothetical: `km-admin` arrived
# as a fifth component, the verification install went on asking for the first four, and it then
# asserted the executable it had never selected. The `.iss` is where the set is written down.
COMPONENTS=()
while IFS= read -r component; do COMPONENTS+=("$component"); done < <(inno_components "$ISS")
# A parse that came back empty -- or without the one component every install offers -- would leave
# the round trip selecting nothing and proving nothing. That is worse than a build that breaks,
# because it passes.
case " ${COMPONENTS[*]} " in
  *" machine "*) ;;
  *) echo "installer: no [Components] found in $ISS; the round trip would verify nothing." >&2
     exit 1 ;;
esac

# -- the version ----------------------------------------------------------------------------------
#
# From the binary, never the manifest: every shipped crate says `version.workspace = true`, so
# reading it means picking the right one of many `version =` lines, and asking the binary cannot
# disagree with the binary.
#
# **Deliberately the GUI-subsystem executable**, though this folder has no console twin to ask
# instead. Standard handles are inherited whatever the subsystem, so a GUI-subsystem process answers
# `--version` perfectly well down the pipe dist_version puts it on -- tools/platform/windows/dist.sh:131-137
# depends on exactly that and says so, having verified it on Windows.
VERSION="$(dist_version "$PAYLOAD/karaokemachine.exe")"

OUTDIR="$(dist_dir setup windows)"
OUTBASE="karaokemachine-setup-$VERSION-windows-$TARGET"
mkdir -p "$OUTDIR"
rm -f "$OUTDIR/$OUTBASE.exe"

# -- the README an installed build gets ------------------------------------------------------------
#
# **Not the payload's own README.txt**, which describes a folder somebody unpacked: it says the
# folder holds every executable this platform can build, lists all seven from a scan of the staging
# folder, and tells you to remove it by deleting the folder because nothing was registered. A setup
# program installs what was ticked, writes a Start Menu group, and -- if the tasks were left on -- a
# PATH entry and a file association, so every one of those sentences is false here. It was also the
# `Read me first` Start Menu entry, which made it the one document a new user was pointed at.
#
# Generated into a directory of its own rather than written into the payload, because `dist/bin` is
# itself a carrier somebody is handed: a second README sitting in it, addressed to people who
# installed rather than unzipped, would be one more thing to explain there. The text is
# `dist_installed_readme` in tools/dist/common.sh, shared with the macOS package so the two setup
# programs cannot come to describe removing the same product differently.
GENDIR="$OUTDIR/generated"
dist_clear "$GENDIR"
dist_installed_readme windows > "$GENDIR/README.txt"

# -- the bank the tick box offers -------------------------------------------------------------------
#
# The recommended bank is 261.9 MiB against a 30.9 MiB shipped tree, so it cannot travel in the
# carrier and it is far too much to download during an install. What the tick box writes is a
# *request*: `crates/machine/karaokemachine/src/firstrun.rs` reads it on the machine's first start
# and fetches the bank there, with a progress line on the television and two more starts to try
# again on. **This installer downloads nothing but the WebView2 bootstrapper**, which is what
# `What setup fetches` in docs/decisions/distribution.md still says.
#
# Read out of the table rather than written here, through the shell reader that already exists --
# `tools/setup/soundfont-banks.sh`, which `fetch-assets.sh` and three `tools/dev/` scripts also
# source. A hand-kept copy of a size or a license is exactly the drift that arrangement prevents,
# and this one would be visible: the wording is on a wizard page somebody reads before agreeing.
. tools/setup/soundfont-banks.sh

BANK_ID=""
for name in $KM_BANKS; do
  km_bank "$name"
  [ "$SF_RECOMMENDED" = "1" ] || continue
  if [ -n "$BANK_ID" ]; then
    echo "installer: two banks are marked recommended ($BANK_ID and $name)." >&2
    echo "           Exactly one row may carry it -- see banks.rs's own test." >&2
    exit 1
  fi
  BANK_ID="$name"
  BANK_NAME="$SF_NAME"
  BANK_SIZE="$SF_SIZE"
  BANK_LICENSE="$SF_LICENSE"
  BANK_STATUS="$SF_STATUS"
done

if [ -z "$BANK_ID" ]; then
  echo "installer: no bank is marked recommended in the table." >&2
  exit 1
fi
if [ "$BANK_STATUS" = "manual" ]; then
  echo "installer: the recommended bank ($BANK_ID) has no direct download address." >&2
  echo "           A tick box offering a download nothing can perform is worse than no tick box." >&2
  exit 1
fi

# What the tick box leaves behind, verbatim. The concrete id and not `recommended`: the bank
# somebody was offered by name and size on the wizard page is the bank that should arrive, even if
# the table's recommendation moves between this release and their first start.
printf '{\n  "bank": "%s"\n}\n' "$BANK_ID" > "$GENDIR/first-run-soundfont.json"
dist_detail "soundfont  $BANK_ID -> $BANK_NAME ($BANK_SIZE)"

# **What makes an installed machine drive a television.** `display.fullscreen` defaults to *off*,
# because the other thing with no settings file is a checkout somebody has just built, and a default
# reaches both. What tells the two apart is who put the machine there -- so the setup program says
# so, in the file the machine already reads.
#
# **Two keys and not a whole settings file.** Every settings struct is `#[serde(default)]`, so this
# is a complete one: the machine fills the rest from its own defaults and writes it back whole on
# the first start. Nothing here needs to know what else is in that file, or track it.
#
# Placed with `onlyifdoesntexist` in the .iss, which is the whole of the safety argument -- see
# `The setup programs pre-write a settings file` in docs/decisions/distribution.md. An installer that
# overwrote a settings file it did not create would lose whatever an upgrade was standing on.
printf '{\n  "display": {\n    "fullscreen": true\n  }\n}\n' > "$GENDIR/settings.json"
dist_detail "settings   fullscreen: true, where there is no settings.json yet"

# **The license has its apostrophes doubled, and the other two deliberately do not.** ISPP's
# `{#Define}` is textual substitution, so a license reading `the author's` lands inside a `'...'`
# literal in [Code] as an unterminated string and the compile fails on a line nobody wrote. The
# recommended bank's license says exactly that today, so this is the live case and not a precaution.
#
# The name and the size are left alone because they go into a [Tasks] description as well, which is
# plain text and would show a doubled apostrophe as two of them. Neither can carry one -- one is a
# filename and the other is a number and a unit -- and if that ever stopped being true the compile
# fails rather than the wording going out wrong, which is the failure to prefer.
BANK_LICENSE_ISS="${BANK_LICENSE//\'/\'\'}"

# -- compile ---------------------------------------------------------------------------------------
#
# MSYS2_ARG_CONV_EXCL is not optional and not cosmetic. Git Bash rewrites any argument that looks
# like a POSIX path, so `/DPayload=...` arrives at a Windows program as `C:/Program Files/Git/DPayload
# =...` -- the same trap that makes `makensis /VERSION` report that it cannot open a script called
# VERSION. Every value is additionally converted to a Windows path, because ISCC is a Windows program
# and has never heard of /c/prog.
dist_step "compiling the installer"
compile_started=$SECONDS
MSYS2_ARG_CONV_EXCL='*' dist_run "iscc" "$ISCC" \
  "/DPayload=$(host_path "$PWD/$PAYLOAD")" \
  "/DGenerated=$(host_path "$PWD/$GENDIR")" \
  "/DVersion=$VERSION" \
  "/DBankName=$BANK_NAME" \
  "/DBankSize=$BANK_SIZE" \
  "/DBankLicense=$BANK_LICENSE_ISS" \
  "/DOutDir=$(host_path "$PWD/$OUTDIR")" \
  "/DOutBase=$OUTBASE" \
  "$(host_path "$PWD/$ISS")"
printf '   compiled in %s\n' "$(dist_elapsed "$compile_started")"

SETUP="$OUTDIR/$OUTBASE.exe"
if [ ! -f "$SETUP" ]; then
  echo "installer: $SETUP was not produced" >&2
  exit 1
fi

# -- report ----------------------------------------------------------------------------------------

payload_bytes="$(dist_bytes "$PAYLOAD")"
setup_bytes="$(wc -c < "$SETUP" | tr -d ' ')"

echo
printf 'built %s\n' "$SETUP"
printf '  karaoke machine %s\n' "$VERSION"
printf '  components: %s\n' "$(printf '%s, ' "${COMPONENTS[@]}" | sed 's/, $//')"
printf '  video yes  (%s ffmpeg DLLs)\n' "${#DIST_FFMPEG_DLLS[@]}"
printf '  %s bytes (~%s MiB), from a %s MiB payload\n' \
  "$setup_bytes" "$((setup_bytes / 1024 / 1024))" "$((payload_bytes / 1024 / 1024))"
if [ "$skipped_profiles" -gt 0 ]; then
  printf '  skipped %s WebView2 profile folder(s) in the payload -- runtime state, not ours to ship\n' \
    "$skipped_profiles"
fi

# -- the round trip ---------------------------------------------------------------------------------
#
# **Tested rather than asserted**, which is this repository's rule for a carrier: tools/platform/windows/dist.sh
# proves its folder is self-contained by starting what it staged with a PATH stripped to Windows
# itself. This is the same check one level up -- install it, run everything, uninstall it, and look.
#
# It catches the two ways an installer of this shape goes wrong, neither of which the compiler can
# see: a file in the payload that no component actually installs, and a component whose DLLs were
# selected out from under it.

echo
dist_step "round trip"

# The three helpers are in tools/platform/windows/inno.sh, shared with the remote's setup program --
# the reason they are not copied is written there, and it is the orphaned uninstall registration this
# trap exists to prevent.
scratch_root="$(mktemp -d)"
trap 'inno_cleanup_scratch "$scratch_root"' EXIT

install_to()     { inno_install_to "$SETUP" "$@"; }
uninstall_from() { inno_uninstall_from "$@"; }

fail() { echo "installer: $*" >&2; exit 1; }

# ---- everything, and prove it runs -------------------------------------------------------------

full="$scratch_root/full"
install_to "$full" "$(IFS=,; printf '%s' "${COMPONENTS[*]}")" || fail "the silent install failed"
[ -f "$full/unins000.exe" ] || fail "no uninstaller in $full -- the install did not complete"

# The exit status, not the output: what is being proved is that the process got as far as running its
# own code, which means every DLL it imports resolved from the install folder with nothing else on
# PATH to rescue it.
for exe in karaokemachine km-package-builder km-package-simple km-remote km-admin km-pack km-lyrics km-wallpaper-pack; do
  [ -f "$full/$exe.exe" ] || fail "$exe.exe was not installed"
  ( cd "$full" && PATH="/c/Windows/System32:/c/Windows" "./$exe.exe" --version >/dev/null 2>&1 ) \
    || fail "$exe.exe would not start from an installed folder with a bare PATH"
done
( cd "$full" && PATH="/c/Windows/System32:/c/Windows" ./karaokemachine.exe --show-paths >/dev/null 2>&1 ) \
  || fail "karaokemachine.exe --show-paths failed from an installed folder"

find "$full/assets" -name '*.sf2' 2>/dev/null | grep -q . \
  || fail "no SoundFont installed -- the machine would come up on a test tone"

# **The tick box was switched off, so nothing may have been asked for.** This is the only assertion
# in the round trip about a file outside the scratch directory, and it is the negative one on
# purpose: /TASKS="" is what keeps a verification run from touching this machine, and the request
# lands in %APPDATA% rather than in {app} -- so the check that matters is that a declined tick box
# writes nothing there. The positive case cannot be tested here at all: Inno resolves {userappdata}
# through the shell folders, which no environment variable can redirect, so proving it would mean
# writing into the developer's own install. The bank the machine would fetch is covered by
# `firstrun`'s tests instead.
#
# `cygpath -u` because %APPDATA% arrives as a Windows path with backslashes and `test -f` is given it
# literally; the same conversion `host_path` does in the other direction.
appdata_posix="$(cygpath -u "${APPDATA:-}" 2>/dev/null || printf '%s' "${APPDATA:-}")"
if [ -n "$appdata_posix" ] \
  && [ -f "$appdata_posix/karaokemachine/config/first-run-soundfont.json" ]; then
  fail "an install with /TASKS=\"\" left a first-start SoundFont request in %APPDATA%"
fi

# ...and the file that tick box would have installed was generated, from the row the table marks.
[ -f "$GENDIR/first-run-soundfont.json" ] \
  || fail "the first-start SoundFont request was not generated"
grep -q "\"$BANK_ID\"" "$GENDIR/first-run-soundfont.json" \
  || fail "the generated request does not name $BANK_ID"

# ...and so was the settings file that makes an installed machine fullscreen. Checked for the key
# and not merely for the file, because an empty or half-written one would install silently and leave
# a television in a 1280x720 window with nothing saying why.
[ -f "$GENDIR/settings.json" ] \
  || fail "the first-start settings file was not generated"
grep -q '"fullscreen": true' "$GENDIR/settings.json" \
  || fail "the generated settings file does not ask for fullscreen"
for dll in "${DIST_FFMPEG_DLLS[@]}"; do
  [ -f "$full/$dll" ] || fail "$dll was not installed with the full selection"
done
[ -f "$full/karaokemachine-console.exe" ] \
  && fail "a console twin was installed; the setup is meant to install the windowed programs only"

# **The README that was installed is the one written for an installed build**, not the folder
# document from the payload. This is asserted rather than trusted because nothing here ever read the
# installed README before, which is how every setup program built so far came to tell people -- from the
# `Read me first` Start Menu entry, no less -- to remove the product by deleting the folder, beside
# an uninstaller, a Start Menu group, a PATH entry and a file association. Both halves are checked:
# a phrase only the folder document has, and one only the installed one has, so swapping the Source
# line back fails here rather than in somebody's hands.
[ -f "$full/README.txt" ] || fail "no README.txt installed -- the 'Read me first' entry points at nothing"
grep -qi 'nothing was registered' "$full/README.txt" \
  && fail "the installed README says nothing was registered; it is the folder document, not dist_installed_readme's"
grep -qi 'Add or remove programs' "$full/README.txt" \
  || fail "the installed README does not say how to remove an installed build"
[ -f "$full/README-karaokemachine.txt" ] \
  || fail "the per-product READMEs were not installed; they are how the other components are found"

# The [UninstallDelete] line, checked deterministically. **What is being tested is that the
# uninstaller removes a directory it did not install**, and the folder is planted here rather than
# produced: waiting for WebView2 to make one on the first window would mean a build script starting a
# GUI program and waiting on it, and since `Where a webview keeps its profile` it makes none there at
# all. Where the directory came from is not the point -- planting it is what keeps the check honest.
mkdir -p "$full/km-package-builder.exe.WebView2/EBWebView"
printf 'profile\n' > "$full/km-package-builder.exe.WebView2/EBWebView/marker"

uninstall_from "$full"
[ -d "$full" ] && fail "the uninstaller left $full behind (WebView2 profile not removed?)"

# ---- one component alone, to prove the wiring saves what it claims to ---------------------------

lean="$scratch_root/lean"
install_to "$lean" "remote" || fail "the remote-only silent install failed"
[ -f "$lean/km-remote.exe" ] || fail "km-remote.exe missing from a remote-only install"
[ -f "$lean/karaokemachine.exe" ] && fail "the machine was installed by a remote-only selection"
# `assets/` here is the payload *folder* of SoundFonts and wallpapers, not the component of the same
# name -- the machine's 140 MB, which a remote-only selection has no use for.
[ -d "$lean/assets" ] && fail "the assets folder was installed by a remote-only selection (140 MB not saved)"
for dll in "${DIST_FFMPEG_DLLS[@]}"; do
  [ -f "$lean/$dll" ] && fail "$dll was installed by a remote-only selection"
done
# **The argument the [Files] comment makes, under test.** The READMEs are installed with no
# `Components:` on the grounds that they are how somebody who ticked one thing finds out the others
# exist -- so the selection that ticked one thing is where that has to hold, and it is the
# selection nothing was checking it for.
[ -f "$lean/README.txt" ] \
  || fail "no README installed by a remote-only selection; nothing would tell that user the other components exist"
grep -q 'KM Package Builder' "$lean/README.txt" \
  || fail "the installed README does not name the components that were left out"
uninstall_from "$lean"
[ -d "$lean" ] && fail "the uninstaller left $lean behind"

echo "verified: installs, runs with a bare PATH, uninstalls clean, a remote-only"
echo "          selection carries neither the assets nor the ffmpeg DLLs, and the"
echo "          README it installs is the one written for an installed build."

echo
echo "now:  ./$SETUP"
