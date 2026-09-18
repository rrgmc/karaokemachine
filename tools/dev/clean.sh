#!/usr/bin/env bash
#
# Takes a checkout back to what a fresh clone of it looks like.
#
#   tools/dev/clean.sh                 # dist/, the cargo directories, the ports' build products
#   tools/dev/clean.sh --dry-run       # ...say what that would remove, remove nothing
#   tools/dev/clean.sh --docker        # ...and the Linux tooling's images and volumes
#   tools/dev/clean.sh --cache         # ...and the machine's asset cache: the SoundFont and ffmpeg
#
# The counterpart of `tools/dist/clean.sh`, which takes away staged *releases* and nothing else. This
# one takes away everything a build here writes, and it **calls that script for the dist half**
# rather than walking `dist/` a second time: the release layout is written down in
# tools/dist/common.sh and read back in exactly one place, and a cleaner that reconstructed it would
# be a second place to keep in step.
#
# **Two things it does not touch by default, each behind a flag, and both for the same reason: what
# they hold is shared with something this run cannot see.**
#
#   `--docker`  The `karaokemachine-deb-build` volume is shared by every checkout and every worktree
#               on this machine -- that is the point of it -- so removing it while a peer session is
#               mid-build takes that session's cache with it. CLAUDE.md says as much about pruning in
#               general. Ask for it deliberately, when nothing else is running.
#   `--cache`   The asset cache is outside the repository and shared by every checkout, and refilling
#               it is a 31 MiB download plus, on macOS and for Android, an ffmpeg build. It is not a
#               build product of this checkout at all; it is here so that "clean everything" can mean
#               it when somebody means it.
#
# **And four it does not touch at all**, which is the boundary worth stating rather than leaving to
# be discovered: `local/` (the asset overlay, and whatever `--data-dir` runs live under it),
# `scratch/`, `tools/cmd/assets/km-wallpaper-pack/.wpcache` with the pack in `out/`, and `.claude/`.
# The first two are somebody's own settings, catalog and working files, and the third is hundreds of
# megabytes that can only be refetched with an API key. None of them is a build product; a cleaner
# that guessed otherwise would be a cleaner nobody could run without checking first.
#
# `.claude/` is the fourth and the one with a checkout under it: `worktrees/` holds every worktree of
# this repository, each with its own `target/`. Those gigabytes leave with
# `tools/dev/worktree.sh --remove`, which refuses a dirty worktree, where a cleaner reaching in would
# take another session's uncommitted work with them.

set -euo pipefail

cd "$(dirname "$0")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

DRY=0
DOCKER=0
WANT_CACHE=0
for arg in "$@"; do
  case "$arg" in
    --dry-run|-n) DRY=1 ;;
    --docker)     DOCKER=1 ;;
    --cache)      WANT_CACHE=1 ;;
    -h|--help)
      echo "usage: tools/dev/clean.sh [--docker] [--cache] [--dry-run]"
      exit 0
      ;;
    *) echo "clean: unknown option $arg" >&2; exit 2 ;;
  esac
done

REMOVED=0
FAILED=0

take() { # <path>
  [ -e "$1" ] || return 0
  if [ "$DRY" -eq 1 ]; then
    echo "  would remove $1"
  else
    echo "  removing $1"
    rm -rf "$1"
  fi
  REMOVED=$((REMOVED + 1))
}

# -- the staged releases -------------------------------------------------------------------------

echo "== dist"
if [ "$DRY" -eq 1 ]; then
  tools/dist/clean.sh --all --dry-run
else
  tools/dist/clean.sh --all
fi

# -- what cargo built ------------------------------------------------------------------------------
#
# `cargo clean` and not `rm -rf target`, for the reason "Nothing may assume cargo builds into
# `target/`" gives in docs/ARCHITECTURE.md: the directory moves, and a cleaner that removed the wrong
# one would report success having freed nothing. Two manifests, because tools/cmd/assets is a second
# workspace with a target directory of its own -- the same pair `task fmt` names, for the same
# reason, and naming its root rather than a member is what keeps it one line as members are added.

echo "== cargo"
for manifest in Cargo.toml tools/cmd/assets/Cargo.toml; do
  if [ "$DRY" -eq 1 ]; then
    echo "  would run cargo clean --manifest-path $manifest"
  else
    echo "  cargo clean --manifest-path $manifest"
    cargo clean --manifest-path "$manifest"
  fi
done

# -- the native shells -------------------------------------------------------------------------
#
# Everything .gitignore lists under `ports/`, and nothing else there. `local.properties` is
# deliberately left: Gradle writes it, but it can also be where somebody put an `sdk.dir` by hand,
# and it costs nothing to keep.

echo "== ports"
for d in \
  ports/machine/android/.gradle \
  ports/machine/android/build \
  ports/machine/android/app/build \
  ports/machine/android/app/src/main/jniLibs \
  ports/machine/android/app/src/main/assets \
  ports/remote/android/.gradle \
  ports/remote/android/build \
  ports/remote/android/app/build \
  ports/remote/android/app/src/main/jniLibs \
  ports/machine/ios/Frameworks \
  ports/machine/ios/build \
  ports/machine/ios/KaraokeMachine.xcodeproj \
  ports/machine/ios/KaraokeMachine/Info.plist \
  ports/machine/ios/KaraokeMachine/assets \
  ports/machine/ios/ffmpeg.yml \
  ports/remote/ios/Frameworks \
  ports/remote/ios/build \
  ports/remote/ios/KaraokeRemote.xcodeproj \
  ports/remote/ios/KaraokeRemote/Info.plist
do
  take "$d"
done

# -- the generated fixtures ------------------------------------------------------------------------
#
# `cargo run -p km-song --features testing --example write_fixtures` writes these and nothing commits
# them -- see fixtures/README.md. `scratch/` beside it is hand-made and is left alone.

echo "== fixtures"
take fixtures/generated

# -- the asset cache, if asked -----------------------------------------------------------------

if [ "$WANT_CACHE" -eq 1 ]; then
  echo "== cache"
  # The one definition of where it is, shared with the three scripts that fill it. Agreeing with them
  # by construction is the whole reason that file exists.
  . tools/setup/asset-cache.sh
  echo "  this is shared with every checkout on this machine, and refilling it means a download"
  take "$CACHE"
fi

# -- Docker, if asked ------------------------------------------------------------------------------
#
# The two volumes by exact name, and every image this repository's tooling tags. Both image families
# are content-addressed -- tools/platform/linux/image-tag.sh and the `verify_image_tag` in
# verify-image.sh -- so there is no single tag to name and a reference filter is the only way to
# reach the lot. It is scoped to this project's own prefix: nothing here runs `docker system prune`,
# which would be somebody else's containers as well as ours.

if [ "$DOCKER" -eq 1 ]; then
  echo "== docker"
  echo "  the build volume is shared by every checkout and worktree -- do not do this beside a peer build"
  export MSYS2_ARG_CONV_EXCL='*'
  if ! docker version >/dev/null 2>&1; then
    echo "clean: cannot reach the Docker daemon, so nothing Docker holds was removed." >&2
    echo "       On Windows, start Docker Desktop and wait for it to say it is running." >&2
    FAILED=1
  else
    for v in karaokemachine-deb-build karaokemachine-apt-cache; do
      if docker volume inspect "$v" >/dev/null 2>&1; then
        if [ "$DRY" -eq 1 ]; then
          echo "  would remove volume $v"
        else
          echo "  removing volume $v"
          # A volume still attached to a container is in use by something this run cannot see, and
          # taking it is not this script's call. `docker volume rm` refuses it; say so and go on.
          docker volume rm "$v" >/dev/null || { echo "clean: $v is in use -- left alone" >&2; FAILED=1; continue; }
        fi
        REMOVED=$((REMOVED + 1))
      fi
    done
    for ref in 'karaokemachine-deb:*' 'karaokemachine-verify-*:*'; do
      while read -r tag; do
        [ -n "$tag" ] || continue
        if [ "$DRY" -eq 1 ]; then
          echo "  would remove image $tag"
        else
          echo "  removing image $tag"
          docker image rm "$tag" >/dev/null || { echo "clean: could not remove $tag" >&2; FAILED=1; continue; }
        fi
        REMOVED=$((REMOVED + 1))
      done <<EOF
$(docker image ls --filter "reference=$ref" --format '{{.Repository}}:{{.Tag}}')
EOF
    done
  fi
fi

echo
if [ "$DRY" -eq 1 ]; then
  echo "clean: $REMOVED item(s) would go, plus whatever cargo holds. Nothing was removed."
else
  echo "clean: $REMOVED item(s) removed."
fi
[ "$WANT_CACHE" -eq 1 ] || echo "       the asset cache is untouched -- --cache takes the SoundFont and ffmpeg too."
[ "$DOCKER" -eq 1 ]     || echo "       Docker is untouched -- --docker takes the Linux tooling's images and volumes."
exit "$FAILED"
