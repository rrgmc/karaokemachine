# shellcheck shell=bash
#
# What the portable tarball needs from the system it runs on, per distribution family.
#
#   . tools/platform/linux/runtime-deps.sh          # sets deps_debian, deps_fedora, deps_arch
#   runtime_deps_for <family>              # prints one family's list, whitespace-normalized
#   runtime_deps_install_cmd <family>      # prints the command that installs it
#   runtime_deps_readme_block              # prints the block README.txt shows the user
#
# Sourced by tools/platform/linux/verify-tarball.sh (which installs these to prove the tarball runs) and by
# tools/platform/linux/tarball-in-container.sh (which writes them into the tarball's README.txt). No shebang
# and no `set -euo pipefail`, for the reason tools/dist/common.sh gives: a sourced file must not
# change the caller's shell.
#
# **Why this file exists.** These lists were written out twice -- once in the verifier and once in the
# README generator -- and the verifier's whole claim is that "the README and this script can be
# compared line by line". That claim was being discharged by a human comparing two heredocs, which is
# exactly the kind of check that holds until the day it does not. Worse, prewarming the verifier's
# image (see tools/platform/linux/verify-image.sh) would have made it three copies. One list, three consumers.
#
# **These are measured, not transcribed.** Every one was checked by installing exactly it into a clean
# container of that distribution and starting the application there. `tools/platform/linux/verify-tarball.sh`
# is that check, kept runnable. Do not add a package here because a wiki says SDL wants it; add it
# because the verifier failed without it.
#
# SDL3, SDL3_ttf and SQLite are compiled into the binary, the instrument bank and the wallpapers are
# in assets/, and a font is bundled -- so none of those appear here. What is left is what SDL opens
# at run time.

deps_debian="libx11-6 libxext6 libxrandr2 libxcursor1 libxi6 libxfixes3 libxss1 libxtst6
             libxrender1 libdrm2 libgbm1 libegl1 libgles2 libgl1-mesa-dri libudev1 libasound2t64
             zlib1g"
deps_fedora="libX11 libXext libXrandr libXcursor libXi libXfixes libXScrnSaver libXtst
             libXrender libdrm mesa-libgbm mesa-libEGL mesa-libGLES mesa-dri-drivers
             systemd-libs alsa-lib zlib-ng-compat"
deps_arch="libx11 libxext libxrandr libxcursor libxi libxfixes libxss libxtst libxrender
           libdrm mesa systemd-libs alsa-lib zlib"

# A note that belongs to the README rather than to the installer: libasound2t64 is the 64-bit-time_t
# rename, so an older release wants the old name. The verifier never sees it, because it runs against
# Debian 13 and Ubuntu 24.04 or newer.
readme_note_debian="(libasound2 on releases before Ubuntu 24.04)"

runtime_deps_for() { # <family> -> the list on one line, single-spaced
  case "$1" in
    debian) printf '%s' "$deps_debian" ;;
    fedora) printf '%s' "$deps_fedora" ;;
    arch)   printf '%s' "$deps_arch" ;;
    *)      echo "runtime-deps: unknown family '$1'" >&2; return 1 ;;
  esac | tr -s '[:space:]' ' '
}

# The install command, and **the exact text matters** -- it is what both the run-time install in
# verify-tarball.sh and the prewarmed image in verify-image.sh run, so that the two are the same test.
#
# Note what is deliberately absent from the Debian line: `--no-install-recommends`. Without it the
# Recommends of those 17 packages are installed too, which means the tarball is being verified in a
# slightly more furnished environment than the strict list describes. That is the behavior this
# check has always had. Adding the flag would make it stricter and might turn up a library the tarball
# has been quietly getting through a Recommends -- a real finding, but one to go looking for
# deliberately, not to discover as a side effect of a change about speed.
runtime_deps_install_cmd() { # <family> -> the shell command that installs that family's list
  case "$1" in
    debian) printf 'apt-get update -qq && apt-get install -y -qq %s >/dev/null' "$(runtime_deps_for debian)" ;;
    fedora) printf 'dnf install -y -q %s >/dev/null' "$(runtime_deps_for fedora)" ;;
    # `-Syu`, not `-Sy`. Arch is rolling, so installing into a partially-updated system is the
    # partial-upgrade hazard its own documentation warns about -- and a prewarmed image makes the
    # window longer, not shorter, because the database ages with the image.
    arch)   printf 'pacman -Syu --noconfirm --needed %s >/dev/null' "$(runtime_deps_for arch)" ;;
    *)      echo "runtime-deps: unknown family '$1'" >&2; return 1 ;;
  esac
}

# The three-column block README.txt carries. Wrapped here rather than stored pre-wrapped, so that the
# lists above stay the only place a package name is written.
runtime_deps_readme_block() {
  # Two extra spaces before the aside, so it reads as a remark rather than as another package. The
  # wrapper collapses runs of whitespace, so they are re-inserted after wrapping.
  _rd_row "Debian, Ubuntu" "$(runtime_deps_for debian) $readme_note_debian" \
    | sed "s/ ${readme_note_debian%% *}/   ${readme_note_debian%% *}/"
  _rd_row "Fedora" "$(runtime_deps_for fedora)"
  _rd_row "Arch" "$(runtime_deps_for arch)"
}

# One label plus its wrapped list. Two spaces of indent, a 17-column label, continuation lines
# aligned under the first package -- which is what the file looked like when it was hand-wrapped.
_rd_row() { # <label> <words>
  printf '%s' "$2" | awk -v label="$1" '
    BEGIN { indent = "                   "; width = 98 }
    {
      line = sprintf("  %-17s", label)
      for (i = 1; i <= NF; i++) {
        if (length(line) + 1 + length($i) > width) { print line; line = indent }
        line = line (line ~ / $/ || line == indent ? "" : " ") $i
      }
      print line
    }'
}
