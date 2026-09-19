#!/usr/bin/env bash
#
# Claude Code's WorktreeCreate / WorktreeRemove hook, wired to tools/dev/worktree.sh.
#
# WHY THIS EXISTS. Claude Code can put a session in its own worktree by itself -- the `-w` flag, the
# EnterWorktree tool, agent isolation -- and left alone it does the plain thing: `git worktree add`
# into `.claude/worktrees/<name>`. That is the "half a checkout" tools/dev/worktree.sh was written to
# avoid: no CLAUDE.local.md, no .claude/settings.local.json, no SoundFont, no wallpaper pack. A
# session in one builds perfectly and sounds wrong.
#
# A WorktreeCreate hook is not a notification -- it is a *provider*. When one is configured, Claude
# Code hands it a name and takes the worktree's absolute path back on stdout, and its own
# `git worktree add` never runs. So this script is how tools/dev/worktree.sh becomes what Claude Code
# uses, and every worktree it makes is a checkout that can build, play and remember.
#
# Two consequences worth knowing, both good:
#
#   * Worktrees land where worktree.sh puts them, which is the same .claude/worktrees/<name> Claude
#     Code would have chosen by itself -- so a session enters one without being asked to approve a
#     permission root outside the repository. The rule and its cost are
#     `A worktree lives inside the checkout` in docs/decisions/repository.md, and this script asks
#     worktree.sh for the path rather than spelling it a second time.
#   * Claude Code never auto-removes a hook-based worktree. Removal comes back here, through
#     WorktreeRemove, and stays under this repository's rules -- which means the branch survives.
#
# CONTRACT. stdin is the hook's JSON event. stdout is *only* the worktree path, so everything this
# script and worktree.sh have to say goes to stderr. Exit non-zero and Claude Code reports the
# failure rather than silently falling back.
#
#   WorktreeCreate  {"hook_event_name":"WorktreeCreate","name":"video-fix", ...}  -> path on stdout
#   WorktreeRemove  {"hook_event_name":"WorktreeRemove","worktree_path":"...", ...}
#
# WIRING IT UP. See "Working in parallel" in CLAUDE.md. The hook goes in
# .claude/settings.local.json rather than the committed .claude/settings.json because naming an
# interpreter is a per-machine fact: on Windows a bare `bash` is *WSL's* bash, which is a different
# git looking at a different filesystem, so the Git Bash path has to be spelled out.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORKTREE_SH="$REPO_ROOT/tools/dev/worktree.sh"
[ -f "$WORKTREE_SH" ] || { echo "worktree-hook: no $WORKTREE_SH" >&2; exit 1; }

say() { echo "worktree-hook: $*" >&2; }
die() { say "$*"; exit 1; }

# The path this prints is consumed by Claude Code, which on Windows is a native process that cannot
# resolve Git Bash's `/c/prog/...`. `pwd -W` is MSYS's "give me the real one" and yields
# `C:/code/...`, which is also exactly the spelling `git worktree list` uses -- so this is what makes
# the reuse check below able to match, as well as what makes the answer usable. Everywhere else
# `pwd -W` is not a thing and plain `pwd` was already right.
native_path() { # <dir>
  ( cd "$1" && { pwd -W 2>/dev/null || pwd; } )
}

[ -f "$WORKTREE_SH" ] || die "cannot find tools/dev/worktree.sh next to this script ($WORKTREE_SH)"

EVENT_JSON="$(cat)"

# Field extraction without jq, which is not installed here and is not worth requiring for two
# string fields. Both are plain JSON strings; `name` is restricted by Claude Code to letters,
# digits, dots, underscores, dashes and "/" separators, so it carries no escapes. `worktree_path`
# can carry \\ and \" on Windows, so those two get undone.
json_str() { # <key>
  printf '%s' "$EVENT_JSON" \
    | sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"\\\\]*\(\\\\.[^\"\\\\]*\)*\)\".*/\1/p" \
    | head -1 \
    | sed 's/\\\\/\x01/g; s/\\"/"/g; s/\x01/\\/g'
}

EVENT="$(json_str hook_event_name)"

case "$EVENT" in

  WorktreeCreate)
    RAW_NAME="$(json_str name)"
    [ -n "$RAW_NAME" ] || die "WorktreeCreate carried no name"

    # Claude Code allows "/" inside a name and generates random ones. worktree.sh turns a name into
    # both a directory suffix and a branch, so flatten the separator and drop anything that has no
    # business in either.
    NAME="$(printf '%s' "$RAW_NAME" | tr '/' '-' | sed 's/[^A-Za-z0-9._-]//g')"
    [ -n "$NAME" ] || die "name '$RAW_NAME' has nothing usable in it"
    [ "$NAME" = "$RAW_NAME" ] && : || say "name '$RAW_NAME' -> '$NAME'"

    # worktree.sh owns where a worktree goes, so ask it. Computing the same path here is a copy of
    # the rule that fails *after* a successful create, at the assertion below, the moment the two
    # spellings drift apart.
    DEST="$(bash "$WORKTREE_SH" --where "$NAME")" || die "tools/dev/worktree.sh --where failed for '$NAME'"

    # Resume rather than fail. EnterWorktree reuses a name when a session comes back to work it
    # already started, and worktree.sh -- correctly, for a human running it -- refuses to touch
    # anything that exists. Here an existing, registered worktree is the answer, not an error.
    if [ -d "$DEST" ] && git -C "$REPO_ROOT" worktree list --porcelain | grep -qxF "worktree $(native_path "$DEST" 2>/dev/null)"; then
      say "reusing existing worktree $DEST"
      native_path "$DEST"
      exit 0
    fi

    say "creating worktree '$NAME' via tools/dev/worktree.sh"
    bash "$WORKTREE_SH" "$NAME" >&2 || die "tools/dev/worktree.sh failed for '$NAME'"

    [ -d "$DEST" ] || die "tools/dev/worktree.sh reported success but $DEST is not there"
    native_path "$DEST"
    ;;

  WorktreeRemove)
    WT_PATH="$(json_str worktree_path)"
    [ -n "$WT_PATH" ] || die "WorktreeRemove carried no worktree_path"

    # Claude Code hands back whatever spelling Windows gave it, which can be `C:\code\...`. To this
    # shell a backslash is an ordinary character, not a separator, so `basename` on that returns the
    # whole string and the name match below silently fails. Git accepts either spelling.
    WT_PATH="$(printf '%s' "$WT_PATH" | tr '\\' '/')"

    # Prefer worktree.sh, so removal keeps the repository's policy -- it refuses a dirty worktree
    # and it leaves the branch alone. It addresses a worktree by name, so recover the name from the
    # path when the path follows its convention, and fall back to plain git when it does not (a
    # worktree made by hand somewhere else, or one from before the convention).
    #
    # The name is the last component, and the test of whether it is really ours is to ask worktree.sh
    # where a worktree of that name goes and see whether that is this path. Asking rather than
    # matching a shape keeps the rule in one file for the reverse direction too, and it is right from
    # wherever this script is run rather than only from the checkout it sits in.
    BASE="$(basename "$WT_PATH")"
    WHERE="$(bash "$WORKTREE_SH" --where "$BASE" 2>/dev/null || true)"

    # **A worktree whose directory is already gone is the outcome asked for, and saying so is not
    # optional.** Claude Code keeps a worktree registered when removal reports failure and tries
    # again at the end of the next session, so a hook that treats "not there" as an error reports the
    # same failure every session from then on, with nothing left for any of them to remove. Both
    # branches below fail that way: the first needs the directory to exist before it will recognize
    # the worktree as ours, and the second hands `git worktree remove` a path that is not a working
    # tree. The prune is what finishes the job when the working files went without git being told,
    # and it is a no-op when they did not.
    if [ ! -d "$WT_PATH" ]; then
      git -C "$REPO_ROOT" worktree prune >&2
      say "$WT_PATH is already gone; any branch it held is still there"
      exit 0
    fi

    if [ -n "$WHERE" ] && [ -d "$WHERE" ] && [ "$(native_path "$WHERE" 2>/dev/null)" = "$(native_path "$WT_PATH" 2>/dev/null)" ]; then
      bash "$WORKTREE_SH" --remove "$BASE" >&2 || die "tools/dev/worktree.sh --remove failed for $WT_PATH"
    else
      say "$WT_PATH is not one of ours by name; removing it with git directly"
      git -C "$REPO_ROOT" worktree remove "$WT_PATH" >&2 || die "git worktree remove failed for $WT_PATH"
      git -C "$REPO_ROOT" worktree prune >&2
      say "removed $WT_PATH; any branch it held is still there"
    fi
    ;;

  "")
    die "no hook_event_name in the event on stdin"
    ;;

  *)
    die "unexpected hook event '$EVENT' -- this script handles WorktreeCreate and WorktreeRemove"
    ;;
esac
