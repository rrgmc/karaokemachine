#!/usr/bin/env bash
#
# What every Inno Setup driver in this repository knows: where the compiler is, how to read a script
# back out, and how to install and remove what was just built without leaving anything behind.
#
# Sourced, never run. `tools/dist/common.sh` has to be sourced first -- `dist_step`, `dist_detail`,
# `dist_run` and `host_path` are used throughout.
#
# **It exists because the round trip must not be copied.** A driver that installs a setup program on
# the developer's own account and then asserts things about it has one failure mode that outlives the
# build: a per-user install registers itself under `HKCU\...\CurrentVersion\Uninstall`, and only
# `unins000.exe` takes that entry back out -- so an assertion failing between the install and the
# uninstall leaves a row in Settings > Installed apps naming a directory that is no longer there, with
# an Uninstall button that cannot work. The cleanup below is what stops that, it is subtle, and a
# second copy of it would be the drift this repository writes single definitions to avoid.

# -- the compiler ----------------------------------------------------------------------------------
#
# Resolved before anything is built, so a missing install fails in a second with the line that fixes
# it rather than after a six-minute staging run. Same discipline `dist_ffmpeg_dir` applies to ffmpeg,
# and for the same reason: the fix is one command and nobody should have to go and find it.
#
# **Three places, and the per-user one is first for a reason.** `winget install JRSoftware.InnoSetup`
# installs into %LOCALAPPDATA%\Programs when it is run without elevation, which is the usual case.
# Looking only in Program Files finds nothing and reports it as "not installed", which is the wrong
# sentence entirely.
inno_find_compiler() {
  local candidate local_appdata
  if command -v ISCC.exe >/dev/null 2>&1; then command -v ISCC.exe; return 0; fi
  if command -v iscc >/dev/null 2>&1; then command -v iscc; return 0; fi
  # %LOCALAPPDATA% arrives as a Windows path, so pasting it in front of a POSIX one gives
  # `C:\Users\...\Local/Programs/...`. Test operators tolerate that and it is a poor thing to print
  # in an error message or hand to `cd`, so it is converted where Git Bash can do it.
  local_appdata="${LOCALAPPDATA:-}"
  if [ -n "$local_appdata" ] && command -v cygpath >/dev/null 2>&1; then
    local_appdata="$(cygpath -u "$local_appdata")"
  fi
  # Spelled out rather than read from %ProgramFiles(x86)%: a variable whose name contains parentheses
  # cannot be referenced by bash's parameter syntax at all, so the literal is the only honest form.
  for candidate in \
    "$local_appdata/Programs/Inno Setup 6/ISCC.exe" \
    "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" \
    "/c/Program Files/Inno Setup 6/ISCC.exe"
  do
    if [ -f "$candidate" ]; then printf '%s' "$candidate"; return 0; fi
  done
  return 1
}

# Sets ISCC, or exits. The banner's first line is `Inno Setup <major> Command-Line Compiler`, and the
# major is asserted rather than assumed: the scripts here use CreateDownloadPage and
# ArchitecturesAllowed=x64compatible, neither of which exists in 5.
inno_require_compiler() {
  local major
  if ! ISCC="$(inno_find_compiler)"; then
    echo "installer: Inno Setup 6 is not installed -- no ISCC.exe on PATH or in the usual places." >&2
    echo "           winget install JRSoftware.InnoSetup" >&2
    exit 1
  fi
  # `|| true` is load-bearing under `set -o pipefail`: ISCC with no arguments prints its banner and
  # usage and then exits non-zero, so without this the version probe takes the whole script down
  # before it has printed anything at all.
  major="$("$ISCC" 2>&1 | sed -n '1s/^Inno Setup \([0-9]*\).*/\1/p' || true)"
  if [ -z "$major" ] || [ "$major" -lt 6 ]; then
    echo "installer: $ISCC is not Inno Setup 6 or newer." >&2
    echo "           winget install JRSoftware.InnoSetup" >&2
    exit 1
  fi
  dist_step "inno setup"
  dist_detail "iscc  $ISCC (version $major)"
}

# -- reading a script back out ----------------------------------------------------------------------
#
# **Parsed rather than restated**, which is the whole bargain these drivers make: the `.iss` says what
# it installs, and bash asks it rather than keeping a second list that can stop agreeing. A list
# written out here is a list that goes stale the day a product is added -- which is not a
# hypothetical, and the failure is the bad kind: it passes.

# The `Source:` paths under `{#Payload}`, as POSIX-relative paths.
inno_payload_sources() { # <iss>
  sed -n 's/^ *Source: *"{#Payload}\\\([^"]*\)".*/\1/p' "$1" | tr '\\' '/'
}

# The [Components] names, one per line. A script with no components prints nothing, which is a fact
# about that script rather than an error -- a carrier with one thing in it has nothing to tick.
inno_components() { # <iss>
  sed -n '/^\[Components\]/,/^\[/ s/^ *Name: *"\([^"]*\)".*/\1/p' "$1"
}

# The AppId, braces and all. **Two setup programs must never share one**: Inno makes a second run an
# upgrade of the first when they match, so a shared id would mean installing one removes the other.
inno_app_id() { # <iss>
  sed -n 's/^AppId=\(.*\)$/\1/p' "$1" | head -n 1
}

# **What the .iss says it installs, against what is actually there.** Anything in the payload that no
# entry covers is a file that would be silently dropped from the installer, which is the one failure a
# carrier must not have -- tools/dist/bin.sh makes the same argument one level down.
#
# Every exclusion is named by the caller and therefore stated out loud. Nothing is dropped from a
# carrier by accident, and an exception that is not written down is exactly what this exists to catch.
inno_coverage_check() { # <iss> <payload> [payload-relative path to exclude...]
  local iss="$1" payload="$2" covered present spec match excluded unaccounted
  shift 2

  covered="$(mktemp)"
  present="$(mktemp)"

  while IFS= read -r spec; do
    [ -n "$spec" ] || continue
    case "$spec" in
      */\*)
        # A recursive entry -- `assets\*` with recursesubdirs. It covers the whole subtree.
        find "$payload/${spec%/\*}" -type f 2>/dev/null >> "$covered" || true
        ;;
      *)
        for match in "$payload"/$spec; do
          [ -e "$match" ] && printf '%s\n' "$match" >> "$covered"
        done
        ;;
    esac
  done < <(inno_payload_sources "$iss")

  sort -u "$covered" -o "$covered"

  # `*.WebView2` is subtracted here rather than by the caller because it is not a build artifact at
  # all: it is the WebView2 runtime's own user-data folder, a browser profile of cache, cookies and
  # logs, which the runtime writes beside any executable that opens a window without naming a cache
  # directory of its own. Shipping somebody else's browser profile would be absurd. Both windowed
  # programs name a per-user directory, so a current build produces none -- and a staged folder is not
  # cleared between builds, so one left by an older binary is still a file sitting in the payload.
  find "$payload" -type f -not -path '*.WebView2/*' | sort -u > "$present"

  for excluded in "$@"; do
    grep -Fxv "$payload/$excluded" "$present" > "$present.keep" || true
    mv "$present.keep" "$present"
  done

  unaccounted="$(comm -23 "$present" "$covered" || true)"
  rm -f "$covered" "$present" "$present.keep"

  if [ -n "$unaccounted" ]; then
    echo "installer: the staged folder holds files that $iss does not install:" >&2
    printf '%s\n' "$unaccounted" | sed "s|^$payload/|             |" >&2
    echo "           Add them to [Files], or name the exclusion in the driver. Nothing is" >&2
    echo "           dropped from a carrier by accident." >&2
    return 1
  fi
}

# The WebView2 profile folders the check above passed over, counted so that one turning up says so
# rather than being ignored in silence.
inno_skipped_profiles() { # <payload>
  find "$1" -maxdepth 1 -type d -name '*.WebView2' | wc -l | tr -d ' '
}

# -- the round trip --------------------------------------------------------------------------------
#
# **Tested rather than asserted**, which is this repository's rule for a carrier: install it, run what
# it installed, uninstall it, and look. It catches the two ways an installer of this shape goes wrong,
# neither of which the compiler can see: a file in the payload that no component actually installs,
# and a component whose libraries were selected out from under it.

# Silent, and with every task switched off: /TASKS="" is what keeps a verification run from editing
# this machine's PATH or claiming a file type.
#
# **The *files* are confined to the scratch directory and two other things are not.** `/DIR=`
# redirects `{app}` and nothing else, so a run still writes a Start Menu group under `{group}` and
# registers itself under `HKCU\...\CurrentVersion\Uninstall`, on the developer's own account both
# times. That is not worth avoiding -- skipping [Icons] would stop the round trip covering them at
# all. What it means is that the uninstaller is the only thing that undoes a run, which is why
# `inno_cleanup_scratch` runs it rather than merely deleting the tree.
inno_install_to() { # <setup> <dir> [components]
  local setup="$1" dir="$2" components="${3:-}" waited=0 args=()
  args=(/VERYSILENT /SP- /NORESTART /TASKS="")
  [ -n "$components" ] && args+=("/COMPONENTS=$components")
  args+=("/DIR=$(host_path "$dir")")
  MSYS2_ARG_CONV_EXCL='*' "$setup" "${args[@]}" || return 1
  # **Waited for rather than assumed finished.** Inno's Setup.exe extracts itself and can hand the
  # work to a second process, so the exit above is not reliably the end of the install. The
  # uninstaller is written last, which makes it the signal that everything else is already there.
  while [ ! -f "$dir/unins000.exe" ] && [ "$waited" -lt 120 ]; do
    sleep 1
    waited=$((waited + 1))
  done
}

# Polls, because Inno's uninstaller relaunches itself from a temp copy and returns immediately.
inno_uninstall_from() { # <dir>
  local dir="$1" waited=0
  MSYS2_ARG_CONV_EXCL='*' "$dir/unins000.exe" /VERYSILENT /NORESTART || true
  while [ -d "$dir" ] && [ "$waited" -lt 60 ]; do
    sleep 1
    waited=$((waited + 1))
  done
}

# **The uninstaller runs first, and that is not tidiness.** See the header: deleting the folder leaves
# the registration behind, and `unins000.exe` was inside the folder. Every assertion a driver makes
# sits between its install and its uninstall, so any one of them failing would otherwise leave an
# orphaned row in Settings > Installed apps on the developer's own machine. One did, and it was
# reported as a Windows bug -- which is exactly what such a row looks like from Settings.
#
# Only ever removes directories the driver made. A failed uninstall leaves a populated folder behind,
# and that is what should be cleaned up here rather than left on the disk.
inno_cleanup_scratch() { # <scratch root>
  local root="$1" dir waited=0
  [ -d "$root" ] || return 0
  for dir in "$root"/*/; do
    [ -f "$dir/unins000.exe" ] || continue
    MSYS2_ARG_CONV_EXCL='*' "$dir/unins000.exe" /VERYSILENT /NORESTART >/dev/null 2>&1 || true
  done
  # Polled for the reason inno_uninstall_from polls: the uninstaller relaunches itself from a temp
  # copy and returns immediately, so deleting the tree straight away would race it -- and an
  # uninstaller killed halfway leaves the registration behind, which is the thing this exists to
  # prevent. The bound is what keeps a failing build from hanging in its own trap.
  while [ "$waited" -lt 60 ]; do
    ls -d "$root"/*/unins000.exe >/dev/null 2>&1 || break
    sleep 1
    waited=$((waited + 1))
  done
  rm -rf "$root" 2>/dev/null || true
}
