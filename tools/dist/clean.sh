#!/usr/bin/env bash
#
# Takes staged releases away again.
#
#   tools/dist/clean.sh --all              # every staged release under dist/
#   tools/dist/clean.sh --old              # only the ones that are not this workspace's version
#   tools/dist/clean.sh --old --dry-run    # ...say what that would remove, remove nothing
#
# The counterpart of the five staging scripts, and it exists as a script rather than as a few lines
# in Taskfile.yml for a reason that is not style: Task runs its commands through an embedded POSIX
# shell, which gives it `for`, `case` and parameter expansion on every platform but **no `rm`** --
# that is an external command, and there is no `rm.exe` on the Windows box this is mostly developed
# on, any more than there is a `sed` or a `grep`. So the Taskfile chooses which script to run, which
# is what it does for staging too, and the deleting happens here where coreutils exist.
#
# It also means `task` stays optional. Nothing in this repository requires it, and this is the
# command the Taskfile's `clean` and `clean:old` are.
#
# **The version is read from the manifest here, not from the artifact.** That is the opposite of
# `dist_version()` in tools/dist/common.sh, deliberately: its method is to run the executable and
# read its `--version`, and half of what this walks is a `.deb` or a `.tar.gz` with nothing to run.
# The two cannot disagree, because a clap `version` is CARGO_PKG_VERSION and that is what `cargo
# pkgid` prints.

set -euo pipefail

cd "$(dirname "$0")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

# **`tools/dist/common.sh` is deliberately not sourced here**, though every staging script does. Its
# helpers all *build* the layout -- name a folder, clear it, stage assets into it, total its bytes --
# and this script is the only one that takes the layout apart, which it does by walking `dist/` and
# reading the names rather than by constructing any. Sourcing it to use nothing from it would suggest
# a coupling that is not there. The coupling that *is* there is the rule itself: `dist_dir()` in that
# file owns `dist/<app>/<platform>/<app>-<version>-<triple>`, and if it changes, this changes.

MODE=""
DRY=0
for arg in "$@"; do
  case "$arg" in
    --all) MODE=all ;;
    --old) MODE=old ;;
    --dry-run|-n) DRY=1 ;;
    -h|--help)
      echo "usage: tools/dist/clean.sh (--all | --old) [--dry-run]"
      exit 0
      ;;
    *) echo "dist-clean: unknown option $arg" >&2; exit 2 ;;
  esac
done

if [ -z "$MODE" ]; then
  echo "dist-clean: say which -- --all (everything) or --old (everything but the current version)" >&2
  exit 2
fi

if [ ! -d dist ]; then
  echo "dist-clean: nothing to clean -- dist/ does not exist"
  exit 0
fi

REMOVED=0

take() { # <path>
  if [ "$DRY" -eq 1 ]; then
    echo "  would remove $1"
  else
    echo "  removing $1"
    rm -rf "$1"
  fi
  REMOVED=$((REMOVED + 1))
}

# -- everything ----------------------------------------------------------------------------------

if [ "$MODE" = all ]; then
  for d in dist/*/; do
    [ -d "$d" ] || continue
    take "${d%/}"
  done
  # Only if it came out empty. A stray file somebody put there is theirs, not ours to guess about.
  [ "$DRY" -eq 1 ] || rmdir dist 2>/dev/null || true
  echo "dist-clean: $REMOVED staged app folder(s)"
  exit 0
fi

# -- everything but the current version ------------------------------------------------------------

# Two strips, because `cargo pkgid` has two output shapes: `...#karaokemachine@1.2.0` when the package
# name differs from its directory, and `...#1.2.0` when it does not. `##*@` is a no-op on the second.
pkg_version() { # <cargo pkgid arguments>  -> prints the version
  local p
  p="$(cargo pkgid "$@")"
  p="${p##*#}"
  printf '%s' "${p##*@}"
}

# **One number, for every product under `dist/`.** `km-wallpaper-pack` and `km-admin` are in the
# excluded workspace under `tools/cmd/assets` and follow the machine's number like everything else,
# which is what makes this a variable rather than three and a `case`. Holding a program on a number
# of its own to the machine's would delete the current build of it every time this ran while
# reporting it had removed something stale, which is the one mistake a cleaner must not make.
VERSION="$(pkg_version -p karaokemachine)"

# An entry is removed only when its name **begins with its app's own name followed by a version that
# is not the wanted one**. Anything unrecognized is left alone rather than guessed at, which is what
# makes two things safe without special-casing either:
#
#   - `KaraokeMachine.app` carries no version at all, by tools/platform/macos/app-bundle.sh's own decision --
#     the number is in Info.plist. It does not begin with the app's name either, so it is never
#     matched and `--old` can never take a macOS bundle. `--all` is what removes one. The same now
#     goes for `KM Package Builder.app`, which tools/dist/cmd.sh stages beside that tool's
#     folder: a second bundle needed no change here, which is the property this rule was written for.
#     **And the same again for `dist/bin/<platform>` and `dist/bin-console/<platform>`**, which
#     tools/dist/bin.sh stages under a versionless name on purpose -- a folder is where you keep the
#     current build, and the number goes on the archive you hand over. So `--old` can never take one
#     of those either; their *zips* it can, and does, at the bottom of this file.
#   - the contents of a folder that is being kept -- `assets`, `karaokemachine-console.exe`,
#     `avcodec-61.dll` -- none of which parse as a version.
#
# **Two things this therefore cannot see, and only one of them is handled elsewhere.**
#
#   - A bundle left under a name a rename took away -- `KaraokeMachine Package Builder.app` beside
#     `KM Package Builder.app`. It carries no version, so nothing above matches it, and
#     tools/dist/bin.sh then globs both into `dist/bin/<platform>` and the setup program refuses the
#     payload. **`dist_stage_macos_bundle` takes it now**, at the moment the surviving name is
#     staged, which is the only place that can tell a fossil from a bundle without being told a list
#     of dead names.
#   - A whole product folder for a program that no longer exists -- `dist/km-assets/`,
#     `dist/wallpaper-pack/`. Nothing stages into one any more, so no staging run can clean it, and
#     the version inside it is current, so `--old` reads it as this build. Knowing it is dead means
#     knowing which products exist, which is a list this script deliberately does not keep -- see the
#     note above about not sourcing tools/dist/common.sh. So it is `--all`, or `rm -rf` by hand, and
#     saying so here is the whole of the fix.
#
# The two separators are both handled because both occur: a folder is `<app>-<version>-<triple>` and
# a Debian package is `<app>_<version>-1_amd64.deb`.
sweep() { # <app> <wanted version> <path>...
  local app="$1" want="$2" e base rest v
  shift 2
  for e in "$@"; do
    [ -e "$e" ] || continue
    base="$(basename "$e")"
    rest="${base#"$app"}"
    [ "$rest" != "$base" ] || continue
    rest="${rest#-}"
    rest="${rest#_}"
    v="${rest%%-*}"
    case "$v" in [0-9]*.[0-9]*) ;; *) continue ;; esac
    [ "$v" != "$want" ] || continue
    take "$e"
  done
}

for appdir in dist/*/; do
  [ -d "$appdir" ] || continue
  app="$(basename "$appdir")"
  for platdir in "$appdir"*/; do
    [ -d "$platdir" ] || continue
    sweep "$app" "$VERSION" "$platdir"*
    # tools/platform/linux/deb.sh puts a --no-video package one level down, because both builds produce a file
    # of exactly the same name. It is the only nested case in the layout.
    sweep "$app" "$VERSION" "$platdir"no-video/*
  done
done

# tools/dist/bin.sh's archives, which sit one level higher than everything else the loop above walks:
# `dist/bin/karaokemachine-bin-<version>-<triple>.zip`, beside the versionless `<platform>/` folder
# rather than inside it. They are swept here rather than by the loop for exactly that reason -- the
# loop descends to `dist/<app>/<platform>/*`, and these are at `dist/<app>/*`.
#
# The folders themselves are left alone, deliberately; see the second bullet above `sweep`.
sweep karaokemachine-bin         "$VERSION" dist/bin/*
sweep karaokemachine-bin-console "$VERSION" dist/bin-console/*

# The Windows setup program, `dist/setup/windows/karaokemachine-setup-<version>-windows-x86_64.exe`.
# It sits where the loop above already walks -- but the loop passes the *directory* name as the app,
# so it looks for something beginning `setup-` and this file begins `karaokemachine-setup-`. Every
# old installer would be kept for ever, silently, and each is about 90 MB.
sweep karaokemachine-setup       "$VERSION" dist/setup/windows/*

# ...and the macOS one, `dist/setup/macos/karaokemachine-setup-<version>-macos-<arch>.pkg`, which
# may also carry a `-unsigned` or `-unnotarized` marker -- `sweep` takes the version as the field
# after the app name, so a marked name is matched exactly like a plain one and needs nothing here.
# The system in the name changes nothing there either, sitting after the version rather than before
# it. A separate line rather than a glob over `dist/setup/*` because `sweep` takes a directory of
# items, and each of these is about 80 MB, so the cost of forgetting one is the same as it was above.
sweep karaokemachine-setup       "$VERSION" dist/setup/macos/*

# The remote's own setup programs, in the same two directories. A second prefix rather than a wider
# glob, because `sweep` takes the version as the field after the app name: `karaokemachine-setup` is
# not a prefix of `km-remote-setup`, so without these every old one is kept for ever.
sweep km-remote-setup            "$VERSION" dist/setup/windows/*
sweep km-remote-setup            "$VERSION" dist/setup/macos/*

echo "dist-clean: $REMOVED older staged item(s); kept $VERSION"
