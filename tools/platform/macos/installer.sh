#!/usr/bin/env bash
#
# Builds the macOS setup program: one .pkg carrying every product this repository makes.
#
#   tools/platform/macos/installer.sh              # stage everything, build the package, check it
#   tools/platform/macos/installer.sh --notarize   # ...signed and notarized, the one you can hand over
#   tools/platform/macos/installer.sh --no-build   # build from what is already staged; build nothing
#   tools/platform/macos/installer.sh --install    # ...and do the real sudo install/uninstall round trip
#   tools/platform/macos/installer.sh -v           # watch the staging and the build; quiet is the default
#
#   -> dist/setup/macos/karaokemachine-setup-<version>-macos-<arch>.pkg              (--notarize)
#      ...-macos-<arch>-unnotarized.pkg   signed only, which spctl still refuses
#      ...-macos-<arch>-unsigned.pkg      ad-hoc, the default
#
# **It gathers; it does not build.** The payload is `dist/bin/macos` **and** `dist/bin-console/macos`,
# which tools/dist/bin.sh produces -- so every fact about which crate takes `video`, how a macOS load
# command is rewritten and what each README says stays in tools/dist/cmd.sh and
# tools/platform/macos/app-bundle.sh, and none of it is restated here. This script runs that one,
# checks what came out, and hands it to pkgbuild. The same bargain
# tools/platform/windows/installer.sh makes, one platform over.
#
# **Two folders rather than one, because on macOS a product with two forms is split across them.**
# `bin/` holds what you double-click and `bin-console/` what has somewhere to print, and macOS spells
# that pair as a bundle in the first beside the executable it wraps in the second. An installed build
# wants both halves and puts them in different places: the `.app` in /Applications, the console
# executable in /usr/local/bin. Reading only `bin/` is what this script did until the split existed,
# and it silently cost `/usr/local/bin` three of its six commands -- see the second staging pass
# below.
#
# **There is no --no-video.** The installer is always the build that can play every kind of song a
# catalog can hold; a person double-clicking a .pkg is not choosing a feature matrix. The portable
# folder is still where `--no-video` means something. If ffmpeg is missing this stops rather than
# quietly producing a setup that lists video songs and refuses to read them.
#
# **System domain, and that is the opposite of the Windows answer for the same reason.** There,
# everything the installer configures beyond the files is per-user, so a machine-wide install would
# configure for one account what it installed for all. Here the association is declared by a bundle
# in /Applications and the PATH entry is a symlink in /usr/local/bin, both machine-wide by
# convention. One administrator prompt is the price, and on macOS it is what every installer costs.
#
# Prerequisite: nothing. pkgbuild, productbuild and pkgutil are in the base system.

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/dist/common.sh
. tools/platform/macos/pkg.sh
DIST_SCRIPT=installer

RES=tools/platform/macos/pkg                  # the Distribution, the two panes, the shared text
UNINSTALL_TEMPLATE=tools/platform/macos/uninstall.sh
UNINSTALL_COMMAND=tools/platform/macos/uninstall.command

PRODUCT=com.rrgmc.karaokemachine
COMPONENTS=(machine builder simple remote admin tools docs)

# **Payload-free components, which is why they are a second array rather than a sixth entry.**
# Everything the list above drives is about files: `claim()` assigns each staged entry to one of
# them, two checks reconcile what was staged against what was archived, and `install_location` says
# where a payload lands. A component that installs no files answers none of those questions, and
# adding it to that array would mean five special cases reading `if [ "$comp" != soundfont ]`.
#
# There is one: `soundfont`, whose whole content is a postinstall that writes down a bank the person
# installing asked for. See the tick box's own section below.
SCRIPT_COMPONENTS=(soundfont)

BUILD=1
INSTALL=0
NOTARIZE=0

for arg in "$@"; do
  case "$arg" in
    --no-build) BUILD=0 ;;
    --install) INSTALL=1 ;;
    --notarize) NOTARIZE=1 ;;
    -v|--verbose) DIST_VERBOSE=1 ;;
    -h|--help)
      echo "usage: tools/platform/macos/installer.sh [--no-build] [--notarize] [--install] [-v]"
      exit 0 ;;
    *) echo "installer: unknown option $arg" >&2; exit 2 ;;
  esac
done

if [ "$(dist_platform)" != "macos" ]; then
  echo "installer: this builds a macOS package and has to run on macOS." >&2
  echo "           On Windows and Linux the carriers are tools/platform/windows/installer.sh and" >&2
  echo "           tools/platform/linux/deb.sh; see CLAUDE.md." >&2
  exit 1
fi

# -- the tools ---------------------------------------------------------------------------------------
#
# Resolved before anything is built, the same discipline `inno_require_compiler` applies on Windows:
# a missing tool should cost a second, not a six-minute staging run. They all ship with macOS, so the
# check is really that somebody has not stripped the command line tools. The list is in
# tools/platform/macos/pkg.sh, shared with the remote's own setup program.
pkg_require_tools

# -- signing -------------------------------------------------------------------------------------------
#
# Two certificates, three states and every refusal are in tools/platform/macos/pkg.sh, shared with the
# remote's own setup program: a resolution that has lost one of its refusals produces a package
# Gatekeeper rejects and a report saying it worked. This sets KM_SIGN_INSTALLER_IDENTITY,
# KM_NOTARY_PROFILE, SIGNING_STATE and SIGNING_MARKER, or stops before anything is staged.
pkg_resolve_signing "$NOTARIZE"

# -- the claim table ----------------------------------------------------------------------------------
#
# **The single source of truth: which component each payload entry belongs to.** The staging loop
# below drives off this, so an entry in dist/bin/macos cannot be staged without being claimed, and
# cannot be claimed without being staged. That is what the Windows driver gets by parsing [Files]
# back out of the .iss -- there the description and the payload are two things that can disagree, and
# here they are one thing.
#
# The destination split is what makes it five packages and not four: pkgbuild takes one --root and
# one --install-location, so /Applications and /usr/local/karaokemachine cannot share a package.
# `docs` is the hidden always-on one, mirroring the .iss's unconditional README lines.
claim() { # <basename of a top-level payload entry> -> prints the component, or fails
  case "$1" in
    "Karaoke Machine.app")      printf 'machine' ;;
    # The same component as the bundle above, because it is the same program: it holds a launch
    # script and an icon, and what it starts is the binary inside `Karaoke Machine.app`. A component
    # of its own could be declined, which would install a launcher pointing at nothing.
    "KM Stream.app") printf 'machine' ;;
    "KM Package Builder.app")  printf 'builder' ;;
    "KM Simple Package.app")   printf 'simple'  ;;
    "KM Remote.app")           printf 'remote'  ;;
    "KM Admin.app")            printf 'admin'   ;;
    # Every bare executable and the libraries they share. `lib/` has to sit beside them: every one
    # of them was staged with `@executable_path/lib`, and splitting them across components would mean
    # either a second copy of those 15 MB or a package that cannot stand on its own.
    km-pack|km-lyrics|km-wallpaper-pack|km-package-builder|km-package-simple|km-remote|km-admin|lib)
                                   printf 'tools'   ;;
    # The READMEs and this workspace's own license texts. `docs` is the hidden always-on component,
    # so these are installed whatever was ticked -- which for the licenses is not a convenience:
    # MIT asks that the notice travel with every copy, and a component tick is exactly the thing
    # that must not be able to drop it. Same reasoning as the .iss giving them no `Components:`.
    # `README.txt` is deliberately NOT here -- it is in `excluded()` below, and naming it in both
    # would leave two functions asserting opposite things about one file with the staging loop's
    # evaluation order deciding which wins.
    README-*.txt|LICENSE-MIT.txt|LICENSE-APACHE.txt) printf 'docs' ;;
    *) return 1 ;;
  esac
}

# Subtracted by name and out loud, with the count reported rather than dropped in silence. This is
# the macOS counterpart of `*.WebView2` on Windows: somebody else's metadata sitting in the payload,
# which must be neither shipped nor quietly ignored. The Finder writes one the moment anybody opens
# the staged folder in a window, and a staged folder is not cleared between builds.
excluded() { # <basename> -> prints why, or fails
  case "$1" in
    .DS_Store) printf "the Finder's own folder metadata, not a build artifact" ;;
    # **Left out on purpose, and replaced rather than dropped.** This is the folder document
    # tools/dist/bin.sh writes for `dist/bin/<platform>`: it says the folder holds every executable
    # this platform can build, lists all seven from a scan of the staging folder, and tells you to
    # remove it by deleting the folder because nothing was installed elsewhere and nothing was
    # registered. Installed, that lands in /usr/local/karaokemachine beside this package's own
    # uninstaller, with the applications in /Applications, six symlinks in /usr/local/bin and five
    # pkgutil receipts -- so every clause of it is false. `dist_installed_readme` writes the one
    # that goes in instead, below, beside the uninstaller and into the same always-on component.
    README.txt) printf "the folder document; dist_installed_readme writes the installed one" ;;
    *) return 1 ;;
  esac
}

# Where each component's payload lands on the target.
install_location() { # <component>
  case "$1" in
    machine|builder|simple|remote|admin) printf '/Applications' ;;
    tools|docs)             printf '/usr/local/karaokemachine' ;;
  esac
}

# -- the payload ---------------------------------------------------------------------------------------

PAYLOAD="$(dist_dir bin macos)"
CONSOLE="$(dist_dir bin-console macos)"

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
    echo "           tools/platform/macos/installer.sh without it, or tools/dist/bin.sh first." >&2
  fi
  exit 1
fi

# **Asked separately, because a missing one of these is silent where a missing `$PAYLOAD` is loud.**
# The second staging pass globs this folder, so if it is not there the glob finds nothing, three
# commands quietly do not get staged and the failure surfaces minutes later in the round trip as
# `km-package-builder was not packaged` -- which reads as a packaging fault rather than as a folder
# that was never staged. `tools/dist/bin.sh` always writes both, so reaching this means somebody
# staged one by hand or deleted the other.
if [ ! -d "$CONSOLE" ]; then
  echo "installer: $PAYLOAD is staged but $CONSOLE is not." >&2
  echo "           Both are needed: the .app bundles come from the first and the command-line" >&2
  echo "           copies of the three bundled products from the second. Run tools/dist/bin.sh." >&2
  exit 1
fi

# **Asked of the folder, never of a flag.** There is no --no-video here, so this is not choosing
# between two builds -- it is refusing to make an installer out of a payload that cannot play video,
# which would be a silent downgrade of what the setup promises. The two conditions are the two places
# ffmpeg lands on this platform: `lib/` beside the flat commands, and Contents/Frameworks inside the
# machine's bundle. A --no-video bundle has no Frameworks directory at all, which is the exact signal.
#
# `DIST_FFMPEG_DLLS` is deliberately not used and there is no macOS twin of it: dist_stage_ffmpeg_macos
# walks the closure rather than listing it, so the set is four against the pinned LGPL build and
# thirteen against Homebrew's, and no file here may name any of them.
missing=()
compgen -G "$PAYLOAD/lib/libav*.dylib" >/dev/null || missing+=("lib/libav*.dylib")
compgen -G "$PAYLOAD/Karaoke Machine.app/Contents/Frameworks/libav*.dylib" >/dev/null \
  || missing+=("Karaoke Machine.app/Contents/Frameworks/libav*.dylib")
if [ "${#missing[@]}" -gt 0 ]; then
  echo "installer: the staged folder has no ffmpeg -- missing ${missing[*]}." >&2
  echo "           The setup package is always a video build, so this cannot be packaged." >&2
  echo "           Run tools/setup/fetch-ffmpeg.sh once on this machine, then this script again." >&2
  echo "           (tools/dist/bin.sh --no-video stages the smaller folder; it is not installable.)" >&2
  exit 1
fi

# **pkgbuild silently drops every .DS_Store it finds, at any depth**, by its own default filter --
# measured rather than taken from the man page: a throwaway root holding `keep.txt`, `.DS_Store`,
# `sub/also.txt` and `sub/.DS_Store` archives as the two `.txt` files and nothing else.
#
# A *top-level* one is `excluded()`'s job below and never reaches a staging root. A *nested* one --
# inside a bundle, which is where the Finder actually leaves them -- is copied by `ditto` and then
# dropped by pkgbuild, so the package would hold something different from what was staged and the
# round trip's staged-versus-archived comparison would fail one screen later with a message about a
# mismatch rather than about the cause. Caught here instead, before anything is built.
#
# **It does not break the bundle's signature, and the first version of this comment said it did.**
# `.DS_Store` is in codesign's own default exclusions, so it is never sealed: verified both ways on a
# copy of a staged bundle -- sealed with one present, then deleted, and `codesign --verify --deep
# --strict` passes in both states. What is being prevented is a carrier quietly differing from what
# it was handed, which is this repository's rule for a carrier and reason enough on its own.
stray="$(find "$PAYLOAD" -name .DS_Store -not -path "$PAYLOAD/.DS_Store" || true)"
if [ -n "$stray" ]; then
  echo "installer: a .DS_Store is sitting inside one of the staged bundles:" >&2
  printf '%s\n' "$stray" | sed 's|^|             |' >&2
  echo "           pkgbuild drops these silently, so the package would not hold what was staged." >&2
  echo "           Delete them and run this again; nothing has to be rebuilt." >&2
  exit 1
fi

# -- the version and the architecture ------------------------------------------------------------------
#
# From the binary, never the manifest: every shipped crate says `version.workspace = true`, so reading
# it means picking the right one of many `version =` lines, and asking the binary cannot disagree with
# the binary. On macOS the machine has no bare executable at all, so the one inside the bundle is what
# answers -- and it is the same read that put @VERSION@ in that bundle's Info.plist.
MACHINE_EXE="$PAYLOAD/Karaoke Machine.app/Contents/MacOS/karaokemachine"
VERSION="$(dist_version "$MACHINE_EXE")"

TARGET="$(dist_host_triple)"; TARGET="${TARGET%%-*}"      # aarch64 | x86_64

# Asked of the artifact, like the ffmpeg check above. An arm64 build says arm64; an x86_64 build says
# both, because Rosetta will run it and refusing an Apple silicon Mac a build it can execute would be
# wrong.
case "$(lipo -archs "$MACHINE_EXE")" in
  *arm64*) ARCHS="arm64" ;;
  *)       ARCHS="x86_64,arm64" ;;
esac

# What every bundle's manifest has to agree about, checked before a single one is packaged.
#
# **The plists are found by glob, not named.** What this replaced opened the machine's manifest alone
# while its own comment claimed three, and by then there were four. A fifth product is now covered the
# day its manifest is added rather than the day somebody remembers this list -- the same correction the
# component and executable counts in this script have already had.
#
# **`plutil` rather than `sed`, because a missing key has to be a failure.** A pattern match over a
# manifest that does not declare the key returns an empty string, and an empty string compares equal
# to nothing in particular -- it reads as agreement. A parser exits non-zero and says which file.
PLISTS=(tools/platform/macos/Info*.plist)
if [ "${#PLISTS[@]}" -lt 4 ]; then
  echo "installer: tools/platform/macos/Info*.plist matched ${#PLISTS[@]} file(s); expected at least four." >&2
  exit 1
fi

# The categories this product uses, and the whole list on purpose: membership rather than a
# `public.app-category.` prefix test, because the prefix is what a typo keeps.
CATEGORIES=(public.app-category.music public.app-category.utilities)

# The minimum is stated in five places -- the four manifests and the Distribution -- and this is what
# notices when they stop agreeing. A plist is the authority: it is what LaunchServices reads.
dist_min="$(sed -n 's/.*<os-version min="\([^"]*\)".*/\1/p' "$RES/distribution.xml")"

for plist in "${PLISTS[@]}"; do
  if ! plist_min="$(plutil -extract LSMinimumSystemVersion raw -o - "$plist" 2>/dev/null)"; then
    echo "installer: $plist declares no LSMinimumSystemVersion." >&2
    exit 1
  fi
  if [ "$plist_min" != "$dist_min" ]; then
    echo "installer: $plist says LSMinimumSystemVersion $plist_min and" >&2
    echo "           $RES/distribution.xml says os-version min $dist_min. They have drifted." >&2
    exit 1
  fi

  # A category is a string nothing else in the build reads, so it drifts silently: a bundle that
  # declares none shows a blank kind in Get Info, and one naming a category macOS does not know is
  # ignored without a word. Neither is visible in a build log, which is why it is visible here.
  if ! plist_cat="$(plutil -extract LSApplicationCategoryType raw -o - "$plist" 2>/dev/null)"; then
    echo "installer: $plist declares no LSApplicationCategoryType, so the bundle it makes has no" >&2
    echo "           category anywhere the system shows one. Expected: ${CATEGORIES[*]}" >&2
    exit 1
  fi
  # `case` rather than a loop with a test in it: this script runs under `set -e`, where a loop whose
  # last test fails takes the script with it. The same trap the staging loop below documents.
  case " ${CATEGORIES[*]} " in
    *" $plist_cat "*) ;;
    *)
      echo "installer: $plist says LSApplicationCategoryType $plist_cat, which is not one this" >&2
      echo "           product uses. Expected: ${CATEGORIES[*]}" >&2
      exit 1
      ;;
  esac
done

dist_step "macos setup package"
dist_detail "version   $VERSION"
dist_detail "arch      $TARGET (hostArchitectures $ARCHS)"
dist_detail "min os    $dist_min"

# -- staging -------------------------------------------------------------------------------------------

STAGE="$(mktemp -d)"
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT

mkdir -p "$STAGE/pkgs" "$STAGE/plists" "$STAGE/resources"
for comp in "${COMPONENTS[@]}"; do mkdir -p "$STAGE/root/$comp" "$STAGE/scripts/$comp"; done
# Scripts and no root: `pkgbuild --nopayload` is given no --root at all, and an empty one staged here
# would be a directory two later checks would then have to be told to ignore.
for comp in "${SCRIPT_COMPONENTS[@]}"; do mkdir -p "$STAGE/scripts/$comp"; done

SKIPPED=()
shopt -s nullglob dotglob
for entry in "$PAYLOAD"/*; do
  base="$(basename "$entry")"
  if why="$(excluded "$base")"; then
    SKIPPED+=("$base -- $why")
    continue
  fi
  if ! comp="$(claim "$base")"; then
    echo "installer: $PAYLOAD holds $base, which no component claims." >&2
    echo "           Add it to claim() with the component that needs it, or to excluded() with" >&2
    echo "           the reason. Nothing is dropped from a carrier by accident." >&2
    exit 1
  fi
  # `ditto` and never `cp -R`: a bundle is made of symlinks and a signature sealed over all of it,
  # and the copy has to preserve every one or `codesign --verify` stops agreeing it will load
  # elsewhere. The flat files do not care and take the same call.
  ditto "$entry" "$STAGE/root/$comp/$base"
done

# **The console halves, from the other folder, and only the ones the first pass did not find.**
#
# `tools/dist/bin.sh` rule 1 sends a bundled product's bare executable to `bin-console/` *alone*, so
# `km-package-builder`, `km-remote` and `km-admin` are in `bin/` only as `.app` bundles. Their
# command-line copies are what `/usr/local/bin` is made of, and reading one folder missed all three:
# the package built, every `.app` was correct, and an install quietly put three commands on the PATH
# instead of six. The round trip below is what caught it, three minutes after the fact.
#
# **Derived, never listed.** Which products have two forms is a question `bin.sh` already answers by
# looking, and repeating the answer here as three names is the copy that goes stale the first time
# there is a fourth. What is asked instead is the thing that is actually true: this folder holds a
# claimed entry that the first pass did not stage.
#
# **Unclaimed entries are passed over in silence here, where `$PAYLOAD` treats one as an error**, and
# the asymmetry is deliberate rather than a relaxed rule. `bin/` is the carrier -- everything in it
# ships, so a name nothing claims means somebody added a product and stopped halfway. `bin-console/`
# is not a carrier; it is a folder this script *draws two files out of*, and it legitimately holds
# things an installed build must not get: `karaokemachine`, whose installed form is the shim into the
# bundle, and `assets/`, the SoundFont and wallpapers that bare executable reads, which the machine's
# own component already carries inside `Karaoke Machine.app`. Both would be a second copy of something
# already installed, and `claim()` declining them is what says so.
#
# `if` rather than `[ … ] && continue`: this script runs under `set -e`, where a bare test that comes
# out false is a failing command and takes the shell with it.
# `CONSOLE_FILES` counts what this pass contributes, for the reconciliation below -- which balances
# what was staged against what the payload held and would otherwise read three extra files as three
# that appeared from nowhere. Counted as *files* rather than as entries, because an entry is allowed
# to be a directory and the reconciliation counts files.
CONSOLE_FILES=0
for entry in "$CONSOLE"/*; do
  base="$(basename "$entry")"
  if ! comp="$(claim "$base")"; then continue; fi
  if [ -e "$STAGE/root/$comp/$base" ]; then continue; fi
  ditto "$entry" "$STAGE/root/$comp/$base"
  CONSOLE_FILES=$((CONSOLE_FILES + $(find "$STAGE/root/$comp/$base" -type f | wc -l | tr -d ' ')))
  dist_detail "console   $base, which is in bin-console/ alone"
done
shopt -u nullglob dotglob

# The uninstaller, with the two substitutions. It rides with the READMEs, deliberately: the one way
# to take this off again must not be something a component tick could decline.
pkg_fill_uninstaller "$UNINSTALL_TEMPLATE" "$RES/data-locations.txt" \
                     "$VERSION" "$STAGE/root/docs/uninstall.sh"
chmod 755 "$STAGE/root/docs/uninstall.sh"
sh -n "$STAGE/root/docs/uninstall.sh"

# **And the double-clickable wrapper beside it**, which is the only part of this an end user is
# expected to find. The Finder has no handler for a `.sh`; `.command` is the extension it hands to
# Terminal, and the name is what makes the file's purpose obvious in a listing. It removes nothing
# itself -- uninstall.sh above stays the only thing that does.
pkg_fill_command "$UNINSTALL_COMMAND" "$VERSION" "$STAGE/root/docs/Uninstall KaraokeMachine.command"
chmod 755 "$STAGE/root/docs/Uninstall KaraokeMachine.command"
sh -n "$STAGE/root/docs/Uninstall KaraokeMachine.command"

# **The README an installed build gets**, in place of the payload's folder document that `excluded()`
# left behind above. It rides in `docs` with the uninstaller and the licenses, for the same reason
# they do: which components were ticked must not be able to decide whether somebody can find out how
# to remove this. The text is shared with the Windows setup program -- see `dist_installed_readme` in
# tools/dist/common.sh -- so the two cannot come to describe removing the same product differently,
# and the macOS half of it ends with the same tools/platform/macos/pkg/data-locations.txt block the
# conclusion pane and the uninstaller print, rather than a fourth copy of those four paths.
dist_installed_readme macos > "$STAGE/root/docs/README.txt"

# A component whose case arm matched nothing produces a smaller package and no error, which is the
# failure this notices. Same argument as dist-bin.sh's count reconciliation.
for comp in "${COMPONENTS[@]}"; do
  if [ -z "$(ls -A "$STAGE/root/$comp")" ]; then
    echo "installer: the $comp component staged nothing." >&2
    echo "           claim() names entries that are in neither $PAYLOAD nor $CONSOLE." >&2
    exit 1
  fi
done

# The strong half of the reconciliation: a `ditto` that silently dropped something *inside* a bundle
# is invisible to the top-level check above, and this is what sees it.
payload_files="$(find "$PAYLOAD" -type f -not -name .DS_Store \
                      -not -path "$PAYLOAD/README.txt" | wc -l | tr -d ' ')"
staged_files="$(find "$STAGE/root" -type f | wc -l | tr -d ' ')"
# Three files are staged that did not come from the payload -- uninstall.sh, the .command beside it,
# and the README written for an installed build -- and they are named rather than allowed for as a
# slack of three. The point of this assertion is that nothing reaches a package without somebody
# accounting for it, so the number moves when the list does.
#
# **The payload's own README.txt is subtracted on the other side**, because `excluded()` leaves it
# behind and `dist_installed_readme` supplies the replacement. Left in, the two would cancel and the
# arithmetic would still balance -- at 2, by accident, one file dropped against one added -- which is
# exactly the kind of agreement this check exists not to accept.
#
# **`$CONSOLE_FILES` is the second source's contribution, and it is added rather than tolerated.**
# The console halves of the three bundled products are staged from `bin-console/`, so they are files
# in `$STAGE/root` that `$PAYLOAD` never held. Counted by the pass that stages them, which keeps this
# assertion saying what it always said -- every file is accounted for by somebody -- rather than
# gaining a slack the next omission could hide in.
GENERATED=3
if [ "$staged_files" -ne "$((payload_files + CONSOLE_FILES + GENERATED))" ]; then
  echo "installer: $payload_files payload file(s) + $CONSOLE_FILES from bin-console/ to stage," >&2
  echo "           but $staged_files staged (expected" >&2
  echo "           $((payload_files + CONSOLE_FILES + GENERATED)); the extra $GENERATED are" >&2
  echo "           uninstall.sh, Uninstall KaraokeMachine.command and the installed README;" >&2
  echo "           the payload's own README.txt is excluded and not counted)." >&2
  exit 1
fi

# -- the scripts -----------------------------------------------------------------------------------------
#
# **The symlinks are made by a script and not archived in the payload**, and that is not a style
# choice. Archiving `bin/km-pack -> ../karaokemachine/km-pack` under --install-location /usr/local
# would put /usr/local/bin itself in the package's bill of materials, mode and owner included -- and
# Installer applies BOM directory entries to directories that already exist. On an Intel Mac that
# directory is Homebrew's and belongs to the user, so a `brew` install would quietly stop working
# after ours. A script that only ever creates named entries cannot do that.

cat > "$STAGE/scripts/tools/postinstall" <<'POST'
#!/bin/sh
set -eu
# mkdir -p, and never chown or chmod: see the note in installer.sh. This creates the directory only
# when it is missing and touches nothing else about it.
[ -d /usr/local/bin ] || { mkdir -p /usr/local/bin; chmod 755 /usr/local/bin; }
# Symlinks rather than shims, and that is verified rather than assumed: dyld resolves
# @executable_path against the *realpath* of the executable, not against the symlink, so
# /usr/local/bin/km-pack finds /usr/local/karaokemachine/lib. None of them reads anything
# relative to its own path, which is the other half of why a symlink is enough here and is not
# enough for the machine.
for t in km-pack km-lyrics km-wallpaper-pack km-package-builder km-package-simple km-remote km-admin; do
  ln -sfn "/usr/local/karaokemachine/$t" "/usr/local/bin/$t"
done
exit 0
POST

cat > "$STAGE/scripts/machine/postinstall" <<'POST'
#!/bin/sh
set -eu
[ -d /usr/local/bin ] || { mkdir -p /usr/local/bin; chmod 755 /usr/local/bin; }
# **A shim and not a symlink, and the difference is not style.** Rust's `current_exe()` on Apple is
# `_NSGetExecutablePath` with no realpath, so through a symlink `Paths::discover_asset_dir` sees
# /usr/local/bin, takes neither the sibling-assets branch nor the Contents/MacOS one, and falls back
# to $PWD/assets -- the machine comes up on a sine test tone over a plain gradient with nothing on
# screen saying why. `exec` makes the kernel record the real path, so the ../Resources/assets branch
# is taken and the instrument bank is found.
cat > /usr/local/bin/karaokemachine <<'SHIM'
#!/bin/sh
# Installed by the KaraokeMachine setup package. Removed by
# /usr/local/karaokemachine/uninstall.sh, which recognizes this line.
exec "/Applications/Karaoke Machine.app/Contents/MacOS/karaokemachine" "$@"
SHIM
chmod 755 /usr/local/bin/karaokemachine

# **The bundles under their one-word names are this component, and an upgrade takes them.** Installer
# places the new names and leaves the old ones standing, so /Applications would hold two machines
# sharing one identifier, and LaunchServices would open whichever it found first. Taken only when the
# identifier inside says it is ours, which is the test uninstall.sh applies for the same reason.
for old in "/Applications/KaraokeMachine.app" "/Applications/KaraokeMachine Stream.app"; do
  [ -d "$old" ] || continue
  id=$(plutil -extract CFBundleIdentifier raw -o - "$old/Contents/Info.plist" 2>/dev/null) || continue
  case "$id" in com.karaokemachine.*) rm -rf "$old" ;; esac
done

# **What makes an installed machine drive a television**, and it rides with the machine's own
# component rather than the SoundFont's: that one is a tick box somebody chose, and this is what
# kind of install this is. Unticking a bank download must not leave the television in a window.
#
# `set +e` from here down, and `exit 0` regardless. distribution.xml sets require-scripts="true", so
# a nonzero exit turns "we could not write down that this is a television" into "the karaoke machine
# would not install", which is the wrong answer by a wide margin. The shim above is worth failing an
# install over -- a machine with no /usr/local/bin entry is broken -- and this is not.
set +e

# Who is actually sitting here; the same question the SoundFont script asks, and the same answer for
# an install driven over SSH, where root is on the console and no home directory is honestly meant.
user="$(stat -f %Su /dev/console 2>/dev/null || true)"
if [ -n "$user" ] && [ "$user" != "root" ]; then
  home="$(dscl . -read "/Users/$user" NFSHomeDirectory 2>/dev/null | awk '{print $2}')"
  if [ -n "$home" ] && [ -d "$home" ]; then
    dir="$home/Library/Application Support/karaokemachine"
    if [ ! -d "$dir" ]; then
      mkdir -p "$dir" && chown "$user" "$dir" 2>/dev/null
    fi
    # **Never over an existing one.** That is the whole of the safety argument: this writes only
    # where it is creating the install, so it is never the thing firstrun.rs refuses -- an installer
    # overwriting a settings file it did not create, losing whatever an upgrade was standing on.
    if [ -d "$dir" ] && [ ! -f "$dir/settings.json" ]; then
      cp "$(dirname "$0")/settings.json" "$dir/settings.json" 2>/dev/null
      chown "$user" "$dir/settings.json" 2>/dev/null
    fi
  fi
fi
exit 0
POST

# Two keys, and that is a whole settings file: every settings struct is `#[serde(default)]`, so the
# machine fills the rest from its own defaults and writes it back complete on the first start.
# `display.fullscreen` defaults to *off* in the binary because the other thing with no settings file
# is a checkout somebody has just built -- see `The setup programs pre-write a settings file` in
# docs/decisions/distribution.md. It rides in the scripts folder for the reason the SoundFont request
# does: pkgbuild extracts that folder whole, so the postinstall finds it beside itself.
printf '{\n  "display": {\n    "fullscreen": true\n  }\n}\n' > "$STAGE/scripts/machine/settings.json"
grep -q '"fullscreen": true' "$STAGE/scripts/machine/settings.json" \
  || { echo "installer: the first-start settings file does not ask for fullscreen" >&2; exit 1; }
dist_detail "settings   fullscreen: true, where there is no settings.json yet"

cat > "$STAGE/scripts/builder/postinstall" <<'POST'
#!/bin/sh
# **Delegated, never duplicated** -- the same rule the .iss's [Run] follows. register.rs owns the
# lsregister nudge, and the file type itself is declared by the bundle's own Info.plist, which
# LaunchServices reads when the .app is placed. So this is a nudge and not the mechanism, and a
# failure is a message rather than a failed install: the declaration is picked up on first launch
# either way.
"/Applications/KM Package Builder.app/Contents/MacOS/km-package-builder" --register \
  >/dev/null 2>&1 || true
exit 0
POST

# -- the bank the tick box offers ------------------------------------------------------------------
#
# **This package downloads nothing, and that sentence is the whole design.** `What setup fetches` in
# docs/decisions/distribution.md says the macOS package "fetches nothing, ever", and it still does
# not: the recommended bank is 261.9 MiB, which is eight times the shipped tree, so what the tick box
# leaves behind is a *request*. `crates/machine/karaokemachine/src/firstrun.rs` reads it on the
# machine's first start and fetches the bank there, with a line on the television saying how far it
# has got and two more starts to try again on.
#
# Read out of the table through the shell reader that already exists rather than written here, for
# the reason the Windows driver gives at the same point: a hand-kept copy of a size or a license is
# the drift that arrangement prevents, and this copy would be visible on a pane somebody reads before
# agreeing to it.
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

# The concrete id and not `recommended`: the bank somebody was offered by name and size on the choice
# pane is the bank that should arrive, even if the table's recommendation moves between this release
# and their first start. It rides in the scripts folder, which is where a --nopayload component can
# carry a file at all -- pkgbuild extracts that folder whole, so the postinstall finds it beside
# itself.
printf '{\n  "bank": "%s"\n}\n' "$BANK_ID" > "$STAGE/scripts/soundfont/first-run-soundfont.json"
grep -q "\"$BANK_ID\"" "$STAGE/scripts/soundfont/first-run-soundfont.json" \
  || { echo "installer: the SoundFont request does not name $BANK_ID" >&2; exit 1; }
dist_detail "soundfont  $BANK_ID -> $BANK_NAME ($BANK_SIZE)"

# **Runs as root, and the request belongs to a person.** Every other postinstall here writes under
# /usr/local, which root owns; this one writes into a home directory, so it has to find out whose.
# The console user is the one sitting in front of the machine that is being installed on, which is
# who ticked the box.
#
# **Nothing in it may fail the install.** distribution.xml sets require-scripts="true", so a nonzero
# exit here would turn "we could not write down which instrument bank you wanted" into "the karaoke
# machine would not install" -- which is the wrong answer by a wide margin. Every step falls out to
# `exit 0`, the same tolerance the builder's `--register` nudge has and for the same reason.
cat > "$STAGE/scripts/soundfont/postinstall" <<'POST'
#!/bin/sh
set -u

# Who is actually sitting here. An install driven over SSH by `installer -pkg` has root on the
# console, and there is then no home directory this could honestly mean -- so it writes nothing.
user="$(stat -f %Su /dev/console 2>/dev/null || true)"
[ -n "$user" ] && [ "$user" != "root" ] || exit 0

home="$(dscl . -read "/Users/$user" NFSHomeDirectory 2>/dev/null | awk '{print $2}')"
[ -n "$home" ] && [ -d "$home" ] || exit 0

# Where `directories::ProjectDirs` puts config_dir on macOS, which is also data_dir and is where
# settings.json lives. No `config` subdirectory here, unlike Windows.
dir="$home/Library/Application Support/karaokemachine"
if [ ! -d "$dir" ]; then
  mkdir -p "$dir" || exit 0
  # **Handed to them, not left owned by root.** A fresh install has no such folder yet, and one made
  # by this script would otherwise be a folder the machine cannot write settings.json into.
  chown "$user" "$dir" 2>/dev/null || true
fi

# Never over an existing one: a reinstall must not reset an attempt count the machine has been
# writing into it, and must not re-ask for a bank somebody has since declined by deleting this.
[ -f "$dir/first-run-soundfont.json" ] && exit 0

cp "$(dirname "$0")/first-run-soundfont.json" "$dir/first-run-soundfont.json" 2>/dev/null || exit 0
chown "$user" "$dir/first-run-soundfont.json" 2>/dev/null || true
exit 0
POST

chmod 755 "$STAGE"/scripts/*/postinstall
# Parsed before they are packaged, the same check `uninstall.sh` already gets above. A postinstall
# that will not parse fails the *install* under require-scripts="true", on somebody else's Mac, with
# nothing on screen naming the line -- and these are heredocs, which is exactly where a stray
# unbalanced quote goes unnoticed.
for script in "$STAGE"/scripts/*/postinstall; do sh -n "$script"; done

# -- the component packages ---------------------------------------------------------------------------

# `pkg_component_plist` in tools/platform/macos/pkg.sh forces BundleIsRelocatable off, which is the
# trap that otherwise upgrades a copy sitting in ~/Downloads and leaves /Applications empty with no
# error anywhere.

build_started=$SECONDS
for comp in "${COMPONENTS[@]}"; do
  args=(--root "$STAGE/root/$comp"
        --identifier "$PRODUCT.$comp"
        --version "$VERSION"
        --install-location "$(install_location "$comp")"
        --ownership recommended)
  case "$comp" in
    machine|builder|simple|remote|admin)
      pkg_component_plist "$STAGE/root/$comp" "$STAGE/plists/$comp.plist"
      args+=(--component-plist "$STAGE/plists/$comp.plist") ;;
  esac
  if [ -n "$(ls -A "$STAGE/scripts/$comp")" ]; then
    args+=(--scripts "$STAGE/scripts/$comp")
  fi
  dist_run "pkgbuild $comp" pkgbuild "${args[@]}" "$STAGE/pkgs/karaokemachine-$comp.pkg"
done

# The payload-free ones. `--nopayload` takes neither a root nor an install-location: there is nothing
# to place, so a bill of materials would be empty and an install-location would be a claim about a
# directory this package has no business describing.
for comp in "${SCRIPT_COMPONENTS[@]}"; do
  dist_run "pkgbuild $comp" pkgbuild \
    --nopayload \
    --identifier "$PRODUCT.$comp" \
    --version "$VERSION" \
    --scripts "$STAGE/scripts/$comp" \
    "$STAGE/pkgs/karaokemachine-$comp.pkg"
done

# -- the product archive --------------------------------------------------------------------------------

# The three bank fields are substituted here rather than written into the template for the reason the
# tick box's own section gives. They are not XML-escaped: the values come from a table in this
# repository rather than from anywhere a stranger can reach, and `xmllint` below is what would catch
# an `&` or a `<` finding its way into one -- at build time, loudly, rather than as a pane that will
# not render.
sed -e "s/@VERSION@/$VERSION/g" -e "s/@ARCHS@/$ARCHS/g" \
    -e "s/@BANK_NAME@/$BANK_NAME/g" -e "s/@BANK_SIZE@/$BANK_SIZE/g" \
    -e "s/@BANK_LICENSE@/$BANK_LICENSE/g" \
    "$RES/distribution.xml" > "$STAGE/distribution.xml"
xmllint --noout "$STAGE/distribution.xml"

# The two panes, rendered by tools/platform/macos/pkg.sh: which Gatekeeper snippet a build's state
# calls for, the `@SIGNING@` and `@DATA_LOCATIONS@` fills, and the doctype assertion that keeps
# Installer from showing a pane as raw markup. The templates and the snippets stay here, because each
# is a description of one product and one description with conditionals in it is how a pane comes to
# say something untrue.
snippet="$(pkg_signing_snippet "$SIGNING_STATE")"
pkg_render_readme "$RES/readme.html.in" "${snippet:+$RES/$snippet}" "$STAGE/resources/readme.html"
pkg_render_conclusion "$RES/conclusion.html.in" "$RES/data-locations.txt" "$STAGE/resources/conclusion.html"
pkg_assert_doctypes "$STAGE/resources"

OUTDIR="$(dist_dir setup macos)"
OUTBASE="karaokemachine-setup-$VERSION-macos-$TARGET$SIGNING_MARKER"
mkdir -p "$OUTDIR"
rm -f "$OUTDIR/$OUTBASE.pkg"

productbuild_args=(--distribution "$STAGE/distribution.xml"
                   --package-path "$STAGE/pkgs"
                   --resources "$STAGE/resources")
if dist_signing; then
  productbuild_args+=(--sign "$KM_SIGN_INSTALLER_IDENTITY" --timestamp)
fi

# **Not through `dist_run` when signing, and that is a real trap rather than a style choice.**
# `dist_run` buffers output and replays it only on failure, and a locked keychain makes productbuild
# block on an unlock prompt -- so the build would sit there with nothing on screen and look hung.
# Signing runs in the open; an unsigned build keeps the quiet path it always had.
if dist_signing; then
  productbuild "${productbuild_args[@]}" "$OUTDIR/$OUTBASE.pkg"
else
  dist_run "productbuild" productbuild "${productbuild_args[@]}" "$OUTDIR/$OUTBASE.pkg"
fi

printf '   built in %s\n' "$(dist_elapsed "$build_started")"

PKG="$OUTDIR/$OUTBASE.pkg"
if [ ! -f "$PKG" ]; then
  echo "installer: $PKG was not produced" >&2
  exit 1
fi

# -- report ------------------------------------------------------------------------------------------

# **Measured on the staging root, not on `$PAYLOAD`.** What this line calls the payload is what
# went into the package, and since the console halves come from a second folder, the size of the
# first one stopped being that number. The staging root is what pkgbuild was handed.
payload_bytes="$(dist_bytes "$STAGE/root")"
pkg_bytes="$(wc -c < "$PKG" | tr -d ' ')"
dylibs="$(find "$PAYLOAD" -name 'libav*.dylib' -o -name 'libsw*.dylib' | wc -l | tr -d ' ')"

echo
printf 'built %s\n' "$PKG"
printf '  karaoke machine %s\n' "$VERSION"
# **Read out of the two arrays, never typed.** A hand-written `machine, builder, remote, tools`
# stays behind every component added after it, so the summary under-reports a package that is in
# fact correct. The same drift as the `.app` count one folder over, which is why that one is
# derived too. `docs` is left out because
# the parenthetical is what it is, and the payload-free array is appended because a component that
# installs no files is still a tick somebody sees.
listed=()
for comp in "${COMPONENTS[@]}"; do
  if [ "$comp" != docs ]; then listed+=("$comp"); fi
done
listed+=("${SCRIPT_COMPONENTS[@]}")
printf '  components: %s (+ the readmes and the uninstaller)\n' \
  "$(printf '%s, ' "${listed[@]}" | sed 's/, $//')"
printf '  applications -> /Applications;  commands -> /usr/local/bin\n'
printf '  video yes  (%s dylib(s) across the payload)\n' "$dylibs"
printf '  %s bytes (~%s MiB), from a %s MiB payload\n' \
  "$pkg_bytes" "$((pkg_bytes / 1024 / 1024))" "$((payload_bytes / 1024 / 1024))"
printf '  signing   %s\n' "$(dist_signing_note)"
if dist_signing; then
  if [ "$NOTARIZE" -eq 1 ]; then
    printf '  installer %s\n' "$KM_SIGN_INSTALLER_IDENTITY"
    printf '  notarized and stapled -- a recipient double-clicks it and Installer opens\n'
  else
    printf '  installer %s\n' "$KM_SIGN_INSTALLER_IDENTITY"
    printf '  not notarized -- a downloaded copy is still refused; add --notarize\n'
  fi
else
  printf '  a recipient right-clicks it and picks Open\n'
fi
for s in ${SKIPPED[@]+"${SKIPPED[@]}"}; do
  printf '  skipped %s\n' "$s"
done

# -- notarization ----------------------------------------------------------------------------------------
#
# **One submission covers everything.** notarytool inspects nested code, so submitting the .pkg
# notarizes every bundle and every dylib inside it; only the .pkg is stapled, because it is the
# thing that gets downloaded and therefore the thing Gatekeeper is asked about.
#
# Not through `dist_run`, for the reason productbuild is not: this can prompt, and it can take
# minutes, and a progress-free wait that turns out to be a keychain dialog is the worst way to spend
# them.
if [ "$NOTARIZE" -eq 1 ]; then
  pkg_notarize "$PKG"
fi

# -- the round trip -------------------------------------------------------------------------------------
#
# **Checked rather than asserted**, which is this repository's rule for a carrier -- but the Windows
# script's trick of installing silently into a scratch directory has no macOS equivalent: a
# system-domain package installs to `/` and needs root. So the default trip *expands* the archive and
# inspects what would land, which needs no password and still catches the two failures that matter:
# a file no component installs, and a component whose libraries were selected out from under it.
# `--install` below is the real thing, for somebody willing to type a password.

echo
dist_step "round trip"

SCRATCH="$(mktemp -d)"
cleanup() { rm -rf "$STAGE" "$SCRATCH"; }
trap cleanup EXIT

fail() { echo "installer: $*" >&2; exit 1; }

# `pkg_expand` in tools/platform/macos/pkg.sh probes the undocumented `--expand-full` and falls back
# to the documented `--expand` plus a cpio extraction, producing the same shape either way.
EXPANDED="$SCRATCH/expanded"
pkg_expand "$PKG" "$EXPANDED"

payload_of() { printf '%s/karaokemachine-%s.pkg/Payload' "$EXPANDED" "$1"; }

# 1. Every component archived exactly what was staged for it.
for comp in "${COMPONENTS[@]}"; do
  p="$(payload_of "$comp")"
  [ -d "$p" ] || fail "the $comp component has no extracted payload at $p"
  a="$(cd "$STAGE/root/$comp" && find . -type f | sort)"
  b="$(cd "$p" && find . -type f | sort)"
  [ "$a" = "$b" ] || fail "the $comp payload does not match what was staged for it"
done

# 2. PackageInfo says what pkgbuild was asked for, rather than what this script believes it asked.
for comp in "${COMPONENTS[@]}"; do
  info="$EXPANDED/karaokemachine-$comp.pkg/PackageInfo"
  grep -q "identifier=\"$PRODUCT.$comp\"" "$info" || fail "$comp has the wrong identifier"
  grep -q "install-location=\"$(install_location "$comp")\"" "$info" \
    || fail "$comp has the wrong install-location"
done
# 2b. The payload-free component is exactly that: a script, an identifier, and no files.
#
#     Worth asserting rather than assuming, because the failure is silent in both directions. A
#     `--nopayload` package that somehow acquired a bill of materials would install files nobody
#     accounted for; one that lost its postinstall would be a tick box that does nothing, and the
#     person who ticked it would find out weeks later that their machine is on the bundled bank.
for comp in "${SCRIPT_COMPONENTS[@]}"; do
  info="$EXPANDED/karaokemachine-$comp.pkg/PackageInfo"
  [ -f "$info" ] || fail "the $comp component package was not built"
  grep -q "identifier=\"$PRODUCT.$comp\"" "$info" || fail "$comp has the wrong identifier"
  if [ -e "$EXPANDED/karaokemachine-$comp.pkg/Payload" ]; then
    fail "$comp carries a payload; it is meant to install no files at all"
  fi
  # Only under `--expand-full`: the fallback above leaves Scripts as an archive rather than a
  # directory, and asserting on a shape that depends on which extractor ran would fail on the
  # machines the fallback exists for.
  scripts="$EXPANDED/karaokemachine-$comp.pkg/Scripts"
  if [ -d "$scripts" ]; then
    [ -x "$scripts/postinstall" ] || fail "$comp has no postinstall, so ticking it would do nothing"
  fi
done

# **The relocation check, and the shape of it is worth knowing.** `<relocate/>` is present in every
# PackageInfo -- it is the *list* of bundles that may move, and an empty one is what we want -- so
# grepping for the word finds it on a correct package too. The attribute is the answer, and the empty
# element is the corroboration: with relocation on, `<relocate>` carries a `<bundle id=.../>`.
for comp in machine builder simple remote admin; do
  info="$EXPANDED/karaokemachine-$comp.pkg/PackageInfo"
  grep -q 'relocatable="false"' "$info" \
    || fail "$comp is still relocatable; BundleIsRelocatable did not take, so an .app somebody
             unzipped into ~/Downloads would be upgraded there and /Applications left empty"
  grep -q '<relocate/>' "$info" \
    || fail "$comp names a bundle inside <relocate>, so it can still be placed somewhere else"
  grep -q '<upgrade-bundle>' "$info" \
    || fail "$comp does not upgrade its bundle wholesale; a file from an older version could
             survive inside the .app and break its signature"
done

# 3. Modes survived the extraction. `dist_verify_macho_portable` finds Mach-Os with `-perm -u+x`, so
#    an extraction that lost the execute bit would make it examine almost nothing and pass vacuously.
machos="$(find "$EXPANDED" -type f \( -name '*.dylib' -o -perm -u+x \) | wc -l | tr -d ' ')"
[ "$machos" -ge 14 ] \
  || fail "only $machos Mach-O candidate(s) in the extraction -- modes were not preserved, so the
           portability check below would have examined almost nothing"

# 4. Nothing loads by absolute path, and every signature verifies.
dist_verify_macho_portable "$EXPANDED" || fail "the extracted payload is not portable"
for app in "$EXPANDED"/*/Payload/*.app; do
  [ -d "$app" ] || continue
  codesign --verify --deep --strict "$app" || fail "$(basename "$app") does not verify after packaging"
done

# 5. Every executable starts, out of the extracted payload, with nothing helping it. The macOS
#    counterpart of the Windows bare-PATH run: what is being proved is that every library resolved
#    from inside the tree, so it is the DYLD_* variables that are stripped rather than PATH, and it
#    is the exit status that is the proof.
run() { env -u DYLD_LIBRARY_PATH -u DYLD_FALLBACK_LIBRARY_PATH -u DYLD_INSERT_LIBRARIES \
            -u DYLD_FRAMEWORK_PATH "$@"; }
tools_payload="$(payload_of tools)"

# **The commands this package puts on the PATH, named once for the three checks below.**
# The postinstall above spells them again and has to: it is a quoted heredoc that runs on somebody
# else's machine, where this array does not exist. Everywhere *here* shares one list, so the two
# counts in the closing summary are read off it rather than typed. A typed count is a number three
# separate places have to remember when a command is added, and they do not.
TOOL_COMMANDS=(km-pack km-lyrics km-wallpaper-pack km-package-builder km-package-simple km-remote km-admin)
for t in "${TOOL_COMMANDS[@]}"; do
  [ -f "$tools_payload/$t" ] || fail "$t was not packaged"
  ( cd "$tools_payload" && run "./$t" --version >/dev/null 2>&1 ) \
    || fail "$t would not start from the packaged folder with DYLD_* stripped"
done
#    **The bundles, as a table for the reason `TOOL_COMMANDS` is a list.** One line per bundle
#    leaves the newest bundle without one: staged, signed and verified, and the one application in
#    the package nobody ever starts. A *command* of the same name being covered makes that thin
#    coverage rather than none -- and thin coverage is what the next bundle gets too, a set of
#    hand-written lines being easier to leave alone than to extend. A row here is the whole of
#    adding one.
#
#    The machine is asked `--show-paths` where the others get `--version`, which is why the flag is
#    part of the row: it is the one whose interesting failure is not starting but starting and then
#    looking in the wrong place for its bank.
BUNDLE_STARTS=(
  "machine|Karaoke Machine.app/Contents/MacOS/karaokemachine|--show-paths|machine would not answer --show-paths"
  # **This row proves the launcher reaches the bundle beside it**, which is the only thing that can
  # be wrong about it and is invisible from the outside: its executable is a script naming a relative
  # path, so a bundle renamed or a layout changed leaves a launcher that starts nothing and says
  # nothing. `--show-paths` returns before anything is opened, so this asks the question without
  # starting an encoder, and it arrives after the `--stream` the script supplies.
  "machine|KM Stream.app/Contents/MacOS/karaokemachine-stream|--show-paths|streaming launcher would not reach the machine beside it"
  "builder|KM Package Builder.app/Contents/MacOS/km-package-builder|--version|package builder would not start"
  "simple|KM Simple Package.app/Contents/MacOS/km-package-simple|--version|simple package builder would not start"
  "remote|KM Remote.app/Contents/MacOS/km-remote|--version|remote would not start"
  "admin|KM Admin.app/Contents/MacOS/km-admin|--version|admin tool would not start"
)
for record in "${BUNDLE_STARTS[@]}"; do
  IFS='|' read -r bcomp bexe bflag bwhy <<<"$record"
  [ -x "$(payload_of "$bcomp")/$bexe" ] || fail "the $bcomp component carries no $bexe"
  ( cd "$(payload_of "$bcomp")" && run "./$bexe" "$bflag" >/dev/null 2>&1 ) \
    || fail "the packaged $bwhy"
done

#    **The bundle *names*, taken off that table rather than typed again.** They were typed again --
#    in the two loops of the `--install` round trip at the end of this file -- and `KM Admin.app`
#    reached neither. It was added to `BUNDLE_STARTS` when the fourth bundle arrived and to nothing
#    else, so a real install never checked that it lands in /Applications, verifies, arrives
#    unquarantined, or is taken away again. Derived, for the reason `claim()` derives what it takes
#    from `bin-console/`: a fifth bundle is a row in one table, and everything that counts bundles
#    already has it.
APP_BUNDLES=()
for record in "${BUNDLE_STARTS[@]}"; do
  IFS='|' read -r _ bexe _ _ <<<"$record"
  APP_BUNDLES+=("${bexe%%.app/*}")
done

# 6. The component wiring saves what it claims to. Without this the ticks could quietly stop meaning
#    anything and nothing would say so -- the Windows script makes the same argument with its
#    remote-only install.
find "$(payload_of machine)" -name '*.sf2' | grep -q . \
  || fail "no SoundFont in the machine component -- it would come up on a test tone"
find "$tools_payload" -name '*.sf2' | grep -q . && fail "the tools carry the SoundFont"
find "$tools_payload" -name '*.app' | grep -q . && fail "the tools carry an application bundle"
[ -d "$tools_payload/lib" ] || fail "the tools carry no lib/, so nothing that needs ffmpeg would run"
find "$(payload_of remote)" -name 'libav*' | grep -q . && fail "the remote carries ffmpeg"
find "$(payload_of remote)" -name '*.sf2' | grep -q . && fail "the remote carries the SoundFont"
[ -f "$(payload_of docs)/uninstall.sh" ] || fail "the uninstaller was not packaged"

# **The README that was packaged is the one written for an installed build**, and not the folder
# document from the payload. Asserted for the reason the Windows round trip asserts the same thing:
# nothing ever read this file, which is how a package came to carry -- into the same folder as the
# uninstaller checked on the line above -- a document saying to remove the product by deleting the
# folder because nothing was installed anywhere else. Both halves are checked, so putting the
# payload's copy back into claim() fails here rather than in somebody's hands.
DOCS_README="$(payload_of docs)/README.txt"
[ -f "$DOCS_README" ] || fail "no README.txt in the docs component"
grep -qi 'nothing was registered' "$DOCS_README" \
  && fail "the packaged README is the folder document, not dist_installed_readme's"
grep -q 'Uninstall KaraokeMachine.command' "$DOCS_README" \
  || fail "the packaged README does not say how to remove an installed build"
grep -q 'Library/Application Support/karaokemachine' "$DOCS_README" \
  || fail "the packaged README does not say where songs and settings are kept"
# The `--install` block below asserts that what this package *places* carries no quarantine flag, so
# the installed README must not tell the reader to clear one. The per-product README-*.txt beside it
# still do, for the zipped folders they were written for; this document disowns them in as many
# words, and this is what keeps it doing so.
grep -q 'xattr -dr com.apple.quarantine' "$DOCS_README" \
  && fail "the installed README tells the reader to clear a quarantine flag; files written out of an installer payload carry none"

# The double-clickable wrapper, which is the half of the removal story an end user is meant to find.
# Checked harder than the .sh because nothing else exercises it: the Finder is what normally runs it,
# and there is no Finder here.
COMMAND="$(payload_of docs)/Uninstall KaraokeMachine.command"
[ -f "$COMMAND" ] || fail "Uninstall KaraokeMachine.command was not packaged"
[ -x "$COMMAND" ] || fail "Uninstall KaraokeMachine.command is not executable, so a double-click
                           would open it in an editor rather than run it"
sh -n "$COMMAND" || fail "Uninstall KaraokeMachine.command is not valid sh"

# **Run it with no terminal on stdin and assert it removes nothing.** This is the whole of the
# wrapper's own logic under test -- it finds its folder, finds uninstall.sh beside it, shows the dry
# run, notices there is nobody to ask, and stops. No password is needed and nothing on this Mac is
# touched, because the path being exercised is the one that stops before sudo.
#
# **The second grep is the load-bearing one**, which is not obvious and was learned by deleting the
# tty check to see this fail: without it the `read` gets EOF, the answer is empty, and the
# defaults-to-no branch prints "Nothing was removed" anyway -- so the first assertion passes on a
# broken wrapper. Two layers doing the same job is the right shape here; an assertion that cannot
# tell them apart is not.
( cd "$(payload_of docs)" && ./"Uninstall KaraokeMachine.command" < /dev/null ) > "$SCRATCH/cmd.out" 2>&1 \
  || fail "Uninstall KaraokeMachine.command exited non-zero with no terminal"
grep -q 'Nothing was removed' "$SCRATCH/cmd.out" \
  || fail "the .command did not stop when there was nobody to ask; it must never take silence for
           an answer"
grep -q 'sudo /usr/local/karaokemachine/uninstall.sh' "$SCRATCH/cmd.out" \
  || fail "the .command did not name the direct command for a caller that cannot be asked"

# 6b. **What signing bought, asserted rather than assumed.** `dist_verify_macho_portable` above has
#     already checked every Mach-O in the extraction for the team identifier and the hardened
#     runtime, since it knows the identity too -- so what is left here is the archive itself, which
#     carries a different signature from a different certificate and is the one Gatekeeper reads.
if dist_signing; then
  pkgutil --check-signature "$PKG" | grep -q "Developer ID Installer" \
    || fail "the .pkg is not signed by a Developer ID Installer certificate, so a recipient sees the
             same dialog as an unsigned build and the signing bought nothing"
  if [ "$NOTARIZE" -eq 1 ]; then
    xcrun stapler validate "$PKG" >/dev/null 2>&1 \
      || fail "the notarization ticket is not stapled to the .pkg, so a recipient with no network
               would still be refused"
    # Gatekeeper's own answer to "would this open?", which is the only check that speaks for the
    # thing a person actually does with the file.
    spctl -a -vvv -t install "$PKG" >/dev/null 2>&1 \
      || fail "spctl refuses the .pkg, so a recipient's Mac would too"
  fi
fi

# 7. The Distribution parses, and every choice it offers has a package behind it.
xmllint --noout "$EXPANDED/Distribution"
for id in $(sed -n 's/.*<pkg-ref id="\([^"]*\)".*/\1/p' "$EXPANDED/Distribution" | sort -u); do
  [ -d "$EXPANDED/karaokemachine-${id##*.}.pkg" ] || fail "$id has no component package"
done

# 8. **The installed shape, built in a scratch directory and run.** Everything above tests the
#    package; this tests the *layout the postinstall scripts make*, which is the half a
#    file-by-file check cannot reach and the half where both of this milestone's real surprises
#    live. It needs no password, because what /usr/local/bin does is not privileged -- only where
#    it is.
SIM="$SCRATCH/usr-local"
mkdir -p "$SIM/karaokemachine" "$SIM/bin"
ditto "$tools_payload" "$SIM/karaokemachine"
for t in "${TOOL_COMMANDS[@]}"; do
  ln -sfn "../karaokemachine/$t" "$SIM/bin/$t"
done

# Symlinks are enough for all of them, and that is a measurement: dyld resolves `@executable_path`
# against the *realpath* of the main executable rather than against the symlink, so `$SIM/bin/km-pack`
# finds `$SIM/karaokemachine/lib`. Running them from a directory that is neither is what proves it.
for t in "${TOOL_COMMANDS[@]}"; do
  ( cd "$SCRATCH" && PATH=/usr/bin:/bin run "$SIM/bin/$t" --version >/dev/null 2>&1 ) \
    || fail "/usr/local/bin/$t would not run through a symlink with a bare PATH"
done

# **And symlinks are not enough for the machine**, which is why its postinstall writes a shim. Both
# halves are asserted, because an assertion that only checks the shim would go on passing the day
# somebody simplifies it into a symlink. Rust's `current_exe()` on Apple is `_NSGetExecutablePath`
# with no realpath, so through a symlink `discover_asset_dir` sees /usr/local/bin, matches neither
# the sibling-assets branch nor the Contents/MacOS one, and falls back to $PWD/assets -- the machine
# comes up on a sine test tone with nothing on screen saying why.
sim_app="$(payload_of machine)/Karaoke Machine.app"
ln -sfn "$sim_app/Contents/MacOS/karaokemachine" "$SIM/bin/karaokemachine-symlink"
printf '#!/bin/sh\nexec "%s/Contents/MacOS/karaokemachine" "$@"\n' "$sim_app" > "$SIM/bin/karaokemachine"
chmod 755 "$SIM/bin/karaokemachine"

# **Captured, never piped into `grep -q`, and that is load-bearing rather than tidiness.** `grep -q`
# exits the instant it matches, which closes the pipe under a writer that is still writing. Rust
# ignores SIGPIPE, so `println!` gets EPIPE and *panics*: the machine exits 101 and, with
# `set -o pipefail`, the pipeline reports that failure rather than grep's success. Both assertions
# below then said the opposite of the truth -- the first failed a release whose bundle was perfectly
# good, and the second, which fails when a match is found, could no longer see one at all. The bug
# needs three lines of `--show-paths` output after the first match to bite, so it arrived with the
# `soundfont` line rather than with either assertion.
shim_paths="$( cd "$SCRATCH" && PATH=/usr/bin:/bin run "$SIM/bin/karaokemachine" --show-paths 2>/dev/null )"
case "$shim_paths" in
  *"Karaoke Machine.app/Contents/Resources/assets"*) ;;
  *) fail "the machine's shim did not find the assets inside the bundle" ;;
esac
symlink_paths="$( cd "$SCRATCH" && PATH=/usr/bin:/bin run "$SIM/bin/karaokemachine-symlink" --show-paths 2>/dev/null || true )"
case "$symlink_paths" in
  *"Karaoke Machine.app/Contents/Resources/assets"*)
    fail "a symlink now finds the bundle's assets too, so the shim in the machine's postinstall has
        stopped being necessary -- read discover_asset_dir and simplify it rather than leaving a
        shim nobody can justify" ;;
esac

echo "verified: every component archives what was staged, nothing loads by absolute path,"
printf '          all %s executables start with DYLD_* stripped, the bundles still verify,\n' \
  "$(( ${#TOOL_COMMANDS[@]} + ${#BUNDLE_STARTS[@]} ))"
echo "          the tools carry neither the SoundFont nor an application bundle, the"
printf '          /usr/local layout runs -- %s commands through symlinks, the machine through\n' \
  "${#TOOL_COMMANDS[@]}"
echo "          its shim, which finds the instrument bank where a symlink does not -- and the"
echo "          removal wrapper runs, shows what would go, and stops when there is nobody to ask."
if dist_signing; then
  printf '          Every Mach-O carries %s and the hardened runtime, and the\n' "$(dist_team_id)"
  if [ "$NOTARIZE" -eq 1 ]; then
    echo "          archive is signed, notarized, stapled and accepted by spctl."
  else
    echo "          archive is signed by a Developer ID Installer certificate."
  fi
fi

# -- the real thing, on request ---------------------------------------------------------------------------

if [ "$INSTALL" -eq 1 ]; then
  echo
  dist_step "installing it, for real, on this Mac"
  echo "   This installs into /Applications and /usr/local and then removes it again."
  echo "   Ctrl-C now if that is not what you want."
  sudo -v

  before_data=0
  [ -d "$HOME/Library/Application Support/karaokemachine" ] && before_data=1

  sudo installer -pkg "$PKG" -target / >/dev/null

  for app in "${APP_BUNDLES[@]}"; do
    [ -d "/Applications/$app.app" ] || fail "/Applications/$app.app was not installed"
    codesign --verify --deep --strict "/Applications/$app.app" \
      || fail "$app.app does not verify once installed"
    # The one genuine advantage this carrier has over the zips: Installer writes files out of a
    # payload, and the quarantine flag is put on the thing that was downloaded rather than inherited
    # by what it contains. So these open with no `xattr -dr` first. Asserted, because the readme pane
    # says it.
    xattr "/Applications/$app.app" 2>/dev/null | grep -q com.apple.quarantine \
      && fail "$app.app was installed quarantined; the readme pane's claim is wrong"
  done

  for c in karaokemachine km-pack km-lyrics km-wallpaper-pack \
           km-package-builder km-package-simple km-remote km-admin; do
    [ -e "/usr/local/bin/$c" ] || fail "/usr/local/bin/$c was not created"
    ( PATH=/usr/bin:/bin "/usr/local/bin/$c" --version >/dev/null 2>&1 ) \
      || fail "/usr/local/bin/$c would not start"
  done
  # Through the shim, which is the whole reason it is a shim rather than a symlink.
  # Captured rather than piped, for the EPIPE-panic reason spelled out at the shim check above.
  installed_paths="$( PATH=/usr/bin:/bin /usr/local/bin/karaokemachine --show-paths )"
  case "$installed_paths" in
    *"Karaoke Machine.app"*) ;;
    *) fail "the machine's shim did not find its assets inside the bundle" ;;
  esac

  receipts="$(pkgutil --pkgs | grep -c "^$PRODUCT\." || true)"
  [ "$receipts" = "${#COMPONENTS[@]}" ] \
    || fail "$receipts receipt(s) recorded; expected ${#COMPONENTS[@]}"

  sudo /usr/local/karaokemachine/uninstall.sh

  for app in "${APP_BUNDLES[@]}"; do
    [ -d "/Applications/$app.app" ] && fail "the uninstaller left /Applications/$app.app behind"
  done
  [ -d /usr/local/karaokemachine ] && fail "the uninstaller left /usr/local/karaokemachine behind"
  for c in karaokemachine km-pack km-lyrics km-wallpaper-pack \
           km-package-builder km-package-simple km-remote km-admin; do
    [ -e "/usr/local/bin/$c" ] && fail "the uninstaller left /usr/local/bin/$c behind"
  done
  pkgutil --pkgs | grep -q "^$PRODUCT\." && fail "the uninstaller left receipts behind"

  # The one promise the uninstaller makes.
  if [ "$before_data" -eq 1 ]; then
    [ -d "$HOME/Library/Application Support/karaokemachine" ] \
      || fail "the uninstaller removed your data directory, which it promises not to"
  fi

  echo "verified: installs to /Applications and /usr/local, runs from a bare PATH, is not"
  echo "          quarantined, uninstalls clean, and leaves your data alone."
fi

echo
if dist_signing && [ "$NOTARIZE" -eq 1 ]; then
  echo "now:  open \"$PKG\""
else
  echo "now:  open \"$PKG\"      # not notarized: right-click it in the Finder and pick Open"
fi
