#!/usr/bin/env bash
#
# What every macOS setup program in this repository knows: which certificates it needs and how to
# refuse a bad combination, the one pkgbuild flag that is silent when wrong, how a template is filled
# in, how a pane is rendered, and how a finished archive is taken apart again.
#
# Sourced, never run. `tools/dist/common.sh` has to be sourced first -- `dist_signing`,
# `dist_signing_note`, `dist_team_id`, `dist_detail` and `dist_run` are used throughout.
#
# **It exists because a second copy of any of this is silent when it drifts.** A signing resolution
# that has lost a refusal produces a package Gatekeeper rejects and a report that says it worked; a
# component plist that has lost `BundleIsRelocatable` places an application wherever an older copy of
# it happens to sit and leaves /Applications empty, with no error anywhere. Neither is the kind of
# fault a build notices.
#
# What is deliberately *not* here: the Distribution, the two panes, the data-locations block and the
# uninstaller templates. Each of those is a description of one product, and one description with
# conditionals in it is how a pane comes to say something untrue.

# -- the tools -------------------------------------------------------------------------------------
#
# Resolved before anything is staged, the same discipline `inno_require_compiler` follows on Windows:
# a missing tool should cost a second, not a three-minute staging run. These all ship with macOS, so
# this is really a check that somebody has not stripped the command line tools.
pkg_require_tools() {
  local tool
  for tool in pkgbuild productbuild pkgutil lipo xmllint; do
    if ! command -v "$tool" >/dev/null 2>&1; then
      echo "${DIST_SCRIPT:-installer}: $tool is not on PATH." >&2
      echo "           xcode-select --install" >&2
      exit 1
    fi
  done
}

# -- signing ---------------------------------------------------------------------------------------
#
# **Resolved before anything is staged**, for the reason above: a certificate that is not there
# should cost a second.
#
# Two variables because they are two certificates. `KM_SIGN_IDENTITY` is a Developer ID *Application*,
# which signs bundles and the Mach-Os inside them, and lives in tools/dist/common.sh because several
# call sites across three scripts share it. `KM_SIGN_INSTALLER_IDENTITY` is a Developer ID
# *Installer*, a different certificate type, and signs one thing: the product archive.
#
# **Three values in the open, because not one of them is a secret.** The two identity strings are
# certificate *common names*: `pkgutil --check-signature` prints them from any package signed with
# them and `codesign -dvvv` from every bundle inside it, so they are published by the artifact rather
# than describing the machine that built it. `KM_NOTARY_PROFILE` is only the *label* of a keychain
# profile -- the Apple ID and app-specific password behind it were stored once by
# `xcrun notarytool store-credentials` and never leave the data-protection keychain. That a name may
# be committed at all is argued under *Published identity* in the
# `What a committed file may say about the machine it was written on` decision.
#
# **Replace all three to sign as yourself.** `security find-identity -v` lists what this keychain
# actually holds, and a value that is not in it fails the check below in about a second, naming
# itself. An environment variable still wins over each, so somebody else overrides without editing a
# tracked file at all:
#
#     KM_SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" \
#       tools/platform/macos/installer.sh --notarize
#
KM_NOTARIZE_APPLICATION="Developer ID Application: Rangel Reale (5XV6P36QZR)"
KM_NOTARIZE_INSTALLER="Developer ID Installer: Rangel Reale (5XV6P36QZR)"
KM_NOTARIZE_PROFILE="karaokemachine"

pkg_identity_exists() { # <identity string>
  security find-identity -v 2>/dev/null | grep -qF "$1"
}

# Sets KM_SIGN_INSTALLER_IDENTITY, KM_NOTARY_PROFILE, SIGNING_STATE and SIGNING_MARKER, or exits.
#
# **Only under `--notarize` are the three defaulted, and that gate is what makes committing them
# safe.** `dist_signing` is `[ -n "$KM_SIGN_IDENTITY" ]`, so defaulting it unconditionally would make
# every build here sign -- and tools/dist/common.sh says why that must not happen: a fresh clone,
# somebody else's Mac and the CI runners have no certificates and must still be able to stage a build.
#
# **`export` on the first, because staging happens in another process.** The staging script runs
# through `dist_run`, and `dist_codesign` signs there. Assigned rather than exported, the payload
# would be signed ad-hoc inside a Developer ID wrapper -- the half-signed bundle notarization refuses
# without naming a file, and the round trip would not say so until the end of a whole staging run.
pkg_resolve_signing() { # <notarize: 0 or 1>
  local notarize="$1" me="${DIST_SCRIPT:-installer}"

  KM_SIGN_INSTALLER_IDENTITY="${KM_SIGN_INSTALLER_IDENTITY:-}"

  if [ "$notarize" -eq 1 ]; then
    export KM_SIGN_IDENTITY="${KM_SIGN_IDENTITY:-$KM_NOTARIZE_APPLICATION}"
    KM_SIGN_INSTALLER_IDENTITY="${KM_SIGN_INSTALLER_IDENTITY:-$KM_NOTARIZE_INSTALLER}"
    KM_NOTARY_PROFILE="${KM_NOTARY_PROFILE:-$KM_NOTARIZE_PROFILE}"
  fi

  if dist_signing; then
    if ! pkg_identity_exists "$KM_SIGN_IDENTITY"; then
      echo "$me: KM_SIGN_IDENTITY names an identity this keychain does not have:" >&2
      echo "             $KM_SIGN_IDENTITY" >&2
      echo "           security find-identity -v   lists what is there." >&2
      exit 1
    fi
    # **The half-signed build is refused rather than produced.** Signed bundles inside an unsigned
    # .pkg is the artifact nobody wants and the one that looks like it worked: Gatekeeper judges the
    # archive, so the recipient sees the same dialog as before and the signing effort bought nothing.
    if [ -z "$KM_SIGN_INSTALLER_IDENTITY" ]; then
      echo "$me: KM_SIGN_IDENTITY is set but KM_SIGN_INSTALLER_IDENTITY is not." >&2
      echo "           Signing the bundles and leaving the .pkg unsigned changes nothing a recipient" >&2
      echo "           sees, because Gatekeeper judges the archive rather than what is inside it." >&2
      echo "           A Developer ID Installer certificate is a different certificate type from the" >&2
      echo "           Application one and is free on the same membership -- make one at" >&2
      echo "           developer.apple.com under Certificates, IDs & Profiles, then set both." >&2
      exit 1
    fi
    if ! pkg_identity_exists "$KM_SIGN_INSTALLER_IDENTITY"; then
      echo "$me: KM_SIGN_INSTALLER_IDENTITY names an identity this keychain does not have:" >&2
      echo "             $KM_SIGN_INSTALLER_IDENTITY" >&2
      echo "           Note that it must be a Developer ID *Installer*, not an Application one." >&2
      exit 1
    fi
  elif [ -n "$KM_SIGN_INSTALLER_IDENTITY" ]; then
    echo "$me: KM_SIGN_INSTALLER_IDENTITY is set but KM_SIGN_IDENTITY is not." >&2
    echo "           That would sign the .pkg over ad-hoc bundles, which notarization refuses." >&2
    exit 1
  fi

  # Notarization is asked for by name and never implied by signing: it needs the network and takes
  # minutes, and a build loop should not pay that on every run.
  if [ "$notarize" -eq 1 ]; then
    if ! dist_signing; then
      echo "$me: --notarize needs a signed build; set KM_SIGN_IDENTITY and" >&2
      echo "           KM_SIGN_INSTALLER_IDENTITY. Apple will not notarize an ad-hoc one." >&2
      exit 1
    fi
    if [ -z "${KM_NOTARY_PROFILE:-}" ]; then
      echo "$me: --notarize needs KM_NOTARY_PROFILE, the name of a stored notarytool profile." >&2
      echo "           Create one once, so no password is ever in a script or an environment:" >&2
      echo "             xcrun notarytool store-credentials <name> \\" >&2
      echo "               --apple-id <your-apple-id> --team-id $(dist_team_id) --password <app-specific>" >&2
      exit 1
    fi
  fi

  dist_detail "signing   $(dist_signing_note)"

  # **Three states, not two**, and the `Signing a macOS release` decision is where that is argued: a
  # correctly signed archive with a valid chain was still `rejected` by `spctl` as
  # `Unnotarized Developer ID`, so signed-but-not-notarized is its own state rather than a
  # nearly-finished one.
  #
  # **One variable read in two places, because they must not disagree.** The Read Me pane and the
  # file name are both this answer, and computing each from its own `if` is how they drift: a package
  # could say one thing in Installer's window and another in its own name.
  if ! dist_signing; then
    SIGNING_STATE=unsigned
  elif [ "$notarize" -eq 0 ]; then
    SIGNING_STATE=unnotarized
  else
    SIGNING_STATE=notarized
  fi

  # **The marker goes on the declined build**, which is tools/dist/bin.sh's rule for `-no-video`: the
  # notarized package is the one anybody is handed, so it keeps the plain name, and a build that
  # declined signing says so in the one place that travels with the file. `sweep` in
  # tools/dist/clean.sh reads the version as the field after the app name, so a marked name is
  # matched exactly like a plain one.
  #
  # **They coexist deliberately.** The names differ, so an ad-hoc rebuild for a local test cannot
  # overwrite a notarized package that cost an Apple round trip.
  case "$SIGNING_STATE" in
    notarized) SIGNING_MARKER="" ;;
    *)         SIGNING_MARKER="-$SIGNING_STATE" ;;
  esac
}

# -- the pkgbuild trap that is silent when wrong -----------------------------------------------------
#
# **BundleIsRelocatable defaults to true, and it is the trap here.** With it on, Installer places the
# .app wherever an existing bundle with the same CFBundleIdentifier already is -- so somebody who
# unzipped the application into ~/Downloads gets that copy upgraded and /Applications stays empty,
# silently. The plist is derived from the root rather than committed, so a bundle that changes shape
# cannot leave a stale one behind.
#
# **Every bundle in the component is answered, not the first.** A component is one product, and a
# product may carry more than one bundle: the machine's streaming launcher stands beside the machine
# it starts. A relocatable second bundle is the same silent misplacement as a relocatable first one,
# and the count is whatever pkgbuild found, so a bundle added to a product is covered by being there.
pkg_component_plist() { # <root> <out>
  local root="$1" out="$2" n i
  pkgbuild --analyze --root "$root" "$out" >/dev/null
  n="$(plutil -convert json -o - "$out" | tr ',' '\n' | grep -c RootRelativeBundlePath || true)"
  if [ "$n" -lt 1 ]; then
    echo "${DIST_SCRIPT:-installer}: $root holds no bundle; expected at least one." >&2
    return 1
  fi
  i=0
  while [ "$i" -lt "$n" ]; do
    plutil -replace "$i.BundleIsRelocatable"    -bool   false   "$out"
    plutil -replace "$i.BundleIsVersionChecked" -bool   false   "$out"
    # `upgrade` replaces the bundle wholesale, deleting paths that are no longer in it. `update` would
    # leave a file from an older version inside the .app, which breaks `codesign --verify`.
    plutil -replace "$i.BundleOverwriteAction"  -string upgrade "$out"
    i=$((i + 1))
  done
}

# -- filling in a template ---------------------------------------------------------------------------
#
# **Each marker has to occur exactly once, and this is not defensive tidiness.** The substitution is a
# plain string replacement over the whole file, so a second mention -- in the header comment
# explaining what gets substituted, which is exactly where it happened -- drops ten lines of prose
# into the middle of a `#` comment. The result is still valid shell, so `sh -n` passes it, and the
# fault only shows when somebody runs the installed uninstaller and reads `They: command not found`.

# The uninstaller: a version and the block of text saying where the data is.
pkg_fill_uninstaller() { # <template> <data-locations file> <version> <out>
  python3 - "$1" "$2" "$3" "$4" "${DIST_SCRIPT:-installer}" <<'PY'
import sys
template, locations, version, out, me = sys.argv[1:6]
text = open(template, encoding="utf-8").read()
for marker in ("@VERSION@", "@DATA_LOCATIONS@"):
    n = text.count(marker)
    if n != 1:
        raise SystemExit(
            f"{me}: {template} mentions {marker} {n} time(s); it has to be exactly one.\n"
            f"           Every mention is substituted, including one inside a comment."
        )
text = text.replace("@VERSION@", version)
text = text.replace("@DATA_LOCATIONS@", open(locations, encoding="utf-8").read().rstrip("\n"))
open(out, "w", encoding="utf-8").write(text)
PY
}

# The double-clickable wrapper beside it, which carries a version and nothing else.
pkg_fill_command() { # <template> <version> <out>
  python3 - "$1" "$2" "$3" "${DIST_SCRIPT:-installer}" <<'PY'
import sys
template, version, out, me = sys.argv[1:5]
text = open(template, encoding="utf-8").read()
n = text.count("@VERSION@")
if n != 1:
    raise SystemExit(
        f"{me}: {template} mentions @VERSION@ {n} time(s); it has to be exactly one."
    )
open(out, "w", encoding="utf-8").write(text.replace("@VERSION@", version))
PY
}

# -- the two panes -------------------------------------------------------------------------------------

# The Read Me, with the Gatekeeper snippet this build's state calls for. An empty snippet path is the
# notarized build, which has nothing to say there: the blank line the placeholder sat on goes with it,
# so the pane does not open with a gap somebody would read as a rendering fault.
pkg_render_readme() { # <template> <snippet or empty> <out>
  python3 - "$1" "$2" "$3" "${DIST_SCRIPT:-installer}" <<'PY'
import sys
template, snippet, out, me = sys.argv[1:5]
text = open(template, encoding="utf-8").read()
n = text.count("@SIGNING@")
if n != 1:
    raise SystemExit(
        f"{me}: {template} mentions @SIGNING@ {n} time(s); it has to be exactly one."
    )
filling = open(snippet, encoding="utf-8").read().rstrip("\n") if snippet else None
text = text.replace("@SIGNING@\n\n", "") if filling is None else text.replace("@SIGNING@", filling)
open(out, "w", encoding="utf-8").write(text)
PY
}

# The closing pane, which names the folders this product keeps, escaped because it lands in HTML.
pkg_render_conclusion() { # <template> <data-locations file> <out>
  python3 - "$1" "$2" "$3" <<'PY'
import html, sys
template, locations, out = sys.argv[1:4]
text = open(template, encoding="utf-8").read()
text = text.replace("@DATA_LOCATIONS@",
                    html.escape(open(locations, encoding="utf-8").read().rstrip("\n")))
open(out, "w", encoding="utf-8").write(text)
PY
}

# Which snippet a state calls for. Beside the two renderers because the mapping is the pane's, not
# the product's: an unsigned archive needs a right-click, and so does a signed-but-not-notarized one,
# because a Developer ID alone has not been enough for a downloaded file since Catalina.
#
# **A notarized build substitutes nothing, and the empty pane is the decision.** It opens on a
# double-click like anything else a person installs, so a heading announcing that it is signed and
# notarized tells them only that what is about to happen is what they expected. A pane of reassurance
# is the pane people stop reading, and the two paragraphs that matter -- how to get past Gatekeeper,
# how to remove this later -- are the ones that then go unread with it.
pkg_signing_snippet() { # <signing state>
  case "$1" in
    unsigned)    printf 'readme-unsigned.html' ;;
    unnotarized) printf 'readme-signed.html' ;;
    *)           printf '' ;;
  esac
}

# **Each pane must begin with a doctype, and this is a real fault caught rather than a style rule.**
# Installer sniffs the file's data to decide whether it is HTML or plain text, and one starting with a
# comment fails that sniff -- the readme was shown as raw markup, authoring comment and all, and
# `mime-type="text/html"` in the Distribution does not rescue it. Reproducible outside Installer with
# the same Cocoa importer: `textutil -stdin -convert txt -stdout < readme.html` echoes the source
# without a doctype and the prose with one.
pkg_assert_doctypes() { # <resources dir>
  local pane
  for pane in readme conclusion; do
    if ! head -c 15 "$1/$pane.html" | grep -qi '^<!DOCTYPE html>'; then
      echo "${DIST_SCRIPT:-installer}: $1/$pane.html does not begin with <!DOCTYPE html>." >&2
      echo "           Installer would show it as raw markup rather than rendering it." >&2
      exit 1
    fi
  done
}

# -- taking a finished archive apart again -------------------------------------------------------------
#
# `--expand-full` extracts each component's Payload into a directory, which is what makes the round
# trip's checks possible. **It is undocumented** -- it is in neither `man pkgutil` nor `--help` -- so
# it is probed rather than trusted, with the documented `--expand` plus a cpio extraction behind it.
# The fallback produces the same shape, so nothing downstream knows which ran.
pkg_expand() { # <pkg> <dir>
  local pkg="$1" dir="$2" p
  if pkgutil --expand-full "$pkg" "$dir" >/dev/null 2>&1; then
    return 0
  fi
  dist_detail "pkgutil --expand-full declined; falling back to --expand + cpio"
  pkgutil --expand "$pkg" "$dir"
  for p in "$dir"/*.pkg; do
    [ -f "$p/Payload" ] || continue
    mkdir "$p/Payload.d"
    ( cd "$p/Payload.d" && gunzip -dc "../Payload" | cpio -idm --quiet )
    rm -f "$p/Payload"
    mv "$p/Payload.d" "$p/Payload"
  done
}

# -- notarization --------------------------------------------------------------------------------------
#
# **One submission covers everything.** notarytool inspects nested code, so submitting the .pkg
# notarizes every bundle and every dylib inside it; only the .pkg is stapled, because it is the thing
# that gets downloaded and therefore the thing Gatekeeper is asked about.
#
# Not through `dist_run`, for the reason productbuild is not: this can prompt, and it can take
# minutes, and a progress-free wait that turns out to be a keychain dialog is the worst way to spend
# them.
pkg_notarize() { # <pkg>
  local pkg="$1" started=$SECONDS
  echo
  dist_step "notarizing (this goes to Apple and takes minutes)"
  if ! xcrun notarytool submit "$pkg" --keychain-profile "$KM_NOTARY_PROFILE" --wait; then
    echo "${DIST_SCRIPT:-installer}: notarization was refused." >&2
    echo "           The submission log is the only thing that says which file and why:" >&2
    echo "             xcrun notarytool log <submission-id> --keychain-profile $KM_NOTARY_PROFILE" >&2
    echo "           The usual cause is a Mach-O without the hardened runtime, which" >&2
    echo "           dist_verify_macho_portable checks for before it gets this far." >&2
    exit 1
  fi
  # Stapling attaches the ticket to the file, so it validates on a Mac that is offline or behind a
  # firewall. Without it Gatekeeper has to ask Apple at open time, and a recipient with no network
  # gets the refusal this whole exercise is about.
  xcrun stapler staple "$pkg"
  printf '   notarized in %s\n' "$(dist_elapsed "$started")"
}
