#!/usr/bin/env bash
#
# Installs what the optional `video` feature needs to build -- ffmpeg's development libraries and
# libclang -- and then teaches **cargo itself** where they are, so no build ever has to be run
# through a wrapper or a sourced script.
#
#   tools/setup/fetch-ffmpeg.sh           # install what is missing, then record the paths for cargo
#   tools/setup/fetch-ffmpeg.sh --force   # re-fetch even if present (Windows only; the rest are packages)
#   tools/setup/fetch-ffmpeg.sh --print   # resolve and report only: install nothing, write nothing
#   tools/setup/fetch-ffmpeg.sh --homebrew # macOS: take Homebrew's GPL ffmpeg instead of building an LGPL one
#
# **Why this exists.** `ffmpeg-sys-next` needs FFMPEG_DIR *and* LIBCLANG_PATH, and when either is
# missing it fails with a wall of `pkg-config`/vcpkg noise that names neither -- which is the single
# most confusing failure a fresh clone of this repository can produce. libclang is mandatory rather
# than a convenience: the crate runs bindgen at build time and ships no pre-generated bindings.
#
# **Why it writes to cargo's config rather than telling you to export two variables.** Cargo's
# `[env]` table is applied to every build script it runs, so once this is recorded, `cargo build
# --features video` works from any shell, from an IDE that invokes cargo itself, and from a script --
# none of which inherit an `export` you typed. An IDE failing on `ffmpeg-sys-next` is exactly the
# symptom of that gap. Anything already set in the environment still wins: cargo's `[env]` does not
# override a real environment variable unless it is marked `force`, and this does not mark it.
#
# It writes to `$CARGO_HOME/config.toml` (usually `~/.cargo/config.toml`), between two marker lines,
# and rewrites only what is between them. The project's own `.cargo/config.toml` cannot hold this:
# it is committed, and these paths are particular to one machine. A per-project override file is not
# an option either -- cargo's config `include` key is **silently ignored on stable** (verified on
# 1.98: no error, no effect), so a project-local file would look right and do nothing.
#
# Nothing else in the build needs any of this. The `video` feature is off by default, so a clone that
# only ever plays MIDI can skip this script entirely -- see BUILDING.md.

set -euo pipefail

cd "$(dirname "$0")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

FORCE=0
PRINT_ONLY=0
PRINT_DIR=0
HOMEBREW=0
for arg in "$@"; do
  case "$arg" in
    --force) FORCE=1 ;;
    # macOS only. Takes Homebrew's ffmpeg instead of building the pinned LGPL one -- faster, already
    # installed on many machines, and GPL. What a *release* built against it means is in
    # `Video in a macOS release` in docs/decisions/song-sources.md; for a development build it makes
    # no difference.
    --homebrew) HOMEBREW=1 ;;
    --print) PRINT_ONLY=1 ;;
    # Machine-readable: resolve, print FFMPEG_DIR on stdout, say nothing else, change nothing. This
    # exists so `tools/platform/windows/dist.sh --video` can find the DLLs it has to stage without knowing
    # where they live -- one resolver, used by both, rather than two that can drift apart.
    --print-dir) PRINT_DIR=1; PRINT_ONLY=1 ;;
    -h|--help) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "fetch-ffmpeg: unknown option $arg" >&2; exit 2 ;;
  esac
done

# ---------------------------------------------------------------------------------------------
# The pinned Windows build.
#
# **Deliberately 7.1, not the newest, and that is the load-bearing part of this pin.** It reports
# `libavutil 59.39.100` and ships `avcodec-61`, which is exactly what Debian trixie's `libavutil59` /
# `libavcodec61` are -- and trixie is what the appliance runs. A newer ffmpeg on a development box
# lets code compile against API that Debian lacks, and the failure then appears only in Docker or in
# CI. The development machine wants to be the lower bound.
#
# **LGPL, not GPL.** This project only ever decodes, the LGPL build carries every decoder that needs,
# and shipping those DLLs beside the executable is compliant under LGPL and would not be under GPL.
#
# **A dated tag, not `latest`.** BtbN's `latest` is rolling -- the same URL returns different bytes
# over time -- and it no longer carries an n7.1 build at all. The `autobuild-*` tags persist, and
# they carry the source commit in the filename.
FF_TAG="autobuild-2026-05-31-13-22"
FF_NAME="ffmpeg-n7.1.4-7-gadcf20da26-win64-lgpl-shared-7.1"
FF_URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/$FF_TAG/$FF_NAME.zip"
FF_SHA256="5aac08b02110e2b3f0bcd015fbc535b254aab543d51887ccc1db7178d2c58b06"

# ---------------------------------------------------------------------------------------------
# The pinned source build: which release, which bytes, which flags.
#
# **All three live in tools/setup/ffmpeg-pin.sh**, sourced below, because tools/platform/linux/ffmpeg-lgpl.sh builds
# the same ffmpeg inside the Debian container and the two must not drift. They already had, by one
# flag, before anybody noticed -- which is how that file came to exist. What stays here is the macOS
# *procedure*: where it caches, how it reports, and how FFMPEG_DIR comes out of it.
#
# **Built here rather than downloaded, because nothing publishes what this needs.** BtbN has no macOS
# target at all; evermeet.cx ships a static GPL *command*, not shared libraries with headers;
# ColorsWind/FFmpeg-macOS is dormant on 5.0.1; and ffmpeg.org itself says plainly that it provides
# source code only. So the choice on this platform is not "prebuilt or source" -- it is "Homebrew's
# GPL build, or one built here". See `Video in a macOS release` in docs/decisions/song-sources.md.
. tools/setup/ffmpeg-pin.sh

# The same cache convention `tools/setup/fetch-assets.sh` uses, for the same reason: outside the repo, so
# `cargo clean` and a fresh clone both leave it alone, and sibling checkouts share one copy. One
# definition, in tools/setup/asset-cache.sh, shared with them and with tools/dev/clean.sh.
. tools/setup/asset-cache.sh

if command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
elif command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
else
  echo "fetch-ffmpeg: need shasum or sha256sum to verify the download" >&2
  exit 1
fi

# Progress goes to stdout normally, and nowhere under --print-dir, whose stdout is a value another
# script parses. Errors always go to stderr, so a caller capturing the value still sees them.
say() { [ "$PRINT_DIR" -eq 1 ] || printf '%s\n' "$*"; }
die() { printf 'fetch-ffmpeg: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
  Linux)  PLATFORM=linux   ;;
  Darwin) PLATFORM=macos   ;;
  MINGW*|MSYS*|CYGWIN*) PLATFORM=windows ;;
  *) die "unsupported platform $(uname -s)" ;;
esac

FFMPEG_DIR="${FFMPEG_DIR:-}"
LIBCLANG_PATH="${LIBCLANG_PATH:-}"
FFMPEG_BIN=""

# ---------------------------------------------------------------------------------------------
# Windows: one pinned zip into the cache.

fetch_windows() {
  local dest="$CACHE/$FF_NAME" zip="$CACHE/$FF_NAME.zip" got llvm
  mkdir -p "$CACHE"

  if [ -d "$dest/include" ] && [ -d "$dest/lib" ] && [ "$FORCE" -eq 0 ]; then
    say "cached    ffmpeg $FF_NAME"
  elif [ "$PRINT_ONLY" -eq 1 ]; then
    die "ffmpeg is not in the cache at $dest; run this without --print to fetch it"
  else
    if [ -f "$zip" ] && [ "$FORCE" -eq 0 ] && [ "$(sha256 "$zip")" = "$FF_SHA256" ]; then
      say "cached    $FF_NAME.zip"
    else
      say "fetching  $FF_NAME.zip  (62 MB)"
      curl -fL --progress-bar --retry 3 --connect-timeout 20 -o "$zip.part" "$FF_URL"
      got="$(sha256 "$zip.part")"
      if [ "$got" != "$FF_SHA256" ]; then
        rm -f "$zip.part"
        die "$FF_NAME.zip failed verification
  expected $FF_SHA256
  got      $got"
      fi
      # Moved into place only after it verifies, so an interrupted run cannot leave a half file
      # looking cached.
      mv "$zip.part" "$zip"
    fi

    say "unpacking $FF_NAME"
    rm -rf "$dest"
    # The zip has the release name as its single top-level directory, so this lands exactly at
    # $dest with include/, lib/ and bin/ under it.
    ( cd "$CACHE" && unzip -q -o "$FF_NAME.zip" )
  fi

  [ -d "$dest/include" ] && [ -d "$dest/lib" ] || die "unpacked $dest has no include/ and lib/"
  [ -n "$FFMPEG_DIR" ] || FFMPEG_DIR="$dest"

  # libclang. Nothing puts LLVM on PATH on Windows, so the usual install location is probed and
  # winget is only *suggested* -- it needs elevation, and from a non-interactive shell it fails with
  # `0x800704c7 : The operation was canceled by the user`, which is a dismissed UAC prompt rather
  # than what it sounds like. Better to say so than to fail that way inside a script.
  if [ -z "$LIBCLANG_PATH" ]; then
    for llvm in "/c/Program Files/LLVM/bin" "/c/Program Files (x86)/LLVM/bin"; do
      [ -f "$llvm/libclang.dll" ] && { LIBCLANG_PATH="$llvm"; break; }
    done
  fi
  if [ -z "$LIBCLANG_PATH" ]; then
    die "libclang was not found, and ffmpeg-sys-next cannot build without it.
  Install LLVM, then run this again:

      winget install LLVM.LLVM

  That prompts for elevation; accept it. If LLVM is already installed somewhere unusual, set
  LIBCLANG_PATH to the directory holding libclang.dll instead."
  fi

  # The DLLs. These matter to *run* a video build rather than to build one, and they are the one
  # thing cargo's `[env]` cannot arrange: it can set a variable but not append to PATH, and replacing
  # PATH wholesale from a config file would be worse than the problem. So this goes on the user's own
  # PATH, once, below.
  FFMPEG_BIN="$dest/bin"

  # And the variables themselves must be in **Windows** form. `ffmpeg-sys-next`'s build script and
  # bindgen are native Windows programs; handed `/c/Users/...` they look for a directory named `c`
  # off the root of the current drive and then report that ffmpeg is missing -- which reads exactly
  # like never having installed it. `cygpath -m` gives `C:/Users/...`, understood by both.
  if command -v cygpath >/dev/null 2>&1; then
    FFMPEG_DIR="$(cygpath -m "$FFMPEG_DIR")"
    LIBCLANG_PATH="$(cygpath -m "$LIBCLANG_PATH")"
  fi
}

# Appends the DLL directory to the *user's* PATH, idempotently. Deliberately PowerShell and
# deliberately the .NET API: `$env:PATH` in PowerShell is the merged machine+user value, so reading
# that and writing it back to User scope would copy the whole system PATH into the user's, for good.
# `setx` is avoided for a second reason -- it truncates at 1024 characters.
add_windows_path() {
  local win_bin
  win_bin="$(cygpath -w "$FFMPEG_BIN")"
  if ! command -v powershell.exe >/dev/null 2>&1; then
    say "note      add this to your PATH by hand, to run video builds:"
    say "          $win_bin"
    return 0
  fi
  powershell.exe -NoProfile -NonInteractive -Command "
    \$bin = '$win_bin'
    \$userPath = [Environment]::GetEnvironmentVariable('PATH','User')
    if (\$userPath -like \"*\$bin*\") { 'PATH      already carries the ffmpeg DLLs' }
    else {
      [Environment]::SetEnvironmentVariable('PATH', (\$userPath.TrimEnd(';') + \";\$bin\"), 'User')
      'PATH      appended the ffmpeg DLLs to your user PATH (restart your shell and IDE)'
    }" | tr -d '\r'
}

# ---------------------------------------------------------------------------------------------
# macOS: a pinned LGPL ffmpeg built from source, the Command Line Tools for libclang.

# Builds the pinned source into the cache and leaves a normal ffmpeg prefix behind -- `include/` and
# `lib/`, exactly the shape FFMPEG_DIR wants and the same shape the Windows zip unpacks to.
#
# Note what this deliberately does *not* produce: `--disable-programs` means there is no `ffmpeg`
# command in it. Nothing here wants one -- these are libraries to link against -- but km-pack's
# re-encode path does shell out to an `ffmpeg` on PATH, so a machine that packages irregular video
# still wants a real one installed. That is a separate tool from these libraries and always was.
build_macos_ffmpeg() { # -> leaves $FFMPEG_DIR pointing at the built prefix
  local dest="$CACHE/$FF_SRC_NAME-lgpl" tar="$CACHE/$FF_SRC_NAME.tar.xz" src="$CACHE/src" got jobs

  if [ -d "$dest/include" ] && [ -d "$dest/lib" ] && [ "$FORCE" -eq 0 ]; then
    say "cached    ffmpeg $FF_SRC_VER (LGPL)"
    FFMPEG_DIR="$dest"
    return 0
  fi

  if [ -f "$tar" ] && [ "$FORCE" -eq 0 ] && [ "$(sha256 "$tar")" = "$FF_SRC_SHA256" ]; then
    say "cached    $FF_SRC_NAME.tar.xz"
  else
    say "fetching  $FF_SRC_NAME.tar.xz  (11 MB)"
    curl -fL --progress-bar --retry 3 --connect-timeout 20 -o "$tar.part" "$FF_SRC_URL"
    got="$(sha256 "$tar.part")"
    if [ "$got" != "$FF_SRC_SHA256" ]; then
      rm -f "$tar.part"
      die "$FF_SRC_NAME.tar.xz failed verification
  expected $FF_SRC_SHA256
  got      $got"
    fi
    # Moved into place only after it verifies, so an interrupted run cannot leave a half file looking
    # cached. Same rule as the Windows zip.
    mv "$tar.part" "$tar"
  fi

  # x86 assembly needs nasm; arm64 assembles with the toolchain already present, which is why an
  # Apple-silicon machine needs nothing installed for this at all. Named rather than worked around:
  # --disable-x86asm would build, and be materially slower at exactly the thing this is for.
  if [ "$(uname -m)" = "x86_64" ] && ! command -v nasm >/dev/null 2>&1; then
    die "building ffmpeg on an Intel Mac needs nasm for its x86 assembly. Install it and try again:

      brew install nasm

  Or run this with --homebrew to take Homebrew's ffmpeg instead (which is GPL; see
  docs/decisions/song-sources.md)."
  fi

  # **Needed on every Mac, unlike nasm above, which only an Intel one wants.** The pinned configure
  # asks for `--enable-libopenh264`, the one external library it takes and the only H.264 encoder an
  # LGPL ffmpeg can have — see `tools/setup/ffmpeg-pin.sh`. Checked here rather than left to
  # configure, which reports a missing library as one line inside a log this script then has to be
  # read out of.
  if ! pkg-config --exists openh264 2>/dev/null; then
    die "building ffmpeg needs openh264, which is what a streaming machine encodes with. \
Install it and try again:

      brew install openh264

  Or run this with --homebrew to take Homebrew's ffmpeg instead (which is GPL; see
  docs/decisions/song-sources.md)."
  fi

  say "unpacking $FF_SRC_NAME"
  rm -rf "$src" "$dest"
  mkdir -p "$src"
  tar -xJf "$tar" -C "$src"

  jobs="$(sysctl -n hw.ncpu 2>/dev/null || echo 4)"
  say "building  ffmpeg $FF_SRC_VER, LGPL, decode only -- a few minutes, once on this machine"
  (
    cd "$src/$FF_SRC_NAME"
    ./configure --prefix="$dest" "${FF_SRC_CONFIGURE[@]}" > "$src/configure.log" 2>&1 \
      || { tail -25 "$src/configure.log" >&2; die "ffmpeg's configure failed; the tail of it is above
  The whole log is at $src/configure.log"; }
    make -j"$jobs" > "$src/make.log" 2>&1 \
      || { tail -25 "$src/make.log" >&2; die "building ffmpeg failed; the tail of it is above
  The whole log is at $src/make.log"; }
    make install > "$src/install.log" 2>&1 \
      || { tail -25 "$src/install.log" >&2; die "installing ffmpeg into $dest failed"; }
  )

  # `make install` does not install the license texts, and Homebrew's prefix carries them only because
  # Homebrew puts them there -- so without this the staged build would have no terms beside it, which
  # is not compliant and which `dist_stage_ffmpeg_license_macos` would (rightly) warn about. Copied
  # into the prefix so the release scripts find them in the same place on either kind of ffmpeg.
  cp "$src/$FF_SRC_NAME"/COPYING.* "$src/$FF_SRC_NAME"/LICENSE.md "$dest/" 2>/dev/null || \
    say "warning   ffmpeg's source carried no COPYING files; the staged terms will be missing"

  [ -d "$dest/include" ] && [ -d "$dest/lib" ] || die "the build left no include/ and lib/ in $dest"
  # The tree is ~1 GB of objects and the tarball is cached, so a rebuild costs a re-unpack and not a
  # re-download. The logs go with it; they were only ever for the failure paths above.
  rm -rf "$src"
  say "built     $dest"
  FFMPEG_DIR="$dest"
}

fetch_macos() {
  local candidate dest="$CACHE/$FF_SRC_NAME-lgpl"
  mkdir -p "$CACHE"

  # Resolution order, and the first two are both ways of saying "do not build one":
  #   1. FFMPEG_DIR already in the environment -- somebody's own build, and it wins everywhere else
  #      in this project too, so it wins here.
  #   2. --homebrew, asked for explicitly.
  #   3. the pinned LGPL build in the cache, built now if it is not there.
  if [ -n "$FFMPEG_DIR" ]; then
    say "using     FFMPEG_DIR from the environment"
  elif [ "$HOMEBREW" -eq 1 ]; then
    command -v brew >/dev/null 2>&1 || die "--homebrew needs Homebrew; see https://brew.sh"
    if ! brew --prefix ffmpeg >/dev/null 2>&1; then
      [ "$PRINT_ONLY" -eq 1 ] && die "ffmpeg is not installed (brew install ffmpeg)"
      say "installing ffmpeg via Homebrew"
      brew install ffmpeg
    fi
    FFMPEG_DIR="$(brew --prefix ffmpeg)"
  elif [ -d "$dest/include" ] && [ -d "$dest/lib" ] && [ "$FORCE" -eq 0 ]; then
    say "cached    ffmpeg $FF_SRC_VER (LGPL)"
    FFMPEG_DIR="$dest"
  elif [ "$PRINT_ONLY" -eq 1 ]; then
    # Resolving must not build anything -- callers use --print to fail in a second rather than after
    # a release build. Homebrew's is reported if it is there, because it is a real answer to "is
    # there an ffmpeg to build against", which is all --print is asked.
    if command -v brew >/dev/null 2>&1 && brew --prefix ffmpeg >/dev/null 2>&1; then
      FFMPEG_DIR="$(brew --prefix ffmpeg)"
    else
      die "no ffmpeg yet: run tools/setup/fetch-ffmpeg.sh (it builds a pinned LGPL one, a few minutes)"
    fi
  else
    build_macos_ffmpeg
  fi
  [ -d "$FFMPEG_DIR/include" ] || die "ffmpeg at $FFMPEG_DIR has no include/"

  # The Command Line Tools carry libclang and this build already requires them for SDL, so there is
  # normally nothing to install. Homebrew's llvm is checked second, for a box that has one.
  if [ -z "$LIBCLANG_PATH" ]; then
    for candidate in "$(xcode-select -p 2>/dev/null)/usr/lib" "$(brew --prefix llvm 2>/dev/null)/lib"; do
      [ -f "$candidate/libclang.dylib" ] && { LIBCLANG_PATH="$candidate"; break; }
    done
  fi
  [ -n "$LIBCLANG_PATH" ] || die "libclang was not found; run: xcode-select --install"

  # Nothing to add to a search path, unlike Windows: a Mach-O records the absolute path it was linked
  # at, so a cargo build finds these wherever they are. Getting them into a *release* is a different
  # job and a solved one -- see `dist_stage_ffmpeg_macos` in tools/dist/common.sh.
}

# ---------------------------------------------------------------------------------------------
# Linux: the distribution's own -dev packages.

fetch_linux() {
  # Debian/Ubuntu names first, because that is what CI, the Docker image and the appliance all use.
  local pkgs_apt="libavcodec-dev libavformat-dev libavutil-dev libswresample-dev libclang-dev"
  local pkgs_dnf="ffmpeg-free-devel clang-devel"
  local pkgs_pac="ffmpeg clang"
  local mgr="" pkgs=""

  if   command -v apt-get >/dev/null 2>&1; then mgr="apt-get install -y --no-install-recommends"; pkgs="$pkgs_apt"
  elif command -v dnf     >/dev/null 2>&1; then mgr="dnf install -y"; pkgs="$pkgs_dnf"
  elif command -v pacman  >/dev/null 2>&1; then mgr="pacman -S --needed --noconfirm"; pkgs="$pkgs_pac"
  else die "no apt-get, dnf or pacman found; install ffmpeg's development libraries and libclang by hand"
  fi

  # Neither variable is wanted here, and that is not an omission. ffmpeg-sys-next finds a
  # distribution ffmpeg through pkg-config and clang-sys finds libclang through the distribution's
  # own layout; setting FFMPEG_DIR would override a correct answer with a guess. So this branch
  # installs and checks, and writes nothing to cargo's config.
  if pkg-config --exists libavutil 2>/dev/null && [ "$FORCE" -eq 0 ]; then
    say "present   ffmpeg development libraries ($(pkg-config --modversion libavutil))"
  elif [ "$PRINT_ONLY" -eq 1 ]; then
    die "ffmpeg development libraries are missing; run: sudo $mgr $pkgs"
  else
    say "installing $pkgs"
    say "          (this needs root, so sudo will ask)"
    # Unquoted on purpose: both are word lists.
    # shellcheck disable=SC2086
    sudo $mgr $pkgs
  fi
}

# ---------------------------------------------------------------------------------------------
# Recording the answer where cargo will find it.

BEGIN_MARK="# >>> karaokemachine video (written by tools/setup/fetch-ffmpeg.sh) >>>"
END_MARK="# <<< karaokemachine video <<<"

write_cargo_env() {
  local home cfg tmp
  home="${CARGO_HOME:-$HOME/.cargo}"
  cfg="$home/config.toml"
  mkdir -p "$home"
  [ -f "$cfg" ] || : > "$cfg"

  # Everything outside the two markers is somebody else's and is copied through untouched. Written
  # to a temporary file and moved, so an interrupted run cannot truncate a config that may hold a
  # registry mirror or a linker choice.
  tmp="$cfg.km.$$"
  awk -v b="$BEGIN_MARK" -v e="$END_MARK" '
    $0 == b { skip = 1; next }
    $0 == e { skip = 0; next }
    !skip   { print }
  ' "$cfg" > "$tmp"

  # A trailing blank line only if the surviving config did not already end in one.
  if [ -s "$tmp" ] && [ -n "$(tail -c 1 "$tmp")" ]; then printf '\n' >> "$tmp"; fi

  {
    printf '%s\n' "$BEGIN_MARK"
    printf '# Rewritten in place each time that script runs; edit outside the markers, not inside.\n'
    printf '# `[env]` is applied to every build script cargo runs, which is why a build works here\n'
    printf '# from any shell and from an IDE. A real environment variable still wins: these are not\n'
    printf '# marked `force`.\n'
    printf '[env]\n'
    printf 'FFMPEG_DIR = "%s"\n' "$FFMPEG_DIR"
    printf 'LIBCLANG_PATH = "%s"\n' "$LIBCLANG_PATH"
    printf '%s\n' "$END_MARK"
  } >> "$tmp"

  mv "$tmp" "$cfg"
  say "recorded  FFMPEG_DIR and LIBCLANG_PATH in $cfg"
}

case "$PLATFORM" in
  windows) fetch_windows ;;
  macos)   fetch_macos   ;;
  linux)   fetch_linux   ;;
esac

# The whole point of --print-dir: one line on stdout, nothing else, exit.
if [ "$PRINT_DIR" -eq 1 ]; then
  [ -n "$FFMPEG_DIR" ] || die "there is no FFMPEG_DIR on this platform (ffmpeg comes from pkg-config)"
  printf '%s\n' "$FFMPEG_DIR"
  exit 0
fi

if [ "$PRINT_ONLY" -eq 0 ] && [ -n "$FFMPEG_DIR" ] && [ -n "$LIBCLANG_PATH" ]; then
  write_cargo_env
fi
if [ "$PRINT_ONLY" -eq 0 ] && [ "$PLATFORM" = windows ]; then
  add_windows_path
fi

echo
say "video dependencies ready"
if [ -n "$FFMPEG_DIR" ];    then say "   FFMPEG_DIR     $FFMPEG_DIR"; fi
if [ -n "$LIBCLANG_PATH" ]; then say "   LIBCLANG_PATH  $LIBCLANG_PATH"; fi
if [ -n "$FFMPEG_BIN" ];    then say "   runtime DLLs   $FFMPEG_BIN"; fi
if [ -z "$FFMPEG_DIR" ] && [ -z "$LIBCLANG_PATH" ]; then
  say "   found through pkg-config; cargo needs no help and nothing was written"
fi
echo
say "Nothing to export, and nothing to source. The plain commands are the video ones:"
say ""
say "    cargo km                                         # run the machine, video included"
say "    cargo km-build                                   # the workspace, with video"
say "    cargo km-test                                    # ...and its tests"
