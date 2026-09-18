#!/usr/bin/env bash
#
# Creates a git worktree that can actually build and run, so a second session -- another Claude Code
# window, another terminal -- can work on this repository without fighting the first one over the
# same files.
#
#   tools/dev/worktree.sh video-fix                    # .claude/worktrees/video-fix, on branch video-fix
#   tools/dev/worktree.sh video-fix --base origin/master
#   tools/dev/worktree.sh video-fix --path /some/other/place
#   tools/dev/worktree.sh video-fix --no-assets        # skip the SoundFont, the pack and the overlay
#   tools/dev/worktree.sh --where video-fix            # print where it would go, and do nothing else
#   tools/dev/worktree.sh --remove video-fix
#   tools/dev/worktree.sh --list
#
# WHY THIS EXISTS, when `git worktree add` is one line. Because `git worktree add` gives you the
# *committed* files, and several of the things this repository needs in order to be useful are
# deliberately not committed:
#
#   CLAUDE.local.md               this machine's paths, the corpus, the appliance box
#   .claude/settings.local.json   Claude Code's per-project permissions
#   assets/soundfont/*.sf2        31 MiB, fetched once per machine by tools/setup/fetch-assets.sh
#   assets/wallpapers/*.zip       a built pack somebody put here with --zip-dest. The shipped set,
#                                 default-wallpapers.zip, is committed and needs no seeding.
#   local/assets/soundfont/*      a bank somebody put there by hand. Nothing installs one any more --
#                                 `task soundfont` caches the bank outside the repository instead --
#                                 but the overlay rule still honors one, so a checkout that has one
#                                 keeps it.
#   local/assets/wallpapers/*.zip ~20 MiB, where tools/cmd/assets/km-wallpaper-pack now puts a pack by default
#   local/assets/fonts/*          a display font dropped in by hand, if there is one
#
# A worktree missing the first is a session that does not know where ffmpeg lives or what the
# appliance is called -- it will rediscover all of it the hard way. A worktree missing the others
# builds perfectly and sounds wrong, which is worse than failing. So this copies them across.
#
# ** `local/assets/**` and nothing else under `local/`. ** That folder also holds each worktree's own
# data directory (`--data-dir ./local/km-<name>`) and personal notes, and carrying those across would
# hand the new worktree the first one's settings.json, catalog and packages folder -- which is
# exactly the collision the closing note below warns about. So the seeding is by explicit glob, never
# by recursing `local/`.
#
# Seeding the overlay is also what makes it *work* in a worktree: km-app enables it only when the
# working directory has both `assets/` and `local/assets/`, and a fresh `git worktree add` creates no
# gitignored folders at all. So `--no-assets` gives a worktree with no overlay whatsoever, not merely
# an empty one -- which is the honest reading of a flag that means "skip the big uncommitted files".
#
# Nothing else is missing, and that is worth stating because it is not obvious: FFMPEG_DIR and
# LIBCLANG_PATH live in cargo's own `[env]` table in $CARGO_HOME/config.toml, which is machine-global
# (see tools/setup/fetch-ffmpeg.sh), so `cargo km-build` works in a fresh worktree with no further setup --
# which matters more now that the plain commands are the video ones.
#
# WHAT IT DELIBERATELY DOES NOT DO: share `target/` between worktrees. Each one therefore rebuilds
# SDL3, SDL3_ttf and the bundled SQLite from source -- about three minutes, and several GB per
# worktree. Pointing them all at one CARGO_TARGET_DIR trades that for cargo's exclusive lock on the
# build directory, so the second build sits at "Blocking waiting for file lock" until the first
# finishes; and it churns worse here than usual, because flipping the `video` feature between
# worktrees invalidates a shared cache. Parallel sessions that block on each other's builds are not
# parallel sessions.
#
# **The lock is the smaller half of the reason, and the larger half was learned the hard way.** Two
# checkouts writing one build directory also *share artifacts*, and cargo will hand a cached test
# binary compiled in checkout A to a run in checkout B whenever the sources match -- with
# `env!("CARGO_MANIFEST_DIR")` baked into it. What that looks like is a test failing on a fixture path
# naming a directory you have never worked in, possibly one that no longer exists. It cost an
# afternoon. See "`CARGO_TARGET_DIR` is not set, and should not be" in CONTRIBUTING.md.
#
# So this script no longer offers to put the build output anywhere. It used to: a CARGO_TARGET_DIR in
# .claude/settings.local.json was rewritten to a per-worktree sibling on the way in. That mechanism is
# gone because it could not work for the case it was written for -- Claude Code resolves that file's
# `env` block when a *session starts*, so a session that entered a worktree mid-flight kept the parent
# checkout's directory and the slot sat empty. Nothing inside a worktree can fix that either: a real
# CARGO_TARGET_DIR environment variable beats both `[build] target-dir` and cargo's `[env]` table with
# `force = true`. Leaving cargo to its default is the only arrangement that is per-checkout no matter
# how the session began -- and it has the property the slots never had, that removing a worktree
# removes its build output with it.

set -euo pipefail

cd "$(dirname "$0")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

REPO_ROOT="$(pwd)"

NAME=""
BASE=""
DEST=""
ASSETS=1
ACTION="create"

die() { echo "worktree: $*" >&2; exit 1; }

# The header block above is the help text. It ends where the comments end, which is why this stops at
# the first line that is not one rather than at a line number: a hardcoded range silently starts
# printing `set -euo pipefail` and the argument parser the moment somebody adds a paragraph.
usage() {
  awk 'NR > 1 { if (!/^#/) exit; sub(/^# ?/, ""); print }' "$0"
  exit "${1:-0}"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --base)       BASE="${2:-}"; [ -n "$BASE" ] || die "--base needs a ref"; shift 2 ;;
    --path)       DEST="${2:-}"; [ -n "$DEST" ] || die "--path needs a directory"; shift 2 ;;
    --no-assets)  ASSETS=0; shift ;;
    --where)      ACTION="where"; NAME="${2:-}"; [ -n "$NAME" ] || die "--where needs a name"; shift 2 ;;
    --remove)     ACTION="remove"; NAME="${2:-}"; [ -n "$NAME" ] || die "--remove needs a name"; shift 2 ;;
    --list)       ACTION="list"; shift ;;
    -h|--help)    usage 0 ;;
    -*)           die "unknown option: $1" ;;
    *)            [ -z "$NAME" ] || die "give one name, not two ($NAME and $1)"; NAME="$1"; shift ;;
  esac
done

git rev-parse --git-dir >/dev/null 2>&1 || die "not a git repository"

if [ "$ACTION" = "list" ]; then
  git worktree list
  exit 0
fi

[ -n "$NAME" ] || usage 1

# Inside the checkout, under the `/.claude/worktrees/` that `.gitignore` covers, because a worktree
# there is inside the permission root a Claude Code session already holds and one beside the
# repository is not -- entering an outside one costs a confirmation, every time, and this repository
# asks a session to enter a worktree before it touches any tracked file. What it charges instead, and
# what answers that, is `A worktree lives inside the checkout` in docs/decisions/repository.md.
#
# Resolved against the main checkout rather than against $PWD, which is not the same directory when
# this script is run from inside a worktree: a relative answer would nest, and each level of
# `.claude/worktrees/<a>/.claude/worktrees/<b>` carries another multi-gigabyte `target/`. The parent
# of the common git directory is the main worktree from anywhere in the repository.
#
# `--path` is the escape hatch, and it keeps whatever it is given.
MAIN_ROOT="$(cd "$(dirname "$(git rev-parse --git-common-dir)")" && pwd)"
[ -n "$DEST" ] || DEST="$MAIN_ROOT/.claude/worktrees/$NAME"

if [ "$ACTION" = "where" ]; then
  # The one place the rule above is spelled, so that tools/dev/claude-worktree-hook.sh can ask rather
  # than restate it. A second copy of the rule is a copy that fails *after* a successful create, when
  # the two spellings drift apart.
  echo "$DEST"
  exit 0
fi

if [ "$ACTION" = "remove" ]; then
  # `git worktree remove` refuses a dirty worktree, which is the correct default and the reason this
  # does not pass --force. The branch is left alone: it may hold the only copy of the work, and
  # deleting somebody's commits to tidy up a directory is not a thing a helper script should do.
  git worktree remove "$DEST"
  git worktree prune
  echo "worktree: removed $DEST"
  echo "worktree: branch '$NAME' is still there -- 'git branch -d $NAME' once it is merged"
  exit 0
fi

[ -e "$DEST" ] && die "$DEST already exists"
git show-ref --verify --quiet "refs/heads/$NAME" && die "branch '$NAME' already exists (pick another name, or 'git worktree add $DEST $NAME')"

# A worktree never inherits uncommitted changes, whatever it is based on -- so say so, rather than
# letting the second session discover that half the work is missing.
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "worktree: note -- this checkout has uncommitted changes, and a worktree never inherits them."
  echo "worktree:         commit or stash first if the new one needs them."
fi

# The destination's parent, which a fresh clone does not have: `/.claude/worktrees/` is ignored, so
# git brings none of it down and the first worktree in a new checkout is the one that creates it.
mkdir -p "$(dirname "$DEST")"

if [ -n "$BASE" ]; then
  git worktree add -b "$NAME" "$DEST" "$BASE"
else
  git worktree add -b "$NAME" "$DEST"
fi

DEST_ABS="$(cd "$DEST" && pwd)"

# Copy the small text files and hard-link the big binaries. Two different answers on purpose:
#
#   - A copy of CLAUDE.local.md means the two sessions can edit their notes independently, which is
#     what you want from something whose whole job is to record what one session learned.
#   - A hard link for the SoundFont and the wallpaper pack means 51 MiB is not duplicated per
#     worktree. Neither file is ever edited in place -- fetch-assets.sh writes a temporary name and
#     moves it over, which breaks the link apart harmlessly rather than writing through it.
#
# `ln` without -s is a real hard link on NTFS under Git Bash and needs no privilege, unlike a
# symlink, which needs Developer Mode or an elevated shell. It still fails across volumes, so every
# call falls back to a copy.
link_or_copy() { # <src> <dest>
  ln "$1" "$2" 2>/dev/null || cp "$1" "$2"
}

copied=0
copy_one() { # <relative path> <copy|link>
  local rel="$1" how="$2" dir
  [ -f "$REPO_ROOT/$rel" ] || return 0
  dir="$(dirname "$DEST_ABS/$rel")"
  mkdir -p "$dir"
  if [ "$how" = "link" ]; then link_or_copy "$REPO_ROOT/$rel" "$DEST_ABS/$rel"
  else cp "$REPO_ROOT/$rel" "$DEST_ABS/$rel"
  fi
  echo "  $rel"
  copied=$((copied + 1))
}

# Every file matching one glob below the repository root.
#
# A glob rather than a directory, deliberately: `local/` holds each worktree's own data directory and
# personal notes as well as the asset overlay, and only the overlay may travel. Naming the shapes that
# may be copied is what keeps that true -- a recursive copy of `local/` would hand the new worktree
# the first one's settings.json and catalog.
#
# `$1` is unquoted on purpose in the `for`: that is what makes the glob expand at all.
seed_glob() { # <relative glob> <copy|link>
  local f
  for f in "$REPO_ROOT"/$1; do
    [ -f "$f" ] || continue
    copy_one "$(dirname "$1")/$(basename "$f")" "$2"
  done
}

# Every CLAUDE.local.md in the tree, not only the one at the root.
#
# These live in the folder they are about -- the Docker and appliance notes in
# tools/platform/linux/, the ffmpeg ones in crates/playback/km-video/ -- because Claude Code loads a
# nested one only when it reads a file in that subtree, where the root file is charged to every
# session. See `Where a folder-scoped instruction lives` in docs/decisions/repository.md.
#
# A `find` rather than the list of six, because the list is the thing that would silently stop being
# complete: a seventh note added to a folder nobody anticipated has to travel too, and nothing would
# report that it had not.
#
# `target/` is pruned because it holds none of these and walking it is the slowest thing this script
# could do. **`.claude/worktrees/` is pruned because the notes under it are another worktree's copies
# rather than this checkout's**, and that prune is required rather than tidy: every worktree lives
# there, so without it the second one is seeded with the first one's `CLAUDE.local.md` files and a
# session reads a machine's notes at one remove from the machine.
#
# The loop reads from a here-document rather than a pipe so that it runs in this shell and `copied`
# survives it; a `find ... | while` would count in a subshell and report nothing carried.
seed_local_notes() {
  local f rel
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    rel="${f#$REPO_ROOT/}"
    copy_one "$rel" copy
  done <<EOF
$(find "$REPO_ROOT" -name target -prune -o -name .git -prune \
    -o -path "$REPO_ROOT/.claude/worktrees" -prune \
    -o -name 'CLAUDE.local.md' -print)
EOF
}

echo
echo "worktree: carrying across what git does not:"
seed_local_notes
copy_one ".claude/settings.local.json" copy

if [ "$ASSETS" = "1" ]; then
  seed_glob 'assets/soundfont/*.sf2'          link
  copy_one  'assets/soundfont/LICENSE.txt'    copy
  seed_glob 'assets/wallpapers/*.zip'         link
  # The local asset overlay. Same split as above and for the same reason: hard-link the big binaries
  # so tens of megabytes are not duplicated per worktree, copy the small text beside them.
  seed_glob 'local/assets/soundfont/*.sf2'    link
  seed_glob 'local/assets/soundfont/*.txt'    copy
  seed_glob 'local/assets/soundfont/*.md'     copy
  seed_glob 'local/assets/wallpapers/*.zip'   link
  seed_glob 'local/assets/fonts/*'            copy
fi
[ "$copied" = "0" ] && echo "  (nothing -- this checkout has none of them either)"

# Either location will do -- a bank in the overlay is the one the machine would play. Written as a
# loop rather than as `ls a b`, which with one path present and one absent prints the file it found
# *and* exits 2, so the condition would be wrong in both directions.
bank_here=0
for f in "$REPO_ROOT"/assets/soundfont/*.sf2 "$REPO_ROOT"/local/assets/soundfont/*.sf2; do
  if [ -f "$f" ]; then bank_here=1; break; fi
done

if [ "$ASSETS" = "1" ] && [ "$bank_here" = "0" ]; then
  echo
  echo "worktree: no SoundFont here to copy. Run tools/setup/fetch-assets.sh in the new worktree;"
  echo "worktree: without one the engine falls back to a sine test tone. To listen to a different"
  echo "worktree: bank afterwards, task soundfont:list and task soundfont BANK=<name> -- which is"
  echo "worktree: per machine rather than per worktree, so it needs doing once wherever you are."
fi

cat <<EOF

worktree: $DEST_ABS  (branch $NAME)

  cd "$DEST_ABS" && claude

Two things collide if both worktrees also *run* the machine, and neither is a git problem:

  * The API binds 0.0.0.0:8177 and km-package-builder 8178. The second instance dies with
    "Address already in use".
  * The data directory comes from the platform's config folder, not the checkout, so every worktree
    shares one settings.json, one catalog and one packages folder. Two instances writing that
    catalog is the real hazard -- the port is only the noisy one.

Both are answered by giving each worktree its own data directory, and setting a different
\`api.bind\` in the settings.json it creates:

  cargo km -- --data-dir ./local/km-$NAME

\`/local/\` is already gitignored, so it stays out of the way. And only one session at a time should
run tools/platform/linux/deploy.sh -- there is one appliance.
EOF
