#!/usr/bin/env bash
#
# Builds the macOS setup program for the remote alone: one .pkg carrying KM Remote and nothing else.
#
#   tools/platform/macos/installer-remote.sh              # stage the remote, build, check it
#   tools/platform/macos/installer-remote.sh --notarize   # ...signed and notarized, the one to hand over
#   tools/platform/macos/installer-remote.sh --no-build   # build from what is already staged
#   tools/platform/macos/installer-remote.sh --install    # ...and do the real sudo install/uninstall
#   tools/platform/macos/installer-remote.sh -v           # watch the staging and the build
#
#   -> dist/setup/macos/km-remote-setup-<version>-macos-<arch>.pkg              (--notarize)
#      ...-macos-<arch>-unnotarized.pkg   signed only, which spctl still refuses
#      ...-macos-<arch>-unsigned.pkg      ad-hoc, the default
#
# **It gathers; it does not build.** The payload is `dist/km-remote/macos/`, which tools/dist/cmd.sh
# stages -- so every fact about how a macOS load command is rewritten and what the README says stays
# there. The all-in-one gathers `dist/bin/macos` and `dist/bin-console/macos` instead, which is every
# product and a far longer run for a carrier that installs one of them.
#
# **Two sources rather than one, because that is the shape `tools/dist/cmd.sh` stages.** The folder
# `km-remote-<version>-<triple>` holds the terminal form and the papers; `KM Remote.app` sits *beside*
# it, because a product's platform folder holds exactly one bundle -- its own -- and
# `dist_stage_macos_bundle` takes any second one as the fossil of a rename.
#
# **The windowed application alone.** `KM Remote.app` goes to /Applications and the bare `km-remote`
# beside it in the staged folder is deliberately left out: nothing here reaches /usr/local/bin, so
# there is no command to type and no postinstall to write one. What does go to /usr/local/km-remote is
# the two licence texts, a README for an installed build, and the uninstaller -- because a .pkg has no
# Add/Remove Programs to register with and no folder somebody unpacked, so the one way off has to be
# a file it places. See the `A setup program for the remote alone` decision in docs/decisions/.
#
# **System domain, exactly as the all-in-one is.** A bundle in /Applications is machine-wide by
# convention, and one administrator prompt is what every macOS installer costs.
#
# Prerequisite: nothing. pkgbuild, productbuild and pkgutil are in the base system.

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/dist/common.sh
. tools/platform/macos/pkg.sh
DIST_SCRIPT=installer-remote

RES=tools/platform/macos/pkg-remote          # the Distribution, the two panes, the shared text
SNIPPETS=tools/platform/macos/pkg            # the two Gatekeeper snippets, shared unchanged
UNINSTALL_TEMPLATE=tools/platform/macos/uninstall-remote.sh
UNINSTALL_COMMAND=tools/platform/macos/uninstall-remote.command
PLIST=tools/platform/macos/Info.remote.plist

# **`com.rrgmc.km-remote` and never `com.rrgmc.karaokemachine.remote`.** Sharing the
# all-in-one's namespace would make Installer treat one install as an upgrade of the other, and its
# uninstaller forgets receipts by that prefix -- it would take this package's receipt with its own.
PRODUCT=com.rrgmc.km-remote

# Two, because `pkgbuild` takes one --root and one --install-location and /Applications cannot share
# a package with /usr/local/km-remote. The count is forced rather than chosen.
COMPONENTS=(app docs)

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
      echo "usage: tools/platform/macos/installer-remote.sh [--no-build] [--notarize] [--install] [-v]"
      exit 0 ;;
    *) echo "installer-remote: unknown option $arg" >&2; exit 2 ;;
  esac
done

if [ "$(dist_platform)" != "macos" ]; then
  echo "installer-remote: this builds a macOS package and has to run on macOS." >&2
  echo "                  The Windows one is tools/platform/windows/installer-remote.sh; Linux has" >&2
  echo "                  none, because the remote travels in the .deb and the tarball." >&2
  exit 1
fi

pkg_require_tools
pkg_resolve_signing "$NOTARIZE"

install_location() { # <component>
  case "$1" in
    app)  printf '/Applications' ;;
    docs) printf '/usr/local/km-remote' ;;
  esac
}

# -- the payload ---------------------------------------------------------------------------------------

if [ "$BUILD" -eq 1 ]; then
  dist_step "staging the remote"
  staging_started=$SECONDS
  args=()
  if dist_verbose; then args=(-v); fi
  dist_run "dist-cmd.sh" tools/dist/cmd.sh "${args[@]+"${args[@]}"}" km-remote
  printf '   staged in %s\n' "$(dist_elapsed "$staging_started")"
fi

# **Found rather than named, and exactly one.** `dist_staged_dir` takes the version, and the version
# comes out of the bundle inside the folder, so the folder has to be resolved first. Two matches is an
# error rather than a choice -- the shape `one_match` in tools/dist/release.sh uses.
#
# **The plain name only.** tools/dist/cmd.sh marks a *declined* build, so `-no-desktop` is a remote
# with no window, and the window is the whole of what this carrier installs.
PARENT="$(dist_dir km-remote macos)"
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

# **Beside the folder, never inside it.** `dist_stage_macos_bundle` writes the bundle into the
# product's platform folder, one level above the versioned one, and its absence is what a
# `--no-desktop` staging looks like here -- the same refusal the Windows driver makes by looking for
# the console twin, which is a file macOS has no equivalent of.
APP="$PARENT/KM Remote.app"
if [ ! -d "$APP" ]; then
  echo "installer-remote: there is no KM Remote.app beside $(basename "$PAYLOAD"), so the remote was" >&2
  echo "                  staged without a window. A setup program installing a remote that cannot" >&2
  echo "                  open one is not what it promises. Run tools/dist/cmd.sh km-remote without" >&2
  echo "                  --no-desktop." >&2
  exit 1
fi

# **A nested `.DS_Store` is refused outright, where a top-level one is merely skipped.** `pkgbuild`
# drops every one of them at any depth by its own default filter, so one inside the bundle would be
# staged, dropped, and surface as a payload mismatch rather than as its cause. It does not break the
# seal -- `.DS_Store` is in codesign's own default exclusions -- so the mismatch is the whole of the
# reason. Asked of the payload rather than the staging root, so the message names a path somebody can
# go and delete.
stray="$(find "$PAYLOAD" "$APP" -name .DS_Store -not -path "$PAYLOAD/.DS_Store" || true)"
if [ -n "$stray" ]; then
  echo "installer-remote: a .DS_Store is sitting where pkgbuild will drop it:" >&2
  printf '  %s\n' "$stray" >&2
  echo "                  pkgbuild drops it silently and the payload check then fails for a" >&2
  echo "                  reason that does not name it. Remove it and run this again." >&2
  exit 1
fi

APP_EXE="$APP/Contents/MacOS/km-remote"
VERSION="$(dist_version "$APP_EXE")"

TARGET="$(dist_host_triple)"; TARGET="${TARGET%%-*}"      # aarch64 | x86_64

# Asked of the artifact rather than of a flag. An arm64 build says arm64; an x86_64 build says both,
# because Rosetta will run it and refusing an Apple silicon Mac a build it can execute would be wrong.
case "$(lipo -archs "$APP_EXE")" in
  *arm64*) ARCHS="arm64" ;;
  *)       ARCHS="x86_64,arm64" ;;
esac

# The minimum is stated in two places here -- the remote's manifest and this Distribution -- and this
# is what notices when they stop agreeing. **`plutil` rather than `sed`, because a missing key has to
# be a failure**: a pattern match over a manifest that does not declare the key returns an empty
# string, and an empty string reads as agreement.
dist_min="$(sed -n 's/.*<os-version min="\([^"]*\)".*/\1/p' "$RES/distribution.xml")"
if ! plist_min="$(plutil -extract LSMinimumSystemVersion raw -o - "$PLIST" 2>/dev/null)"; then
  echo "installer-remote: $PLIST declares no LSMinimumSystemVersion." >&2
  exit 1
fi
if [ "$plist_min" != "$dist_min" ]; then
  echo "installer-remote: $PLIST says LSMinimumSystemVersion $plist_min and" >&2
  echo "                  $RES/distribution.xml says os-version min $dist_min. They have drifted." >&2
  exit 1
fi

dist_step "macos setup package -- the remote alone"
dist_detail "payload   $PAYLOAD"
dist_detail "version   $VERSION"
dist_detail "arch      $TARGET (hostArchitectures $ARCHS)"
dist_detail "min os    $dist_min"

# -- staging -----------------------------------------------------------------------------------------

STAGE="$(mktemp -d)"
SCRATCH="$(mktemp -d)"
cleanup() { rm -rf "$STAGE" "$SCRATCH"; }
trap cleanup EXIT

mkdir -p "$STAGE/pkgs" "$STAGE/plists" "$STAGE/resources"
for comp in "${COMPONENTS[@]}"; do mkdir -p "$STAGE/root/$comp"; done

# The whole of the app component, taken from beside the folder rather than out of it.
ditto "$APP" "$STAGE/root/app/KM Remote.app"

# **Every entry in the folder is claimed or the build stops**, which is the rule the all-in-one's
# `claim()` enforces with a five-way table and this one needs a one-way one for: the two licence
# texts are all it takes from there. The bare `km-remote` is the named exclusion -- it is the terminal
# form, and this carrier installs the windowed program alone. `README.txt` is the second: it is the
# folder document, and `dist_installed_readme` writes the one an installed build gets. `.DS_Store` is
# the Finder's, written the moment anybody opens the staged folder in a window, and a staged folder is
# not cleared between builds.
SKIPPED=()
# `dotglob`, or the `.DS_Store` arm below never fires and the file-count reconciliation then fails
# with arithmetic instead of with the name of the file. `nullglob` so an empty folder is not a loop
# over the literal pattern.
shopt -s nullglob dotglob
for entry in "$PAYLOAD"/*; do
  base="$(basename "$entry")"
  case "$base" in
    LICENSE-MIT.txt|LICENSE-APACHE.txt)
      ditto "$entry" "$STAGE/root/docs/$base" ;;
    km-remote)
      SKIPPED+=("$base -- the terminal form; this package installs the application alone") ;;
    README.txt)
      SKIPPED+=("$base -- the folder document; dist_installed_readme writes the installed one") ;;
    .DS_Store)
      SKIPPED+=("$base -- the Finder's own folder metadata, not a build artifact") ;;
    *)
      echo "installer-remote: $PAYLOAD holds $base, which nothing here claims." >&2
      echo "                  Add it to the case above with the component that needs it, or name" >&2
      echo "                  the exclusion. Nothing is dropped from a carrier by accident." >&2
      exit 1 ;;
  esac
done
shopt -u nullglob dotglob

# The uninstaller and its double-clickable wrapper, with the substitutions counted. They ride with the
# READMEs deliberately: the one way to take this off again must not be something a tick could decline.
pkg_fill_uninstaller "$UNINSTALL_TEMPLATE" "$RES/data-locations.txt" \
                     "$VERSION" "$STAGE/root/docs/uninstall.sh"
chmod 755 "$STAGE/root/docs/uninstall.sh"
sh -n "$STAGE/root/docs/uninstall.sh"

pkg_fill_command "$UNINSTALL_COMMAND" "$VERSION" "$STAGE/root/docs/Uninstall KM Remote.command"
chmod 755 "$STAGE/root/docs/Uninstall KM Remote.command"
sh -n "$STAGE/root/docs/Uninstall KM Remote.command"

# The README an installed build gets, in place of the folder document the loop above left behind.
dist_installed_readme macos km-remote > "$STAGE/root/docs/README.txt"

# A component whose case arm matched nothing produces a smaller package and no error, which is the
# failure this notices.
for comp in "${COMPONENTS[@]}"; do
  if [ -z "$(ls -A "$STAGE/root/$comp")" ]; then
    echo "installer-remote: the $comp component staged nothing." >&2
    exit 1
  fi
done

# **The strong half of the reconciliation**: a `ditto` that silently dropped something *inside* the
# bundle is invisible to the loop above, and this is what sees it. Three files are staged that did not
# come from the payload -- the uninstaller, the .command beside it and the installed README -- and
# they are named rather than allowed for as a slack of three, so the number moves when the list does.
# The skipped entries are subtracted on the other side for the same reason.
payload_files="$(find "$PAYLOAD" -type f | wc -l | tr -d ' ')"
app_files="$(find "$APP" -type f | wc -l | tr -d ' ')"
staged_files="$(find "$STAGE/root" -type f | wc -l | tr -d ' ')"
GENERATED=3
expected=$((payload_files - ${#SKIPPED[@]} + app_files + GENERATED))
if [ "$staged_files" -ne "$expected" ]; then
  echo "installer-remote: $payload_files folder file(s) less ${#SKIPPED[@]} skipped, plus the" >&2
  echo "                  $app_files in the bundle and the $GENERATED generated ones, is" >&2
  echo "                  $expected -- but $staged_files staged." >&2
  exit 1
fi

# -- the component packages ---------------------------------------------------------------------------

build_started=$SECONDS
for comp in "${COMPONENTS[@]}"; do
  args=(--root "$STAGE/root/$comp"
        --identifier "$PRODUCT.$comp"
        --version "$VERSION"
        --install-location "$(install_location "$comp")"
        --ownership recommended)
  if [ "$comp" = app ]; then
    pkg_component_plist "$STAGE/root/$comp" "$STAGE/plists/$comp.plist"
    args+=(--component-plist "$STAGE/plists/$comp.plist")
  fi
  dist_run "pkgbuild $comp" pkgbuild "${args[@]}" "$STAGE/pkgs/km-remote-$comp.pkg"
done

# -- the product archive --------------------------------------------------------------------------------

sed -e "s/@VERSION@/$VERSION/g" -e "s/@ARCHS@/$ARCHS/g" \
    "$RES/distribution.xml" > "$STAGE/distribution.xml"
xmllint --noout "$STAGE/distribution.xml"

# The two panes. The Gatekeeper snippet comes from the all-in-one's resource folder unchanged: that
# obstacle is a property of the archive rather than of the product, so the two carriers have one copy
# of the text between them.
snippet="$(pkg_signing_snippet "$SIGNING_STATE")"
pkg_render_readme "$RES/readme.html.in" "${snippet:+$SNIPPETS/$snippet}" "$STAGE/resources/readme.html"
pkg_render_conclusion "$RES/conclusion.html.in" "$RES/data-locations.txt" "$STAGE/resources/conclusion.html"
pkg_assert_doctypes "$STAGE/resources"

OUTDIR="$(dist_dir setup macos)"
OUTBASE="km-remote-setup-$VERSION-macos-$TARGET$SIGNING_MARKER"
mkdir -p "$OUTDIR"
rm -f "$OUTDIR/$OUTBASE.pkg"

productbuild_args=(--distribution "$STAGE/distribution.xml"
                   --package-path "$STAGE/pkgs"
                   --resources "$STAGE/resources")
if dist_signing; then
  productbuild_args+=(--sign "$KM_SIGN_INSTALLER_IDENTITY" --timestamp)
fi

# **Not through `dist_run` when signing.** `dist_run` buffers output and replays it only on failure,
# and a locked keychain makes productbuild block on an unlock prompt -- so the build would sit there
# with nothing on screen and look hung. An unsigned build keeps the quiet path.
if dist_signing; then
  productbuild "${productbuild_args[@]}" "$OUTDIR/$OUTBASE.pkg"
else
  dist_run "productbuild" productbuild "${productbuild_args[@]}" "$OUTDIR/$OUTBASE.pkg"
fi

printf '   built in %s\n' "$(dist_elapsed "$build_started")"

PKG="$OUTDIR/$OUTBASE.pkg"
if [ ! -f "$PKG" ]; then
  echo "installer-remote: $PKG was not produced" >&2
  exit 1
fi

# -- report ------------------------------------------------------------------------------------------

payload_bytes="$(dist_bytes "$STAGE/root")"
pkg_bytes="$(wc -c < "$PKG" | tr -d ' ')"

echo
printf 'built %s\n' "$PKG"
printf '  km remote %s\n' "$VERSION"
printf '  the application alone: /Applications, nothing on the PATH, no ffmpeg\n'
printf '  the papers and the uninstaller -> /usr/local/km-remote\n'
printf '  %s bytes (~%s MiB), from a %s MiB payload\n' \
  "$pkg_bytes" "$((pkg_bytes / 1024 / 1024))" "$((payload_bytes / 1024 / 1024))"
printf '  signing   %s\n' "$(dist_signing_note)"
if dist_signing; then
  printf '  installer %s\n' "$KM_SIGN_INSTALLER_IDENTITY"
  if [ "$NOTARIZE" -eq 1 ]; then
    printf '  notarized and stapled -- a recipient double-clicks it and Installer opens\n'
  else
    printf '  not notarized -- a downloaded copy is still refused; add --notarize\n'
  fi
else
  printf '  a recipient right-clicks it and picks Open\n'
fi
for s in ${SKIPPED[@]+"${SKIPPED[@]}"}; do
  printf '  skipped %s\n' "$s"
done

if [ "$NOTARIZE" -eq 1 ]; then
  pkg_notarize "$PKG"
fi

# -- the round trip -------------------------------------------------------------------------------------
#
# **Checked rather than asserted**, which is this repository's rule for a carrier -- but a
# system-domain package installs to `/` and needs root, so the default trip *expands* the archive and
# inspects what would land. That needs no password and still catches the failure that matters: a file
# no component installs. `--install` below is the real thing, for somebody willing to type a password.

echo
dist_step "round trip"

fail() { echo "installer-remote: $*" >&2; exit 1; }

EXPANDED="$SCRATCH/expanded"
pkg_expand "$PKG" "$EXPANDED"

payload_of() { printf '%s/km-remote-%s.pkg/Payload' "$EXPANDED" "$1"; }

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
  info="$EXPANDED/km-remote-$comp.pkg/PackageInfo"
  grep -q "identifier=\"$PRODUCT.$comp\"" "$info" || fail "$comp has the wrong identifier"
  grep -q "install-location=\"$(install_location "$comp")\"" "$info" \
    || fail "$comp has the wrong install-location"
done

# 3. **The coexistence check in its cheap static form.** Nothing in this archive may claim the
# all-in-one's namespace: its uninstaller forgets receipts by that prefix, so a component here that
# borrowed one would have its receipt taken by a package that never wrote it.
if grep -rq 'com\.rrgmc\.karaokemachine' "$EXPANDED"/*/PackageInfo; then
  fail "a component claims the karaoke machine's receipt namespace"
fi

# 4. **`BundleIsRelocatable` is off**, which is the trap that otherwise upgrades a copy sitting in
# ~/Downloads and leaves /Applications empty with no error anywhere. The check is not the obvious
# grep: `<relocate/>` is present in *every* PackageInfo -- it is the *list* of bundles that may move,
# and an empty one is the good case -- so grepping for the word passes on a correct package and fails
# on one. `relocatable="false"` is the answer.
grep -q 'relocatable="false"' "$EXPANDED/km-remote-app.pkg/PackageInfo" \
  || fail "the app component is relocatable; Installer would place it wherever an older copy sits"

# 5. Modes survived the extraction, and every Mach-O in it is portable and hardened.
dist_verify_macho_portable "$EXPANDED" || fail "the extracted payload is not portable"

# 6. **The negatives that define this carrier.** Each is a promise the report above makes out loud.
APP_PAYLOAD="$(payload_of app)"
DOCS_PAYLOAD="$(payload_of docs)"
find "$APP_PAYLOAD" -name 'libav*' -o -name 'libsw*' | grep -q . \
  && fail "an ffmpeg library is in the payload; the remote links none"
find "$EXPANDED" -name '*.sf2' | grep -q . \
  && fail "an instrument bank is in the payload; the remote plays nothing"
[ -e "$DOCS_PAYLOAD/km-remote" ] && fail "the terminal form was archived; this installs the application alone"
[ -d "$APP_PAYLOAD/KM Remote.app" ] || fail "KM Remote.app is not in the app component"
for other in KaraokeMachine "KM Package Builder" "KM Admin"; do
  [ -d "$APP_PAYLOAD/$other.app" ] && fail "$other.app was archived by the remote's own package"
done

# 7. The papers, the uninstaller, and the README proved to be the one meant for an installed build.
[ -f "$DOCS_PAYLOAD/LICENSE-MIT.txt" ]    || fail "LICENSE-MIT.txt was not archived"
[ -f "$DOCS_PAYLOAD/LICENSE-APACHE.txt" ] || fail "LICENSE-APACHE.txt was not archived"
[ -x "$DOCS_PAYLOAD/uninstall.sh" ]       || fail "uninstall.sh was not archived, or is not executable"
[ -x "$DOCS_PAYLOAD/Uninstall KM Remote.command" ] \
  || fail "the .command wrapper was not archived, or is not executable"
grep -qi 'Add or remove programs' "$DOCS_PAYLOAD/README.txt" \
  && fail "the archived README is the Windows one"
grep -q '/usr/local/km-remote' "$DOCS_PAYLOAD/README.txt" \
  || fail "the archived README does not say where the uninstaller is"

# 8. **The wrapper says what to type, and that is the load-bearing half.** Run with no terminal it
# refuses to remove anything, which is right -- and a refusal that did not name the direct command
# would leave somebody with no way forward.
out="$("$DOCS_PAYLOAD/Uninstall KM Remote.command" </dev/null 2>&1 || true)"
printf '%s' "$out" | grep -q 'Nothing was removed' \
  || fail "the .command removed something, or did not say it had not, without a terminal"
printf '%s' "$out" | grep -q 'sudo /usr/local/km-remote/uninstall.sh' \
  || fail "the .command does not name the command to type instead"

# 9. Every choice in the Distribution has a component package behind it.
for ref in $(sed -n 's/.*<pkg-ref id="\([^"]*\)" version.*/\1/p' "$STAGE/distribution.xml"); do
  case " ${COMPONENTS[*]} " in
    *" ${ref##*.} "*) ;;
    *) fail "the Distribution refers to $ref, which no component package provides" ;;
  esac
done

# 10. What signing bought, asserted rather than assumed.
if dist_signing; then
  pkgutil --check-signature "$PKG" | grep -q "Developer ID Installer" \
    || fail "the .pkg is not signed with a Developer ID Installer certificate"
  if [ "$NOTARIZE" -eq 1 ]; then
    xcrun stapler validate "$PKG" >/dev/null 2>&1 \
      || fail "the .pkg has no stapled ticket, so an offline Mac would refuse it"
    spctl -a -vvv -t install "$PKG" >/dev/null 2>&1 \
      || fail "spctl refuses the .pkg, so a recipient's Mac would too"
  fi
fi

echo "verified: two components with their own identifiers, the application in /Applications"
echo "          and not relocatable, no ffmpeg and no bank, no terminal form, the papers and"
echo "          an uninstaller that says what to type."

# -- the real thing, for somebody willing to type a password -------------------------------------------
#
# **The only way to test what actually lands**, and it is opt-in because it installs on this Mac for
# real. It ends by running the uninstaller, so what it leaves behind is nothing -- which is also the
# assertion.
if [ "$INSTALL" -eq 1 ]; then
  echo
  dist_step "installing for real (sudo)"
  sudo installer -pkg "$PKG" -target / >/dev/null

  [ -d "/Applications/KM Remote.app" ] || fail "KM Remote.app is not in /Applications"
  codesign --verify --deep --strict "/Applications/KM Remote.app" \
    || fail "the installed bundle does not verify"
  # **Nothing an installer places is quarantined**, which is what the readme pane promises: the
  # right-click is for the .pkg, once, and not for what comes out of it.
  xattr "/Applications/KM Remote.app" 2>/dev/null | grep -q com.apple.quarantine \
    && fail "KM Remote.app was installed quarantined; the readme pane's claim is wrong"
  # The negative that proves "the application alone".
  [ -e /usr/local/bin/km-remote ] && fail "a command was placed on the PATH"
  [ -x /usr/local/km-remote/uninstall.sh ] || fail "the uninstaller is not in /usr/local/km-remote"

  receipts="$(pkgutil --pkgs | grep -c "^$PRODUCT\." || true)"
  [ "$receipts" = "${#COMPONENTS[@]}" ] \
    || fail "expected ${#COMPONENTS[@]} receipts under $PRODUCT., found $receipts"

  echo
  dist_step "uninstalling again (sudo)"
  sudo /usr/local/km-remote/uninstall.sh
  [ -d "/Applications/KM Remote.app" ] && fail "the uninstaller left the application behind"
  [ -d /usr/local/km-remote ] && fail "the uninstaller left its own folder behind"
  [ "$(pkgutil --pkgs | grep -c "^$PRODUCT\." || true)" = "0" ] \
    || fail "the uninstaller left a receipt behind"

  echo "verified: installs, verifies unquarantined, puts nothing on the PATH, and its"
  echo "          uninstaller removes the application, the folder and both receipts."
fi

echo
echo "now:  open $PKG"
