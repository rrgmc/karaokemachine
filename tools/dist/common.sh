# shellcheck shell=bash
#
# Shared staging helpers. Sourced, never executed -- there is no shebang and no `set -euo pipefail`
# here on purpose, because a sourced file that sets shell options changes the caller's shell in ways
# the caller did not ask for. Every caller sets its own.
#
#   cd "$(dirname "$0")/../.."      # ...however many levels reach the repository root
#   . tools/dist/common.sh
#   DIST_SCRIPT=dist                # what error messages call this script
#
# Every function here assumes the working directory is the repository root, which is what every
# staging script does as its first act.
#
# **How many `..` depends on where the caller sits, and every caller counts them by hand** -- two
# from `tools/dist/`, `tools/dev/` and `tools/setup/`, three from `tools/platform/<os>/`, four from
# `tools/port/<product>/<os>/`. Getting it wrong lands somewhere with no `tools/dist/common.sh` in
# it and fails on this very line, which is loud and says nothing useful; and a caller that does not
# source this file at all gets no check at all. Hence `dist_assert_root` below, and hence the same
# two lines open the six scripts that deliberately do not source this one.
#
# Why this file exists. Seven things get staged -- the app on three desktops and
# on Android, and three command-line tools -- and before this they each spelled out the same five
# idioms: read the version out of the artifact, clear a folder without removing it, copy the asset
# tree, total the bytes, zip the result. Five copies of an idiom is five chances for one of them to
# drift, and the one that matters most is the *layout*: `dist/<app>/<platform>/<thing>` is a rule
# about where releases go, and a rule written out seven times is not a rule.
#
# What is deliberately NOT here: the README each *staged folder* gets. Those differ in every word,
# they are the part a person actually reads, and a "shared" README with seven substitutions would be
# worse than seven honest ones.
#
# **`dist_installed_readme` below is the exception, and it is here for that rule's own reason rather
# than in spite of it.** It is not one of the seven: it is the document an *installed* build gets,
# there are exactly two callers -- the Windows and the macOS setup programs -- and what they have to
# agree about is facts rather than prose. Two setup programs describing how to remove the same
# product differently is what one copy prevents, and it is invisible when it happens, because nobody
# opens an installed README until something has already gone wrong.

# -- what a staging run says out loud ----------------------------------------------------------------

# A release is quiet, and a diagnostic is asked for by name. That is the rule `What a shipped build
# says out loud` in docs/decisions/distribution.md applies to the shipped binaries, arriving late at
# the scripts that
# build them -- where it was the worse of the two problems, because a staging run's noise is not this
# repository's own `debug!` stream but `cargo build --release`, and `tools/dist/cmd.sh` with no
# arguments is that six times over.
#
# What stays at the default verbosity: the `== phase` lines, every final report, every `verified:`,
# every warning, and everything on stderr. What goes: the build logs, and the detail lines that are
# only interesting when a build turns out to have linked the wrong thing.
# **Asserted here, at source time, because this file is the first thing every caller reaches after
# its `cd`.** It costs one `test` and turns "some later command behaved oddly" into one line naming
# the script and the directory it actually landed in.
dist_assert_root() {
  [ -f Cargo.toml ] && [ -d crates ] && [ -d tools ] && return 0
  echo "${0##*/}: not at the repository root -- landed in $PWD" >&2
  echo "  (the \`cd \"\$(dirname \"\$0\")/..\"\` at the top of this script counts the wrong number of levels)" >&2
  exit 1
}
dist_assert_root

DIST_VERBOSE="${DIST_VERBOSE:-0}"

dist_verbose() { [ "$DIST_VERBOSE" -eq 1 ]; }

# -- how a macOS build is signed ---------------------------------------------------------------------

# **Empty means ad-hoc, which is what this repository did for its whole life until now.** Set it to a
# Developer ID and every macOS signing call in the tree uses it instead. Same graceful-degrade
# convention `KM_CHECKOUT` follows: absent is a real answer and not a failure, because a fresh clone,
# somebody else's Mac and the CI runners have no certificates and must still be able to stage a build.
#
#     KM_SIGN_IDENTITY="Developer ID Application: Some Name (TEAMID)"
#
# **One variable read in one place, because the four call sites must not disagree.** A bundle whose
# dylibs are ad-hoc and whose executable is Developer ID is a bundle notarization refuses, and the
# refusal names neither file. `dist_codesign` below is what makes that impossible to get wrong.
KM_SIGN_IDENTITY="${KM_SIGN_IDENTITY:-}"

# The team identifier, pulled out of the identity string rather than configured twice: a Developer ID
# is spelled `Developer ID Application: Name (TEAMID)`, and TEAMID is what a designated requirement
# and every provenance assertion needs. Empty for an ad-hoc build.
dist_team_id() { # -> prints the team identifier, or nothing
  case "$KM_SIGN_IDENTITY" in
    *\(*\)*) printf '%s' "${KM_SIGN_IDENTITY##*\(}" | tr -d ')' ;;
  esac
}

dist_signing() { [ -n "$KM_SIGN_IDENTITY" ]; }

# Every `codesign` in this repository goes through here.
#
# **`--options runtime` is applied at every call and not only to the bundle**, and that is the part
# that is easy to get wrong: sealing a bundle does not add the hardened runtime to code already signed
# inside it, and `codesign --verify --deep --strict` does not check that it is there -- so a partial
# application passes every check this tree has and then fails notarization, naming nothing useful.
#
# `--timestamp` asks Apple's timestamp authority, so a signed build needs the network where an ad-hoc
# one does not. Notarization requires it.
dist_codesign() { # <path...>
  if dist_signing; then
    codesign --force --sign "$KM_SIGN_IDENTITY" --options runtime --timestamp "$@"
  else
    codesign --force --sign - "$@"
  fi
}

# The code hash of a signed Mach-O: what it *is*, as distinct from the bytes it happens to occupy.
# Empty for anything unsigned or not a Mach-O, so a caller comparing two of these must check for that
# rather than treating two empties as a match.
dist_cdhash() { # <mach-o> -> prints the CDHash, or nothing
  codesign -d --verbose=4 "$1" 2>&1 | sed -n 's/^CDHash=//p'
}

# The one line every report prints about what it just made. Said in one place so that no two scripts
# can describe the same artifact differently.
dist_signing_note() { # -> prints one line
  if dist_signing; then
    printf 'Developer ID -- %s' "$KM_SIGN_IDENTITY"
  else
    printf 'ad-hoc -- set KM_SIGN_IDENTITY for a Developer ID build'
  fi
}

# `== thing`, the convention every staging script already used before there was a function for it.
# Always printed: it is what says which phase a silent minute is being spent in.
dist_step() { printf '== %s\n' "$*"; }

# A fact about the step just announced. `ffmpeg C:/Users/.../ffmpeg-n7.1.4-...` is what you want when
# a release turns out to have linked the wrong one, and noise every other time.
#
# The `return 0` is load-bearing rather than tidy: `dist_verbose && printf` returns 1 when it is not
# verbose, and every caller runs under `set -e` -- so without it the quiet path would take the script
# down at the first detail line, which is the one bug this whole change could plausibly ship.
dist_detail() { dist_verbose && printf '   %s\n' "$*"; return 0; }

# Cargo's own flag rather than capturing cargo's output, and the difference is the whole reason there
# are two mechanisms here instead of one: `--quiet` drops `Compiling` and `Finished` and leaves every
# rustc warning and error exactly where it was. So a quiet build that fails is already as
# diagnosable as a loud one, and needs neither a log file nor a rerun. Nothing else quieted below has
# that property, which is what `dist_run` exists for.
#
# Called unquoted -- `cargo build --release $(dist_cargo_quiet) -p x` -- so that the verbose case
# expands to no argument at all rather than to an empty one.
dist_cargo_quiet() { dist_verbose || printf -- '--quiet'; }

# Runs a command whose output is a build log rather than a report: docker's layer stream,
# fetch-assets.sh's progress -- anything answering "what is happening" rather than "what was
# produced".
#
# On failure the whole log is replayed to stderr, before the caller's `set -e` takes the script down.
# The tail would be cheaper, and is what tools/platform/linux/ffmpeg-lgpl.sh does for ffmpeg's `configure`;
# the whole thing is right here because these commands are minutes long and several are remote, so
# making somebody rerun one to find out why it failed is precisely the cost this exists to avoid
# rather than a small inconvenience.
dist_run() { # <label> <command...>
  local label="$1" log rc=0
  shift
  if dist_verbose; then "$@"; return; fi
  log="$(mktemp)"
  "$@" >"$log" 2>&1 || rc=$?
  if [ "$rc" -ne 0 ]; then
    printf '%s: %s failed (exit %s). Its output:\n' "${DIST_SCRIPT:-dist}" "$label" "$rc" >&2
    cat "$log" >&2
  fi
  rm -f "$log"
  return "$rc"
}

# Reassurance in place of the scroll. A quiet build is a silent minute or six, and `built in 2m14s`
# afterwards is what tells you the silence was work rather than a hang.
#
# `SECONDS` is bash's own, so this needs no `date`. That binds on nothing today, but the rule about
# what these scripts may assume is on PATH is worth keeping whether or not it currently bites.
dist_elapsed() { # <the value of SECONDS when the step started>
  local d=$((SECONDS - $1))
  if [ "$d" -ge 60 ]; then printf '%sm%ss' "$((d / 60))" "$((d % 60))"; else printf '%ss' "$d"; fi
}

# -- the layout ------------------------------------------------------------------------------------

# Where cargo puts what it builds.
#
# **Not `target/`.** That is only the default, and this repository is routinely built with
# `CARGO_TARGET_DIR` pointed somewhere else: off a spinning disk and onto an SSD, or at a slot of its
# own per worktree so two checkouts do not queue behind cargo's build lock -- which is exactly what
# `tools/dev/worktree.sh` arranges. A script that spells `target/release/x` then finds nothing, having
# just watched cargo print `Finished`, and reports it as a build that produced no executable. The
# error names the one path that was never going to be there, which is the least useful thing it
# could say.
#
# **Asking cargo is the only answer that covers every way the directory moves**: the environment
# variable, a `[build] target-dir` in any of the config.toml files cargo layers, and `--target-dir`.
# Reading `${CARGO_TARGET_DIR:-target}` covers the first of those three and looks like it covers all
# of them.
#
# It also settles `tools/cmd/assets/km-wallpaper-pack`, which is excluded from the workspace and therefore builds
# into a target directory of its own -- *unless* `CARGO_TARGET_DIR` is set, in which case it shares
# the same one as everything else. That is a fact about the environment, not about the crate, so
# nothing here can hard-code either answer; the manifest argument is what asks the right question.
#
# `--no-deps` is the difference between about 0.1s and resolving the whole dependency graph. The
# output is one line of JSON and the value wanted is a path, so `sed` is enough and no `jq` has to be
# installed -- which matters, because there is no jq on the Windows box this is mostly developed on.
#
# **The value is JSON, so on Windows its separators arrive doubled.** That is not a corner case: it
# is what happens whenever cargo derives the path itself rather than passing `CARGO_TARGET_DIR`
# through, i.e. the default. Un-escaping it is therefore mandatory, and the normalization to forward
# slashes afterwards is what keeps the two cases indistinguishable to every caller -- an environment
# variable set to `C:/Users/...` and a derived `C:\Users\...` should not produce paths that behave
# differently. It is done only for a drive-lettered path, because a backslash is a perfectly legal
# character in a Unix filename and rewriting one there would corrupt it.
#
# **It is deliberately not memoised**, though it looks like it should be. Every caller is written as
# `X="$(dist_target_dir)"`, and a command substitution is a subshell: a cache written inside one is
# gone before the assignment completes, so the version of this that carried an associative array
# never once returned a cached answer. Storing the result in the caller is the memoisation, and that
# is what every caller does; the staging loop in tools/dist/cmd.sh is the only place that asks more
# than once, at 0.1s a tool against a release build.
dist_target_dir() { # [manifest, default: this workspace's]  -> prints the directory cargo builds into
  local manifest="${1:-Cargo.toml}" dir
  dir="$(cargo metadata --format-version 1 --no-deps --manifest-path "$manifest" \
         | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
  if [ -z "$dir" ]; then
    echo "${DIST_SCRIPT:-dist}: could not ask cargo where it builds ($manifest)" >&2
    return 1
  fi
  case "$dir" in
    [A-Za-z]:*) dir="$(printf '%s' "$dir" | sed 's|\\\\|/|g')" ;;
    *)          dir="$(printf '%s' "$dir" | sed 's|\\\\|\\|g')" ;;
  esac
  printf '%s' "$dir"
}

# The one place the release layout is written down:
#
#   dist/<app>/<platform>/<the staged thing>
#
# App first, then platform. That ordering is the useful one because a release is a thing you hand to
# somebody -- you want every build of `km-package-builder` together, not every Windows build of
# everything together. It also gives each product a folder of its own, which the old layout only
# gave to `km-package-builder` while the app was scattered across `dist/windows`, `dist/linux` and
# `dist/macos`.
dist_dir() { # <app> <platform>
  printf 'dist/%s/%s' "$1" "$2"
}

# `rustc -vV` rather than a hard-coded triple, so a script that runs on all three desktops names what
# it actually built rather than what its author was sitting in front of.
dist_host_triple() {
  rustc -vV | awk '/^host: / {print $2}'
}

# The platform component of the path above. Short names rather than triples: `dist/km-pack/windows`
# reads as a place, `dist/km-pack/x86_64-pc-windows-msvc` reads as a build product -- and the triple
# is already in the folder name one level down, where it distinguishes an arm64 build from an x86 one.
dist_platform() { # [triple, default: this host]
  case "${1:-$(dist_host_triple)}" in
    *windows*) printf 'windows' ;;
    *darwin*)  printf 'macos' ;;
    *linux*)   printf 'linux' ;;
    *)         printf 'unknown' ;;
  esac
}

dist_exe_ext() { # <triple>
  case "$1" in *windows*) printf '.exe' ;; *) printf '' ;; esac
}

# The other half of `dist_dir`: given an app, a platform and a version, find the folder a staging
# script actually produced.
#
# **Constructing the name is not enough**, which is why this is a function rather than a `printf`. The
# full convention is `<app>-<version>-<triple><suffix>`, and the suffix is whatever was *declined* --
# `-no-video`, `-no-desktop`, or both -- so a caller that spells the plain name finds nothing after a
# `--no-video` run and reports it as a missing build. The exact name is tried first and a
# `<app>-<version>-*` glob is the fallback, which is the shape `Taskfile.yml`'s `_staged` already uses
# for `task run`.
#
# **This exists to stop the number of places that reconstruct the layout going up.** `dist_dir()` owns
# `dist/<app>/<platform>/`, and the folder name under it was until now reconstructed independently by
# `Taskfile.yml` and by `tools/dist/clean.sh` -- the coupling docs/ARCHITECTURE.md flags as the one to
# watch. `tools/dist/bin.sh` would have been a fourth; it calls this instead. The Taskfile keeps its
# own and has to: Task's embedded shell cannot source a bash file.
#
# The version is a parameter rather than read here, because the two ways of getting one are each right
# somewhere -- `dist_version` runs the artifact, and a script that is *looking* for the artifact has
# nothing to run yet, so it asks `cargo pkgid` exactly as tools/dist/clean.sh does and for the same
# reason.
# **Naming the suffixes you will accept is how a caller stays honest**, and leaving them out is what a
# caller that genuinely wants "whatever is there" does. The two are different questions and the
# difference is not cosmetic: `tools/dist/bin.sh --no-video` asked for a build that cannot play video,
# and a bare glob would have handed it the video folder staged an hour earlier and then written a
# README over it saying video songs will not play. So that caller passes the suffixes that answer its
# own flags, most specific first -- `-no-video-no-desktop`, `-no-video`, `-no-desktop`, `` -- and the
# empty one is what catches every product that has no such feature to decline and therefore never
# carries a marker. With no suffixes given the old behavior stands: the exact name, then a glob,
# which is what `task run` wants and what `Taskfile.yml`'s `_staged` does.
dist_staged_dir() { # <app> <platform> <version> [acceptable suffix...]  -> prints the directory, or fails
  local app="$1" platform="$2" version="$3" parent base d suffix
  shift 3
  parent="$(dist_dir "$app" "$platform")"
  base="$parent/$app-$version-$(dist_host_triple)"

  if [ "$#" -gt 0 ]; then
    for suffix in "$@"; do
      if [ -d "$base$suffix" ]; then printf '%s' "$base$suffix"; return 0; fi
    done
    echo "${DIST_SCRIPT:-dist}: nothing staged for $app $version under $parent/ matching what was asked for" >&2
    return 1
  fi

  if [ -d "$base" ]; then printf '%s' "$base"; return 0; fi
  for d in "$parent/$app-$version-"*/; do
    if [ -d "$d" ]; then printf '%s' "${d%/}"; return 0; fi
  done
  echo "${DIST_SCRIPT:-dist}: nothing staged for $app $version under $parent/" >&2
  return 1
}

# -- the artifact ------------------------------------------------------------------------------------

# The version comes from the binary rather than from a manifest, and that is not laziness. Every
# shipped crate says `version.workspace = true`, so the number lives in the root manifest and reading
# it means picking the right one of many `version =` lines. Asking the binary cannot disagree with
# the binary.
#
# Every command here uses clap's `version`, so `--version` prints `<name> <number>` and the second
# field is the number.
# The `./` is what stops a bare name being looked up on PATH, so it is not decoration -- but it made
# the helper accept only repository-relative paths, and `.//build/target/release/karaokemachine` is
# not a file. That is not a hypothetical: a build inside the container writes to CARGO_TARGET_DIR,
# which is an absolute path outside the source tree, so tools/platform/linux/tarball-in-container.sh has
# nothing relative to hand this. An absolute path is passed through as it is; everything else keeps
# the prefix and the guarantee that went with it.
#
# `/*` alone is not what "absolute" means on Windows, and that gap opened the moment the desktop
# staging scripts started asking cargo where it builds: the path they now hand this is routinely
# `C:/Users/.../release/km-pack.exe`, which is absolute and matches neither branch, so it became
# `./C:/Users/...` and the version read failed on a file that was sitting right there.
dist_version() { # <path to executable>  -> prints the version, or fails
  local exe="$1" version cmd
  case "$exe" in
    /* | [A-Za-z]:[/\\]*) cmd="$exe" ;;
    *)                    cmd="./$exe" ;;
  esac
  version="$("$cmd" --version | awk '{print $2}')"
  if [ -z "$version" ]; then
    echo "${DIST_SCRIPT:-dist}: could not read a version out of $exe --version" >&2
    return 1
  fi
  printf '%s' "$version"
}

# The product version as the manifests hold it, for a script that is looking *for* an artifact rather
# than at one and so has nothing to run. `dist_version` above is the other method and the better one
# wherever there is an executable. One number covers every product; see `One version number for the
# whole repository` in docs/decisions/repository.md.
dist_manifest_version() { # -> prints the version
  local pkgid
  pkgid="$(cargo pkgid -p karaokemachine)"
  pkgid="${pkgid##*#}"
  printf '%s' "${pkgid##*@}"
}

# -- the folder --------------------------------------------------------------------------------------

# Cleared rather than merged, so a file that stopped being shipped cannot survive a rerun and quietly
# go out in the next zip.
#
# The *contents* are cleared rather than the folder itself. On Windows a directory that any shell has
# as its working directory cannot be removed -- and `cd` into the staged folder to run the thing is
# exactly what every one of these scripts tells you to do at the end, so the next staging failed with
# "Device or resource busy" and no hint that the fix was to leave the folder. Clearing the contents
# gives the same guarantee. `find -delete` works depth-first, so files go before the directories
# holding them.
dist_clear() { # <dir>
  local dir="$1"
  if [ -d "$dir" ]; then
    find "$dir" -mindepth 1 -delete
  fi
  mkdir -p "$dir"
}

# The asset tree, copied under `<dest>/assets`. `.gitkeep` exists to hold empty directories in git and
# is dead weight in a release.
#
# `tools/port/machine/android/assets.sh` has the same loop and deliberately does not call this one: it copies into
# the APK's asset root rather than into an `assets/` subfolder, it adds the dev remote, and it writes
# a MANIFEST afterwards because SDL can open an asset by name but cannot list a directory. The rule
# about what counts as an asset is the shared part, and it is one line -- the `find` below.
dist_stage_assets() { # <dest dir>  -> prints the number of files copied
  local dest="$1" rel count=0
  # Checked here so that the three carriers that share this helper cannot forget it, and so that a
  # fourth one gets it for free. **On stderr, and that is not style**: this function runs inside a
  # command substitution and its stdout *is* the file count, so an unredirected report would end up
  # inside `$assets` and be printed as part of the total.
  tools/dist/check-assets.sh >&2 || return 1
  while IFS= read -r rel; do
    [ -n "$rel" ] || continue
    mkdir -p "$dest/assets/$(dirname "$rel")"
    cp "assets/$rel" "$dest/assets/$rel"
    count=$((count + 1))
  done < <(cd assets && find . -type f ! -name '.gitkeep' | sed 's|^\./||' | sort)
  printf '%s' "$count"
}

# `wc -c <` per file rather than one `wc -c` over all of them: that prints a "total" line which has to
# be filtered out by name, and `stat`'s size flag differs between GNU and BSD.
dist_bytes() { # <dir>  -> prints the total size in bytes
  local dir="$1" file total=0
  while IFS= read -r file; do
    total=$((total + $(wc -c < "$file" | tr -d ' ')))
  done < <(find "$dir" -type f)
  printf '%s' "$total"
}

# -- ffmpeg ------------------------------------------------------------------------------------------

# Exactly the four libraries the executables end up importing, and no more. `ffmpeg-next` is taken
# with `default-features = false, features = ["codec", "format", "software-resampling"]`, so avdevice,
# avfilter and swscale are never linked -- and this list is closed under dependency: avformat needs
# avcodec and avutil, avcodec needs avutil and swresample, swresample needs avutil, and none of them
# reaches further. Verified by reading the import tables rather than assumed.
#
# Naming them rather than copying bin/*.dll is worth 29 MiB (avfilter and avdevice alone are 30) and,
# more importantly, keeps a staged folder honest about what it actually links.
DIST_FFMPEG_DLLS=(avcodec-61.dll avformat-61.dll avutil-59.dll swresample-5.dll)

# Where ffmpeg lives, asked of the one thing that knows. Callers resolve this *before* building, so a
# missing install fails in a second with a sentence naming the fix, rather than after a release build
# that then dies in `ffmpeg-sys-next` with pkg-config noise mentioning neither ffmpeg nor clang.
#
# The build itself needs no variable exported: tools/setup/fetch-ffmpeg.sh records FFMPEG_DIR and
# LIBCLANG_PATH in cargo's own `[env]`. What this is for is the *staging*, which has to find the DLLs
# to copy them.
#
# **Windows only, and that is about the DLL names below rather than about copying.** macOS copies
# libraries out of its ffmpeg prefix too now -- see `dist_stage_ffmpeg_macos` -- but it finds that
# prefix through Homebrew rather than through this, which looks for a `bin/avcodec-61.dll` that a Mac
# has no reason to have. On Linux `--print-dir` has
# nothing to print at all, because a distribution ffmpeg is found through pkg-config and
# tools/setup/fetch-ffmpeg.sh deliberately sets no FFMPEG_DIR. Calling this off Windows used to fail either
# way -- on a Mac by demanding `bin/avcodec-61.dll` from a perfectly good Homebrew install and
# blaming the ffmpeg for it -- which is what `dist_ffmpeg_check` below is for.
dist_ffmpeg_dir() { # -> prints the directory, or fails.  Windows only; see dist_ffmpeg_check
  local dir dll
  if ! dir="$(tools/setup/fetch-ffmpeg.sh --print-dir)"; then
    echo "${DIST_SCRIPT:-dist}: video is on by default and needs ffmpeg, which" >&2
    echo "      tools/setup/fetch-ffmpeg.sh could not find." >&2
    echo "      Run tools/setup/fetch-ffmpeg.sh once on this machine, or pass --no-video." >&2
    return 1
  fi
  for dll in "${DIST_FFMPEG_DLLS[@]}"; do
    if [ ! -f "$dir/bin/$dll" ]; then
      echo "${DIST_SCRIPT:-dist}: $dir/bin/$dll is missing -- that is not a shared ffmpeg build." >&2
      echo "      Video needs the *-shared-* build; tools/setup/fetch-ffmpeg.sh fetches one." >&2
      echo "      Or pass --no-video to stage the build that does not read video." >&2
      return 1
    fi
  done
  printf '%s' "$dir"
}

# The same question off Windows, where the answer is not a directory: is there an ffmpeg to build
# against at all? Asked for the same reason and at the same moment -- a second now beats a release
# build that dies in `ffmpeg-sys-next` later -- and asked of the same script, which is what knows how
# each platform answers it: Homebrew's prefix and the Command Line Tools on macOS, pkg-config on
# Linux. `--print` resolves without installing anything and without writing to cargo's config, and it
# dies with the command to run when something is missing, so its own message is the one worth having.
#
# Its output on success is a short build tutorial -- true, and not what a release script's `== video`
# section should be printing -- so it is kept back unless it failed.
dist_ffmpeg_check() { # -> ok, or fails having said what is missing and how to fix it
  local out
  if ! out="$(tools/setup/fetch-ffmpeg.sh --print 2>&1)"; then
    printf '%s\n' "$out" >&2
    echo "${DIST_SCRIPT:-dist}: video is on by default and needs ffmpeg; the line above says what" >&2
    echo "      is missing. Install it, or pass --no-video." >&2
    return 1
  fi
}

# Puts ffmpeg's `bin` on PATH for the rest of the caller's run. Needed because reading `--version` out
# of a freshly built exe *runs* it, and a video build cannot start without its DLLs -- which are not
# staged yet at that point.
#
# Converted back to POSIX form first, and that is not a nicety. FFMPEG_DIR is a *Windows* path
# (`C:/Users/...`) because that is what ffmpeg-sys-next's build script needs, but PATH here is bash's
# own and is separated by colons -- so a Windows path in it is read as a directory `C` followed by
# another called `/Users/...`, and the exe then fails to start on a missing swresample-5.dll while the
# directory holding it is apparently right there on PATH.
dist_ffmpeg_on_path() { # <ffmpeg dir>
  local bin="$1/bin"
  if command -v cygpath >/dev/null 2>&1; then bin="$(cygpath -u "$bin")"; fi
  PATH="$bin:$PATH"
  export PATH
}

# The macOS answer to "where is the ffmpeg I am about to copy libraries out of", and deliberately one
# line of delegation rather than a probe of its own. `tools/setup/fetch-ffmpeg.sh` is what knows the order --
# an FFMPEG_DIR in the environment, then the pinned LGPL build it puts in the cache, then Homebrew --
# and two resolvers that could disagree about which ffmpeg a release was built against is exactly the
# thing worth not having. `--print-dir` resolves without building or writing anything, so a missing
# ffmpeg costs a second here rather than a release build that dies later.
dist_ffmpeg_dir_macos() { # -> prints the prefix, or fails having said what to run
  local dir
  if ! dir="$(tools/setup/fetch-ffmpeg.sh --print-dir)"; then
    echo "${DIST_SCRIPT:-dist}: video is on by default and needs an ffmpeg to copy libraries out" >&2
    echo "      of; the line above says how to get one. Or pass --no-video." >&2
    return 1
  fi
  if [ ! -d "$dir/lib" ]; then
    echo "${DIST_SCRIPT:-dist}: $dir has no lib/ under it -- that is not an ffmpeg prefix." >&2
    return 1
  fi
  printf '%s' "$dir"
}

# Beside the exe, not in a subfolder: the Windows loader searches the directory the executable was
# loaded from, and nowhere else that would help here.
#
# The license travels with the libraries, and this is an obligation rather than a courtesy. These are
# LGPL binaries, redistributed unmodified; shipping them is compliant *because* they are dynamically
# linked and replaceable, and because their terms and their source are named. The build's own
# LICENSE.txt is copied verbatim, and each README says which build it is and where the source is.
dist_stage_ffmpeg() { # <dest dir> <ffmpeg dir>  -> prints the number of DLLs copied
  local dest="$1" dir="$2" dll count=0
  for dll in "${DIST_FFMPEG_DLLS[@]}"; do
    cp "$dir/bin/$dll" "$dest/$dll"
    count=$((count + 1))
  done
  if [ -f "$dir/LICENSE.txt" ]; then
    cp "$dir/LICENSE.txt" "$dest/ffmpeg-LICENSE.txt"
  else
    echo "${DIST_SCRIPT:-dist}: warning -- no LICENSE.txt in $dir, so none was staged." >&2
    echo "      Redistributing LGPL binaries without their terms is not compliant; add it by hand." >&2
  fi
  printf '%s' "$count"
}

# -- the application's own terms -------------------------------------------------------------------

# Copies LICENSE-MIT and LICENSE-APACHE into a staged folder.
#
# **This is an obligation rather than a courtesy, and that is why it is unconditional.** MIT says the
# notice "shall be included in all copies or substantial portions of the Software", so a folder or a
# bundle handed to somebody with only the *words* "MIT OR Apache-2.0" in its README does not satisfy
# the license this workspace chose. Every README already named the pair; until the two files existed
# in the tree there was nothing to name.
#
# Called by every carrier, including the ones that stage no ffmpeg: the terms are about the
# application, not about what it links. The one carrier that does not call it is the `.deb`, which
# has `/usr/share/doc/<pkg>/copyright` and a policy about what goes in it.
dist_stage_app_licenses() { # <dest dir> [prefix for the file names]
  local dest="$1" prefix="${2-}"
  cp LICENSE-MIT "$dest/${prefix}LICENSE-MIT.txt"
  cp LICENSE-APACHE "$dest/${prefix}LICENSE-APACHE.txt"
}

# The paragraph every video build's README ends with. Written once because getting an LGPL notice
# subtly different in three folders is exactly the kind of thing nobody notices until it matters.
#
# The build is named from the directory rather than from the pin in tools/setup/fetch-ffmpeg.sh, because
# FFMPEG_DIR can legitimately point at somebody's own build -- and a license note that confidently
# names the wrong build would be worse than one that names none.
dist_ffmpeg_license_note() { # <ffmpeg dir>
  cat <<NOTE

The avcodec, avformat, avutil and swresample DLLs are FFmpeg, licensed under the GNU Lesser
General Public License. The terms this build is distributed under are in ffmpeg-LICENSE.txt, and
that file is what governs -- FFmpeg is "LGPL v2.1 or later", so the version stated there is the
one that applies here. They are redistributed unmodified, and this program links them dynamically,
so you may replace them with your own build of the same libraries. This is the LGPL configuration
of FFmpeg, not the GPL one, and it contains no GPL components.

The build staged here is $(basename "$1").
That name carries the upstream version and commit, which is what identifies the corresponding
source: FFmpeg's own repository is at https://git.ffmpeg.org/ffmpeg.git, and prebuilt LGPL
copies of it come from https://github.com/BtbN/FFmpeg-Builds/releases
NOTE
}

# -- the README an installed build carries ------------------------------------------------------------

# **An installed build is not a folder you unpacked, and it needed its own document.** Both setup
# programs used to install `dist/bin/<platform>/README.txt` -- the one tools/dist/bin.sh writes for the
# gathered folder -- and on Windows they made it the `Read me first` Start Menu entry, which is to say
# the one document a new user is pointed at. Almost none of it was true once installed: it says the
# folder holds every executable this platform can build (a setup program installs what was ticked),
# it lists all seven programs from a scan of the *staging* folder, and its removal section says to
# delete the folder because "nothing was installed anywhere else and nothing was registered" -- next
# to, on Windows, an uninstaller, a Start Menu group, a PATH entry and a file association, and on
# macOS an uninstaller sitting in the very same folder. See the `What an installed build contains`
# decision in docs/decisions/.
#
# **Here rather than in either installer**, on the same argument as `dist_ffmpeg_license_note` above:
# two setup programs saying subtly different things about how to remove the same product is exactly
# what one copy prevents. The two platforms genuinely differ -- one folder and a Start Menu against
# /Applications, /usr/local/bin and a `.command` you double-click -- so the text branches, and the
# branch is here where both can be read at once.
#
# The folder README is left exactly as it was and goes on being right about the folder; what changed
# is that neither installer ships it any more. Each excludes it by name and says so.
#
# **The `product` argument is which carrier is asking, and it is not a third platform.** The
# remote-only setup programs install one windowed program and configure nothing, so every section of
# the text below is wrong for them -- five components, a songs folder, a PATH entry, an instrument
# bank the remote never plays. Four setup programs describing how to remove the same two products is
# what one copy prevents, and the remote-only pair is the half most likely to drift, being the half
# nobody opens.

dist_installed_readme_remote() { # <platform: windows|macos>
  cat <<'HEAD'
KM Remote
=========

The karaoke remote, with your own copy of the song list. Point it at a karaoke machine once and it
copies the whole catalog onto this computer, so browsing, searching and your favorites work whether
or not the machine is switched on.

HEAD
  case "$1" in
    windows)
      cat <<'BODY'
What was installed
------------------

    km-remote.exe          the remote, which opens a window of its own
    README-km-remote.txt   what it does, how to point it at a machine, every option it takes
    LICENSE-MIT.txt        the terms, both of them; the program is MIT OR Apache-2.0, at your option
    LICENSE-APACHE.txt

The Start Menu has a "KM Remote" entry, and that is how to start it. Nothing was added to your
PATH and no file type was claimed: this installs one program and configures nothing.

The karaoke machine itself is a separate download, along with the tool that turns a folder of songs
into a package and the one that finds pictures and instrument banks. That download carries this
program too, so a computer that plays the songs needs this one only if it never had the other.

Where the song list and your favorites are kept
-----------------------------------------------

    %APPDATA%\km-remote

catalog.sqlite is the copy of a machine's song list, and favorites.sqlite is your folders and the
songs in them. They are separate on purpose: refreshing the list may throw the first away and
rebuild it, and a collection built up over a year must not be able to go with it. Removing or
reinstalling this program touches neither.

Getting rid of it
-----------------

Open "Add or remove programs" from the Start menu, find KM Remote and choose Uninstall -- or use
the "Uninstall KM Remote" entry in the Start menu. Both run the same uninstaller.

Deleting this folder by hand does not. It leaves the KM Remote group in your Start menu pointing at
files that are no longer there. Either way the song list and your favorites are left alone.
BODY
      ;;
    macos)
      cat <<'BODY'
What was installed
------------------

    KM Remote          the remote, in /Applications
    this folder        /usr/local/km-remote -- the terms, this document and the uninstaller

Nothing was placed in /usr/local/bin: this package carries the application alone, and there is no
km-remote command to type. Nothing it placed is quarantined either, so the application opens on a
double-click.

The karaoke machine itself is a separate download, along with the tool that turns a folder of songs
into a package and the one that finds pictures and instrument banks. That download carries this
program too, so a Mac that plays the songs needs this one only if it never had the other.

Getting rid of it
-----------------

Double-click "Uninstall KM Remote.command" in this folder. It shows what it is about to remove,
asks, and only then asks for your password -- in that order. From a terminal,
sudo /usr/local/km-remote/uninstall.sh is the same thing, and --dry-run shows the list on its own
and needs no password.

Dragging KM Remote to the Trash is not the same thing: it leaves this folder and the installer's
receipts behind.

BODY
      # The one folder this product keeps, from the file the conclusion pane and the uninstaller also
      # print. The same argument as the all-in-one's `data-locations.txt` one directory over.
      if [ ! -f tools/platform/macos/pkg-remote/data-locations.txt ]; then
        echo "dist_installed_readme: tools/platform/macos/pkg-remote/data-locations.txt is missing;" >&2
        echo "      the installed README, the conclusion pane and the uninstaller all print it." >&2
        return 1
      fi
      cat tools/platform/macos/pkg-remote/data-locations.txt
      ;;
    *)
      echo "dist_installed_readme: no remote text for platform '$1'" >&2
      return 1
      ;;
  esac
}
dist_installed_readme() { # <platform: windows|macos> [product: all|km-remote]
  local product="${2:-all}"
  if [ "$product" = km-remote ]; then
    dist_installed_readme_remote "$1"
    return
  fi
  if [ "$product" != all ]; then
    echo "dist_installed_readme: no text for product '$product'" >&2
    return 1
  fi

  cat <<'HEAD'
KaraokeMachine
==============

HEAD
  case "$1" in
    windows)
      cat <<'BODY'
This folder holds the programs you chose when you installed them. What is actually in it depends on
what you ticked; there are five things the setup program can install, and running it again is how
you add one you left out:

    KaraokeMachine      plays the songs
    KM Package Builder  turns a folder of songs into a package
    KM Remote           search and queue from this computer
    KM Admin            finds pictures and instrument banks for the machine
    Command-line tools  km-pack, km-lyrics, km-wallpaper-pack

Each one has a README of its own beside this file, named for it -- README-karaokemachine.txt,
README-km-pack.txt and so on. Those are the documents to read: they say what each program is for,
how to run it, and what the libraries beside it are licensed under. They are all installed
whatever you ticked, so they are also how you find out what the other components do.

Those documents describe the portable folder each program is also handed out as, so where one talks
about copying a folder to a USB stick, or about a -console.exe sitting beside it, it is talking
about that and not about this. The setup program installs the windowed form of each program only.

Where your songs go
-------------------

The Start Menu has a "Karaoke songs folder" entry, which opens the folder to put them in. Drop a
.kmpkg package in there and start the machine again.

Your songs, settings and catalog are kept there and not in this folder, so removing or
reinstalling the programs does not touch them.

The one rule about this folder
------------------------------

Keep it together. If you installed the machine, assets/ holds the instrument bank and the
wallpapers and has to stay beside karaokemachine.exe, which looks for it there; the avcodec,
avformat, avutil and swresample DLLs are what read video and have to stay beside the programs that
read them.

Getting rid of it
-----------------

Open "Add or remove programs" from the Start menu, find KaraokeMachine and choose Uninstall -- or
use the "Uninstall KaraokeMachine" entry in the Start menu. Both run the same uninstaller, and the
uninstaller is the only thing that undoes everything setup did.

Deleting this folder by hand does not. It leaves the KaraokeMachine group in your Start menu, this
folder's entry on your PATH, and -- if you let setup make them -- the desktop shortcut and the
association that opens .kmbuild files with the Package Builder, all pointing at files that are no
longer there. Either way your songs, settings and catalog are left alone, and the uninstaller
names the folders they are in on its way out.
BODY
      ;;
    macos)
      cat <<'BODY'
This folder, /usr/local/karaokemachine, holds the command-line tools and the uninstaller. The
applications went to /Applications -- KaraokeMachine, KM Package Builder, KM Remote and KM Admin --
and the commands here are on your PATH through /usr/local/bin, so they can be typed from anywhere.

What is actually here depends on what you ticked. There are five things the installer can place,
and running it again is how you add one you left out:

    KaraokeMachine      plays the songs                         /Applications
    KM Package Builder  turns a folder of songs into a package  /Applications
    KM Remote           search and queue from this Mac          /Applications
    KM Admin            finds pictures and instrument banks     /Applications
    Command-line tools  six names you can type                  here

The six are km-pack, km-lyrics, km-wallpaper-pack, km-package-builder, km-remote and km-admin -- so
the last three are each an application *and* a command, and the command comes with
the tools tick rather than with the application's own. The machine has a command too,
karaokemachine, and that one comes with the application.

Each of the commands has a README of its own beside this file, named for it -- README-km-pack.txt,
README-km-remote.txt and so on. Those are the documents to read: they say what each program is for,
how to run it, and what the libraries beside it are licensed under. They are all installed whatever
you ticked, so they are also how you find out what the other components do.

Those documents describe the folder each command is also handed out as, so where one tells you to
clear a quarantine flag off a bundle, or to keep a folder together on a USB stick, it is talking
about that and not about this. Nothing this installer placed is quarantined: the applications in
/Applications open on a double-click.

The one rule about this folder
------------------------------

Keep it together. lib/ holds the libraries that read video, and every command here looks for them
beside itself -- which is why the entries in /usr/local/bin point back here instead of being
copies. The applications carry their own copies inside them and need nothing from this folder, and
for the same reason nothing should be taken out of an .app either.

Getting rid of it
-----------------

Double-click "Uninstall KaraokeMachine.command" in this folder. It shows what it is about to
remove, asks, and only then asks for your password -- in that order. From a terminal,
sudo /usr/local/karaokemachine/uninstall.sh is the same thing, and --dry-run shows the list on its
own and needs no password.

Dragging the applications to the Trash is not the same thing: it leaves this folder, the entries in
/usr/local/bin and the installer's receipts behind.

Where your songs go
-------------------

The machine starts with nothing in it. Build a package with the Package Builder or with km-pack,
then drop the .kmpkg into the packages folder named below.

BODY
      # `cat` and not a @DATA_LOCATIONS@ substitution: the uninstaller takes the marker route because
      # it is a committed template, and that route needs a "occurs exactly once" guard because a
      # replacement over a whole file once dropped ten lines of prose into a `#` comment. There is no
      # template here and nothing to count.
      if [ ! -f tools/platform/macos/pkg/data-locations.txt ]; then
        echo "dist_installed_readme: tools/platform/macos/pkg/data-locations.txt is missing; the" >&2
        echo "      installed README, the conclusion pane and the uninstaller all print it." >&2
        return 1
      fi
      cat tools/platform/macos/pkg/data-locations.txt
      ;;
    *)
      echo "dist_installed_readme: no text for platform '$1'" >&2
      return 1
      ;;
  esac

  # **Outside the case, so both carriers say it in the same words.** The same argument the macOS body
  # takes `data-locations.txt` from a file for: two setup programs describing one behavior
  # differently is what one copy prevents, and this one is a several-minute event on somebody's first
  # start. Printed whether or not the box was ticked, because this text is generated when the
  # installer is built and the tick happens later.
  cat <<'TAIL'

The instrument bank
-------------------

If you left "download the recommended instrument bank" ticked when you installed this,
KaraokeMachine fetches it the first time you start it, and plays it instead of the smaller bank that
came in the box. It says so on the screen while it runs and needs no help from you. If it cannot --
no network yet, most likely -- it tries again on the next two starts and then stops asking, and the
machine plays the bundled bank in the meantime, which works perfectly well.

Nothing else is ever downloaded on your behalf: no songs, no updates.
TAIL
}

# -- ffmpeg on macOS: copied in, and every load command rewritten -------------------------------------

# macOS resolves a dynamic library by the path recorded in the load command, and Homebrew records an
# absolute one -- `/opt/homebrew/opt/ffmpeg/lib/libavcodec.62.dylib`. A binary linked that way plays
# video on the machine that built it and nowhere else, which is what these two functions exist to end.
# The Windows equivalent is `dist_stage_ffmpeg` above and it is four lines, because the Windows loader
# searches the executable's own directory and nothing has to be rewritten at all.
#
# **The closure is walked, not listed.** `km-video` links four ffmpeg libraries, but those four pull in
# nine more of their own -- x264, x265, SVT-AV1, libvpx, dav1d, LAME, Opus and OpenSSL's two, 33 MB in
# all against this Homebrew. Starting from the binary and following every non-system load command means
# the set staged is exactly the set actually needed, and that an FFMPEG_DIR pointing at a different
# build -- an LGPL one, say, with no x264 or x265 in it -- stages a different and smaller set with no
# change here. A hard-coded list would have to be revised every time either of those changed.

# The non-system libraries a Mach-O loads, one per line. `/usr/lib` and `/System` are the OS's own and
# are present on every Mac by definition, so they are left exactly as they are.
#
# A dylib's own id is the first entry `otool -L` prints and is not a dependency, so it is taken from
# `otool -D` and dropped. Left in, it would be harmless -- it is deduplicated by name a moment later,
# and `install_name_tool -change` ignores a path it cannot find -- but it would make the count wrong.
dist_macho_deps() { # <mach-o> -> prints the non-system install names it loads, one per line
  local file="$1" id path
  id="$(otool -D "$file" 2>/dev/null | sed -n '2p')"
  otool -L "$file" | tail -n +2 | awk '{print $1}' | while read -r path; do
    [ -n "$path" ] || continue
    [ "$path" = "$id" ] && continue
    case "$path" in
      /usr/lib/*|/System/*) continue ;;
    esac
    printf '%s\n' "$path"
  done
}

# Copies the whole closure beside the binary and points everything at `@rpath`.
#
# Keyed by the **install-name basename** rather than the filename on disk, and the two differ: the load
# command says `libavcodec.62.dylib` while the file in the Cellar is `libavcodec.62.28.101.dylib`. The
# load command is what has to be satisfied at run time, so it is the name the copy takes. (`cp` follows
# the symlink chain, so the copy is the real library either way.)
#
# **Every file touched is re-signed, and that is not optional.** `install_name_tool` invalidates the
# ad-hoc signature these libraries carry, and on Apple silicon dyld refuses to load a Mach-O whose
# signature does not match -- so a bundle that skipped this would stage perfectly and then fail at
# launch, on the user's machine rather than here.
dist_stage_ffmpeg_macos() { # <mach-o> <lib dest dir> <rpath> -> prints the number of dylibs staged
  local bin="$1" dest="$2" rpath="$3"
  local pending seen dep name count=0

  mkdir -p "$dest"
  seen=""
  pending="$(dist_macho_deps "$bin")"

  while [ -n "$pending" ]; do
    dep="$(printf '%s\n' "$pending" | head -n 1)"
    pending="$(printf '%s\n' "$pending" | tail -n +2)"
    [ -n "$dep" ] || continue
    name="${dep##*/}"
    case " $seen " in *" $name "*) continue ;; esac
    if [ ! -f "$dep" ]; then
      echo "${DIST_SCRIPT:-dist}: a load command names $dep, which is not a file." >&2
      echo "      A load command starting with @rpath or @loader_path cannot be resolved from" >&2
      echo "      here; this expects the absolute paths Homebrew records." >&2
      return 1
    fi
    seen="$seen $name"
    cp "$dep" "$dest/$name"
    # Homebrew leaves them read-only, and install_name_tool writes in place.
    chmod u+w "$dest/$name"
    # Stripped before it is edited rather than invalidated by editing. Both end with the same ad-hoc
    # signature applied below, but this way install_name_tool has nothing to warn about -- and it warns
    # once per change, which is forty lines of "will invalidate the code signature" across this set.
    codesign --remove-signature "$dest/$name" 2>/dev/null || true
    count=$((count + 1))
    # Read the copy rather than the original: same bytes, and it is the file about to be rewritten.
    pending="$(printf '%s\n%s\n' "$pending" "$(dist_macho_deps "$dest/$name")" | grep -v '^$' || true)"
  done

  # Rewritten only once the walk is finished, so that every dependency read above was still the
  # absolute path it was linked as.
  for name in $seen; do
    install_name_tool -id "@rpath/$name" "$dest/$name"
    for dep in $(dist_macho_deps "$dest/$name"); do
      install_name_tool -change "$dep" "@rpath/${dep##*/}" "$dest/$name"
    done
    dist_codesign "$dest/$name" 2>/dev/null
  done


  codesign --remove-signature "$bin" 2>/dev/null || true
  for dep in $(dist_macho_deps "$bin"); do
    install_name_tool -change "$dep" "@rpath/${dep##*/}" "$bin"
  done
  # Only if it is not already there: adding the same rpath twice is an error, and re-staging over an
  # existing output is the ordinary case.
  if ! otool -l "$bin" | grep -q "path $rpath "; then
    install_name_tool -add_rpath "$rpath" "$bin"
  fi
  # Not silenced: a signature that failed to apply is exactly the failure this whole function exists
  # to avoid, and it is invisible until dyld refuses the library on somebody else's Mac.
  dist_codesign "$bin"

  printf '%s' "$count"
}

# Fails, loudly, if anything staged still names a path that will not exist on another machine.
#
# This is the check the whole exercise is for, and it is cheap enough to run every time rather than
# leaving it to whoever first tries the build on a second Mac. `codesign --verify` is here for the
# same reason: a signature broken by `install_name_tool` is invisible until dyld refuses the library.
#
# **On a signed build it checks provenance as well as integrity, and the difference matters.** A bare
# `--verify` only asks whether the seal is intact, which an ad-hoc signature satisfies perfectly -- so
# on its own it cannot tell a Developer ID build from an ad-hoc one, and would pass a bundle where one
# of the four signing sites had been missed. With an identity set it evaluates a designated
# requirement naming the team, which is the thing notarization will check five minutes later and
# refuse without saying which file was wrong.
# **Nothing here pipes into `grep -q`, and that is a fix rather than a style.** Every script that
# calls this runs under `set -o pipefail`, and `grep -q` exits the moment it matches -- which sends
# SIGPIPE to whatever is upstream, so the pipeline's status becomes 141 and the `if` reads it as *no
# match*. For the absolute-path check that is the dangerous direction: a binary that really did load
# from /opt would be waved through. It survives today only because `otool -L` prints a handful of
# lines and usually finishes before grep exits -- a race, and one that gets likelier against
# Homebrew's thirteen-dylib closure than against the pinned build's four. Output is captured first
# and matched afterwards, so no producer is ever killed early. Found by a signing check that failed
# on a file which was, on inspection, signed perfectly well.
dist_verify_macho_portable() { # <dir> -> ok, or fails naming the file and the path
  local dir="$1" file bad=0 team absolute info
  while IFS= read -r file; do
    # A directory holding no Mach-O reaches the loop as one empty line, because that is what a
    # heredoc carrying an empty command substitution is. A bundle whose executable is a launch
    # script is exactly that directory, and it passes this check by having nothing to walk.
    [ -n "$file" ] || continue
    absolute="$(otool -L "$file" | tail -n +2 | grep -E '^[[:space:]]+(/opt/|/usr/local/)' || true)"
    if [ -n "$absolute" ]; then
      echo "${DIST_SCRIPT:-dist}: $file still loads a library by absolute path:" >&2
      printf '%s\n' "$absolute" >&2
      bad=1
    fi
    if ! codesign --verify "$file" 2>/dev/null; then
      echo "${DIST_SCRIPT:-dist}: $file has no valid signature; dyld will refuse to load it." >&2
      bad=1
    elif dist_signing; then
      team="$(dist_team_id)"
      if ! codesign --verify --strict \
             -R="anchor apple generic and certificate leaf[subject.OU] = \"$team\"" \
             "$file" 2>/dev/null; then
        echo "${DIST_SCRIPT:-dist}: $file is not signed by $team." >&2
        echo "      This build set KM_SIGN_IDENTITY, so every Mach-O in it should be. Notarization" >&2
        echo "      would refuse it without naming the file." >&2
        bad=1
      fi
      info="$(codesign -dv "$file" 2>&1)"
      case "$info" in
        *"(runtime)"*) ;;
        *)
          echo "${DIST_SCRIPT:-dist}: $file is signed but has no hardened runtime." >&2
          echo "      Notarization requires it on every Mach-O, not only on the bundle." >&2
          bad=1 ;;
      esac
    fi
  done <<EOF
$(find "$dir" -type f \( -name '*.dylib' -o -perm -u+x \) -exec sh -c 'file -b "$1" | grep -q Mach-O && echo "$1"' _ {} \;)
EOF
  [ "$bad" -eq 0 ]
}

# The macOS counterpart of `dist_ffmpeg_license_note`, and deliberately not a call to it: that note
# ends "This is the LGPL configuration of FFmpeg, not the GPL one, and it contains no GPL components",
# which is true of the BtbN build Windows fetches and **false** of Homebrew's, which is configured
# `--enable-gpl --enable-version3` and links x264 and x265.
#
# So this one reports rather than asserts, and what it reports is derived from the libraries actually
# sitting in the staged folder -- the only thing that can be right for every FFMPEG_DIR somebody points
# at it. The terms travel with the libraries, the same rule as the Windows one: whichever COPYING files
# the ffmpeg prefix carries are copied in beside them.
# The texts go somewhere of their own rather than in with the libraries, and on a bundle that is not a
# preference: `codesign` treats Contents/Frameworks as code and refuses to seal a bundle with a text
# file sitting in it. (Found the hard way -- the seal was computed and then broken by a COPYING file
# landing beside the dylibs a moment later.)
dist_stage_ffmpeg_license_macos() { # <ffmpeg dir> <lib dir> <text dir> -- copies the terms, prints the note
  local dir="$1" libdir="$2" textdir="$3" gpl="" lib copied=0

  for lib in libx264 libx265 libxvidcore; do
    if ls "$libdir" 2>/dev/null | grep -q "^$lib\."; then gpl="$gpl ${lib#lib}"; fi
  done
  # " x264 x265" -> "x264 and x265". It is read as a sentence, so it should be one.
  gpl="$(printf '%s' "$gpl" | sed 's/^ //; s/ \([^ ]*\)$/ and \1/')"

  mkdir -p "$textdir"
  for lib in "$dir"/COPYING.* "$dir"/LICENSE.md; do
    if [ -f "$lib" ]; then cp "$lib" "$textdir/"; copied=$((copied + 1)); fi
  done
  if [ "$copied" -eq 0 ]; then
    echo "${DIST_SCRIPT:-dist}: warning -- no COPYING or LICENSE file in $dir, so none was staged." >&2
    echo "      Redistributing these binaries without their terms is not compliant; add them by hand." >&2
  fi

  cat <<NOTE

FFmpeg
------
This build bundles the FFmpeg libraries it links, and whatever those in turn depend on, so that it
plays video on a machine with no ffmpeg installed. These are all of them, and the COPYING files
staged beside this note are the terms they are under:

$(ls "$libdir" | grep '\.dylib$' | sed 's/^/  /')

They are redistributed unmodified and are linked dynamically, so you may replace any of them with your
own build of the same library -- which is the condition attached to distributing them this way.
NOTE

  if [ -n "$gpl" ]; then
    cat <<NOTE

**This is FFmpeg's GPL configuration, not its LGPL one.** It includes $gpl, which are GPL-licensed,
and an FFmpeg configured with them is itself GPL rather than LGPL -- so what covers this as a whole is
the GNU General Public License, and COPYING.GPLv2 beside this note is the text of it. Homebrew's
ffmpeg is built this way, with --enable-gpl and --enable-version3.

To ship under the LGPL instead, run tools/setup/fetch-ffmpeg.sh without --homebrew: it builds a pinned
FFmpeg configured --disable-gpl, and this staging step then copies neither of these libraries in,
because it follows what it finds rather than a list.

Sources: FFmpeg is at https://git.ffmpeg.org/ffmpeg.git, x264 at
https://code.videolan.org/videolan/x264, x265 at https://bitbucket.org/multicoreware/x265_git.
NOTE
  else
    cat <<NOTE

This is FFmpeg's LGPL configuration: it carries no GPL-licensed component, which is why there are
four libraries here and not thirteen. It is built from FFmpeg's own source release by
tools/setup/fetch-ffmpeg.sh, configured --disable-gpl --disable-nonfree --disable-version3, with only the
decoders this application uses -- it never encodes through these libraries. The terms are LGPL v2.1
or later; COPYING.LGPLv2.1 beside this note is the text.

Source: FFmpeg's own repository, https://git.ffmpeg.org/ffmpeg.git. The version staged here is named
in the library filenames, and the exact release and its checksum are pinned in tools/setup/fetch-ffmpeg.sh.
NOTE
  fi
}

# -- the macOS application bundle ---------------------------------------------------------------------

# A `.app` is a directory with a fixed shape, and three things now want one: the machine
# (`tools/platform/macos/app-bundle.sh`) and `km-package-builder` (`tools/dist/cmd.sh`), with the offline
# remote to follow. What differs between them is the plist, the executable and the icon; what does not
# is everything below, which is why it is here rather than copied.
#
# **Why a bundle at all.** A bare Mach-O on macOS is a terminal program: no icon in the Dock or the
# Finder, not double-clickable out of a download, and no Info.plist -- so nothing names the window and
# there is no `CFBundleIconFile` to draw. It is also the only place on that platform an icon can live.
#
# Contents/MacOS is for executables **only**, and Contents/Resources for everything the program reads.
# That is Apple's layout rather than a preference, and a bundle that ignores it works right up until
# something tries to sign it.

# Answers whether this process may write inside a bundle that is already there.
#
# **macOS reads a signed `.app` as an application rather than as a folder**, and writing inside one
# is App Management permission, held per program. The program it is read against is the one the
# terminal belongs to -- an editor's integrated terminal carries the editor's grant rather than
# Terminal's -- so the same command goes through in one window and is refused in another. Nothing
# prompts for it: the request is denied where it stands and `mkdir` reports `Operation not
# permitted`, which names a file and not the permission behind it.
#
# **Asked before anything is removed**, for the reason the icon is checked early below: `dist_clear`
# empties the bundle it is about to rebuild, so a refusal arriving after it leaves neither the old
# bundle nor a new one. The probe is a directory made and dropped at the depth the staging writes
# at, because what is being asked about belongs to the bundle rather than to the folder holding it.
#
# A path that is not there yet is nobody's application. There is nothing to ask about, and a refusal
# there costs nothing, so it is left to the staging.
dist_macos_bundle_writable() { # <app dir>
  local app="$1" dir probe
  [ -d "$app" ] || return 0

  dir="$app"
  if [ -d "$app/Contents" ]; then
    dir="$app/Contents"
  fi
  probe="$dir/.km-write-probe.$$"

  if mkdir "$probe" 2>/dev/null; then
    rmdir "$probe" 2>/dev/null || true
    return 0
  fi

  echo "${DIST_SCRIPT:-dist}: cannot write inside $app." >&2
  echo "      macOS asks for App Management permission before a program writes into a signed" >&2
  echo "      application bundle, and refuses one that does not hold it rather than prompting." >&2
  echo "      Grant it to the application this terminal belongs to, under System Settings," >&2
  echo "      Privacy & Security, App Management, then quit that application and open it again." >&2
  return 1
}

# Stages the fixed parts: the executable, the icon, the manifest and the type code.
#
# **The icon is copied under its own basename, and that is a contract with the plist**: macOS reads
# `CFBundleIconFile` and looks for `<that>.icns` in Contents/Resources, so the two have to agree. A
# mismatch is not an error anywhere -- it draws a generic icon and says nothing -- which is how the
# builder's plist spent two milestones naming a file it was never given.
#
# Nothing is signed here. The seal comes last, after every caller has staged what it carries, and
# `dist_seal_macos_bundle` below is that step.
dist_stage_macos_bundle() { # <app dir> <plist template> <exe src> <exe name> <icns src> <version>
  local app="$1" plist="$2" exe="$3" name="$4" icns="$5" version="$6"
  local contents="$app/Contents"

  # Checked before anything is cleared, so a missing icon costs nothing: `dist_clear` empties the
  # bundle that is already there, and failing after that would leave neither the old one nor a new one.
  if [ ! -f "$icns" ]; then
    echo "${DIST_SCRIPT:-dist}: $icns is missing." >&2
    echo "      Regenerate it: cargo run -p km-display --example icon" >&2
    return 1
  fi
  if [ ! -f "$plist" ]; then
    echo "${DIST_SCRIPT:-dist}: $plist is missing." >&2
    return 1
  fi

  # Ahead of the fossil sweep as well as of the clear, so that a refusal takes nothing with it.
  dist_macos_bundle_writable "$app" || return 1

  # **Any other bundle in this folder is this one under a name it no longer has, and it is taken
  # now.** A product's platform folder holds exactly one `.app` -- its own -- so a second can only be
  # a fossil of a rename. `dist_clear` below empties the bundle it is about to write and cannot see a
  # sibling, and nothing downstream collects one either: `tools/dist/clean.sh --old` matches a name
  # followed by a version that is not wanted, a bundle carries no version by
  # `tools/platform/macos/app-bundle.sh`'s own decision, and so `--all` was the only thing that could
  # ever take one.
  #
  # Left alone it does not sit still. `tools/dist/bin.sh` gathers bundles by globbing `*.app` here,
  # so a fossil rode into `dist/bin/<platform>` on every run and the setup program refused the
  # payload -- `KaraokeMachine Package Builder.app, which no component claims`. That check was right
  # and could not say the bundle was simply old. It also surfaced nowhere near the rename that made
  # it: three bundles were renamed on a machine with no `dist/` to fossilize, and the failure arrived
  # a day later on one that had.
  #
  # Derived rather than named -- nothing here lists a dead name, so the next rename needs no edit.
  # Compared by basename because the two callers spell the path differently.
  #
  # **`DIST_MACOS_ALSO_STAGES` is how a product says it has more than one**, and the machine is the
  # one that does: the second bundle is the same program started a different way, and without this it
  # would be swept as a fossil by whichever of the two was staged first. Anything not named there is
  # still a fossil and still goes, which is the whole of what this guard is for.
  #
  # An array rather than a string, because `KM Package Builder.app` and `KM Stream.app`
  # both have spaces in them and a split list would sweep bundles it had been told to keep.
  local sibling keep
  for sibling in "$(dirname "$app")"/*.app; do
    [ -d "$sibling" ] || continue
    [ "$(basename "$sibling")" != "$(basename "$app")" ] || continue
    for keep in ${DIST_MACOS_ALSO_STAGES[@]+"${DIST_MACOS_ALSO_STAGES[@]}"}; do
      [ "$(basename "$sibling")" != "$keep" ] || continue 2
    done
    echo "${DIST_SCRIPT:-dist}: removed $(basename "$sibling") -- this folder's bundle is $(basename "$app")"
    rm -rf "$sibling"
  done

  dist_clear "$app"
  mkdir -p "$contents/MacOS" "$contents/Resources"

  cp "$exe" "$contents/MacOS/$name"
  chmod 755 "$contents/MacOS/$name"

  cp "$icns" "$contents/Resources/$(basename "$icns")"

  sed "s/@VERSION@/$version/g" "$plist" > "$contents/Info.plist"

  # Eight bytes that predate Info.plist and that the Finder still reads. Cheap, and its absence makes
  # some tools report the bundle as an unknown kind.
  printf 'APPL????' > "$contents/PkgInfo"
}

# Seals the bundle and proves it will load somewhere else.
#
# **Call this last.** `codesign` walks Contents and seals what it finds, so any file staged after it
# invalidates the signature -- which is not hypothetical: the ffmpeg license texts did exactly that
# when they were first written in after the binary had been signed.
#
# **Which identity depends on `KM_SIGN_IDENTITY`, and the default is still ad-hoc.** This comment used
# to say the seal was ad-hoc because "signing properly needs a paid Apple Developer account, which is
# a purchase rather than a build step" -- a sound argument whose premise stopped being true. It is
# kept here rather than deleted because the *default* is unchanged and the reasoning still explains
# it: a fresh clone has no certificates and must still be able to stage a build. What changed is that
# a machine that does have one can now say so, and every caller's report says which it got.
#
# The two checks afterwards are the point of the exercise. An absolute load path left behind, or a
# signature broken by the rewriting, are both invisible until somebody opens the bundle on another Mac.
dist_seal_macos_bundle() { # <app dir>
  local app="$1"
  dist_codesign "$app"
  dist_verify_macho_portable "$app"
  codesign --verify --deep "$app"
  # The Finder caches icons per bundle path aggressively, and a rebuilt bundle at the same path very
  # often keeps showing the previous one. Touching the bundle is the documented nudge.
  touch "$app"
}

# `ditto` rather than `dist_zip`, and this is the reason that function refuses the job: a plain zip
# flattens the resource forks and symlinks a bundle is made of, so what comes out the other end is a
# folder rather than an application.
dist_zip_macos_bundle() { # <app dir> <zip path>
  local app="$1" zip="$2"
  rm -f "$zip"
  ditto -c -k --keepParent "$app" "$zip"
  printf 'zipped %s (%s bytes)\n' "$zip" "$(wc -c < "$zip" | tr -d ' ')"
}

# -- iOS ---------------------------------------------------------------------------------------------

# An `.ipa` around an unsigned app bundle: a zip holding `Payload/<name>.app` and nothing else.
#
# **There is no `xcodebuild -exportArchive` path to this.** Every export method Apple defines signs,
# and each needs a certificate and a provisioning profile naming the devices allowed to run the
# result -- so an export turns a packaging step into a credentials check and produces a file that
# installs for its author and nobody else. What ships instead is unsigned, and the person installing
# it signs it with their own Apple ID.
#
# `ditto` rather than `zip`, for the reason `dist_zip_macos_bundle` gives: a bundle carries symlinks
# and a plain zip flattens them.
dist_ipa() { # <app bundle> <ipa path>
  local app="$1" ipa="$2" parent stage
  parent="$(dirname "$ipa")"
  stage="$parent/Payload"
  rm -rf "$stage"
  rm -f "$ipa"
  mkdir -p "$stage"
  ditto "$app" "$stage/$(basename "$app")"
  ( cd "$parent" && ditto -c -k --sequesterRsrc --keepParent Payload "$(basename "$ipa")" )
  rm -rf "$stage"
}

# The version the bundle says it is. **`CFBundleShortVersionString` is a third copy of the product
# version**, written by hand in each `project.yml`, and `tools/dev/check-version-pin.sh` reconciles
# the two Cargo manifests and not this one. A carrier named from the manifest around a bundle that
# says something else is the one moment that gap costs anything, so the two are compared here.
dist_bundle_version() { # <app bundle>  -> prints the version, or fails
  local app="$1" version
  version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Info.plist" 2>/dev/null || true)"
  if [ -z "$version" ]; then
    echo "${DIST_SCRIPT:-dist}: no CFBundleShortVersionString in $app/Info.plist" >&2
    return 1
  fi
  printf '%s' "$version"
}

# -- the zip ---------------------------------------------------------------------------------------

# `zip` first, PowerShell second: `zip` exists on macOS and Linux, and PowerShell is what a stock
# Windows box has instead. Neither being present leaves the folder in place and says so -- the folder
# is the deliverable, the zip is a convenience.
#
# `tools/platform/macos/app-bundle.sh` deliberately does not call this: a `.app` needs `ditto`, which preserves
# the resource forks and symlinks that a plain zip flattens.
dist_zip() { # <parent dir> <folder name>
  local parent="$1" name="$2"
  if command -v zip >/dev/null 2>&1; then
    ( cd "$parent" && zip -qr "$name.zip" "$name" )
  elif command -v powershell >/dev/null 2>&1; then
    powershell -NoProfile -Command \
      "Compress-Archive -Force -Path '$parent/$name' -DestinationPath '$parent/$name.zip'"
  else
    echo "note: neither zip nor powershell is on PATH, so no archive was made. The folder above is"
    echo "      complete; compress it by hand."
    return 0
  fi
  printf 'zipped %s/%s.zip (%s bytes)\n' "$parent" "$name" "$(wc -c < "$parent/$name.zip" | tr -d ' ')"
}

# The same zip, for a folder whose own name is not the name the archive should carry.
#
# **A zip's top-level entry is a directory name somebody is going to be looking at**, and that is the
# whole reason this exists rather than a `mv` after the fact. `tools/dist/bin.sh` stages into
# `dist/bin/windows`, because `dist/bin/<platform>` is what that carrier is *called*; an archive of it
# has to unpack to something that names the product and the version, not to a folder called `windows`
# sitting in somebody's Downloads. Renaming the .zip afterwards does not fix that -- the bad name is
# inside the file.
#
# So the folder is renamed, zipped under the name it should carry, and renamed back. Both renames are
# within one directory and therefore instantaneous, and the one back is in a `trap` so that a failed
# zip, or a Ctrl-C in the middle of one, cannot leave the folder under a name nothing else looks for.
# The trap is cleared on the way out rather than left set, because this is a sourced library and the
# caller's own EXIT handling is not ours to keep.
#
# **On macOS it goes through `ditto`, not `zip`**, and that is the same reason `dist_zip` refuses a
# `.app` outright: a plain zip flattens the symlinks and resource forks a bundle is made of, so what
# comes out is a folder rather than an application. This function's caller stages *folders containing*
# bundles, which is the case neither of the two existing helpers covered -- `dist_zip` would break the
# bundle and `dist_zip_macos_bundle` expects the bundle itself. Choosing on `ditto` being present
# rather than on a platform variable keeps it to one line and cannot disagree with the host.
dist_zip_as() { # <staged dir> <archive path, without .zip>
  local dir="$1" archive="$2" parent name tmp
  parent="$(dirname "$archive")"
  name="$(basename "$archive")"
  tmp="$parent/$name"
  _dist_zip_here() {
    if command -v ditto >/dev/null 2>&1; then
      dist_zip_macos_bundle "$parent/$name" "$parent/$name.zip"
    else
      dist_zip "$parent" "$name"
    fi
  }
  if [ "$dir" = "$tmp" ]; then _dist_zip_here; return; fi
  if [ -e "$tmp" ]; then
    echo "${DIST_SCRIPT:-dist}: $tmp is in the way of zipping $dir." >&2
    return 1
  fi
  rm -f "$archive.zip"
  mv "$dir" "$tmp"
  # shellcheck disable=SC2064
  trap "mv '$tmp' '$dir' 2>/dev/null || true" EXIT
  _dist_zip_here
  mv "$tmp" "$dir"
  trap - EXIT
}

# -- Docker, for the Linux scripts -------------------------------------------------------------------

# The host side of a bind mount has to be in the form the daemon understands: `C:/code/...` on
# Windows, the path as-is everywhere else. cygpath exists only under MSYS/Cygwin, which is exactly the
# case that needs it.
#
# Its partner is `export MSYS2_ARG_CONV_EXCL='*'`, which each caller sets for itself because it has to
# happen before any Windows executable is invoked: MSYS (Git Bash) rewrites anything that looks like a
# Unix path before handing it over, which turns `/src` into `C:/Program Files/Git/src` and mounts the
# wrong thing entirely.
#
# `tools/platform/linux/check.sh` keeps its own copy of this. It stages nothing and writes nothing under
# `dist/`, so sourcing a staging library for one three-line function would be the tail wagging the dog.
host_path() { # <path>
  if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}
