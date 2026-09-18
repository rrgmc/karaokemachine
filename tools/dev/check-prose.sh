#!/usr/bin/env bash
#
# Prose in this repository states the rule, not how it was arrived at.
#
#   tools/dev/check-prose.sh --changed  # only the lines this branch added -- what a push should run
#   tools/dev/check-prose.sh --commits  # the branch's own commit messages, subject and body
#   tools/dev/check-prose.sh            # every tracked file
#   tools/dev/check-prose.sh --list     # ...and print the shapes it looks for, then check
#
# The standing decision is `How a document in this repository is written` in
# docs/decisions/repository.md, and the short form is in CLAUDE.md. This keeps the mechanical half of
# it true. The half it cannot see is the important one -- a paragraph of reassurance has no
# distinctive shape, and neither does an appositive tail -- so a clean run is not a pass. New prose
# still needs a human pass, exactly as `check-no-local-refs.sh` says of a brand name.
#
# **`--changed` is what `task check` runs, and the whole-tree form is the audit.** The split is a
# cost: fourteen shapes over 700 files is ~10,000 `grep` spawns, which is 2m21s on this repository's
# Windows box against 3.3s for a branch's own lines -- and `sys` is two thirds of that 2m21s, so it
# is process creation rather than matching. A 2m21s guard in front of `check-no-local-refs.sh`'s 13s
# would invert the order `task check` is arranged in; three seconds does not.
#
# The whole-tree form is the one to run by hand after a large rewrite, and the one CI runs on every
# event -- it needs no history and no remote ref, where `--changed` needs `origin/master` to resolve a
# base.
#
# **What it matches is the list that decision gives, in the order it gives them**: what something
# used to be, chronology, and meta-commentary on the writing. Those three have shapes. It runs over
# code comments as well as documents, because the decision says the rule applies to both.
#
# **`--commits` is the one mode reading something with no file behind it**, and its reach is the
# branch's own commits above `origin/master` -- so a message that is already on the default branch is
# never read, and the shapes are judged where a reword is still free. A hit names the short sha and
# whether it fell in the subject or the body.

set -uo pipefail

cd "$(dirname "$0")/../.." || exit 2

# One extended-regex per shape, so a hit can name the rule it broke rather than a pattern number.
#
# `\b` around the short ones: "used to" catches "it used to be" and must not catch "accustomed to".
# The milestone shapes are deliberately narrow -- a *measurement* may be dated, and
# `docs/architecture/` is full of legitimate ones, so only a version-numbered claim is matched.
SHAPES=(
  "what something used to be|\\b(used to (be|say|sit|carry|live|exist|read|have|do|call|spell|hold|mean))\\b"
  # **`no longer` is only a violation about an identity, and this is the shape that tells them
  # apart.** *No longer a copy*, *no longer an argument*, *no longer the only way* say a thing became
  # a different thing, which is the repository's own past. *No longer receiving*, *no longer holds*,
  # *no longer works* say a machine's state changed, which is what a product description is for and
  # is present tense however it reads. So the article -- or `what`, or `is` with nothing after it --
  # is what carries the match, and a verb after it carries none.
  "what something used to be|\\bno longer (a|an|the|what|only|its|their)\\b"
  "what something used to be|\\bno longer (is|was|does|did)\\b"
  "what something used to be|\\b(this|which) (reverses|replaces a|used to)\\b"
  "what something used to be|\\bwas (billed|formerly|previously)\\b"
  "what something used to be|\\bformer(ly)? (name|default|behaviou?r|spelling)\\b"
  "chronology|\\b(since|until|before|after|in) [0-9]+\\.[0-9]+(\\.[0-9a-z]+)?\\b"
  "chronology|\\bfor (two|three|four|five|several) milestones\\b"
  "chronology|\\bmilestone [0-9]"
  # **`worth knowing` is not in this list, and the omission is the calibration.** The entry names
  # `worth saying out loud`, and *saying*, *recording* and *reading* are all about the act of writing
  # or reading the document -- its subject is itself. *Worth knowing* is about the reader's knowledge
  # of the subject, and it is how this repository signposts a list: *five things worth knowing before
  # changing any of it*. Flagging it would rewrite sixty-odd lines on a reading the entry does not
  # ask for.
  "meta-commentary on the writing|\\bworth (recording|saying|reading)\\b"
  "meta-commentary on the writing|\\b(recorded|noted|stated) rather than\\b"
  "meta-commentary on the writing|\\bthat is the record of\\b"
  "meta-commentary on the writing|\\bthis (paragraph|sentence|entry|row) (replaces|used to)\\b"
  "meta-commentary on the writing|\\brather than an (accident|oversight|omission)\\b"
)

# `docs/HISTORY.md` and `docs/learning-rust.md` are exempt and say so in place: one is the origin
# story and the other a teaching document, where the arc from wrong to right *is* the content. This
# script's own header names the shapes it hunts, so it excludes itself for the reason
# `check-no-local-refs.sh` does.
EXEMPT='^(docs/HISTORY\.md|docs/learning-rust\.md|tools/dev/check-prose\.sh)$'

CHANGED=0
COMMITS=0
for arg in "$@"; do
  [ "$arg" = "--changed" ] && CHANGED=1
  [ "$arg" = "--commits" ] && COMMITS=1
done

# **`--commits` on its own reads messages and no files.** `lint:prose` names the two forms as two
# commands so a failure says which of them it is, and a run that read every tracked file to check a
# commit message would put the slowest form in front of the fastest one.
FILES_MODE=1
[ "$COMMITS" -eq 1 ] && [ "$CHANGED" -eq 0 ] && FILES_MODE=0

# Text this repository writes: documents, and the languages whose comments carry reasoning.
KINDS=('*.md' '*.rs' '*.html' '*.css' '*.ftl' '*.toml' '*.sh' '*.yml')

# The default branch's tip is what both `--changed` and `--commits` measure from: the lines this
# branch added, and the commits it added them in.
#
# **A base that will not resolve is fatal, and the fallback it replaces is why.** Falling back to
# `git rev-parse HEAD` diffs HEAD against itself: no files, `nothing to read`, exit 0 -- a clean
# run over nothing, which is the one answer a gate must never give. A clone with no `origin/master`
# is the ordinary way to get there, and `actions/checkout` makes one by default at `fetch-depth: 1`.
# Say which ref is missing and stop; the whole-tree form needs no ref at all and is what CI runs for
# that reason.
#
# It assigns rather than prints, because a command substitution would take the `exit 2` into a
# subshell and leave the caller running.
base=""
resolve_base() {
  if ! base=$(git merge-base origin/master HEAD 2>/dev/null); then
    echo "check-prose: $1 needs origin/master, and this clone has no such ref." >&2
    echo "             Fetch it, or run the whole-tree form, which needs no history." >&2
    exit 2
  fi
}

if [ "$FILES_MODE" -eq 1 ]; then
  if [ "$CHANGED" -eq 1 ]; then
    # A file added and not yet committed counts: `--changed` is what somebody runs before asking for
    # a merge.
    resolve_base --changed
    mapfile -t FILES < <({
      git diff --name-only --diff-filter=d "$base" -- "${KINDS[@]}"
      git ls-files --others --exclude-standard -- "${KINDS[@]}"
    } | sort -u | grep -Ev "$EXEMPT")
  else
    mapfile -t FILES < <(git ls-files "${KINDS[@]}" | grep -Ev "$EXEMPT")
  fi
fi

# **An empty list is a broken run, not a clean one.** Seven hundred tracked files match those globs,
# so nothing legitimate reaches here with none: what does is `git` missing from the PATH, or a
# directory that is not a checkout, and in both cases every shape went unread. `--changed` over a
# branch that touched no prose is the one honest empty case, and it is reported as such above.
#
# The Debian image `task check:linux` uses carries no `git`, which is how this was found: the
# whole-tree form there printed a clean run having read nothing at all.
if [ "$FILES_MODE" -eq 1 ] && [ "${#FILES[@]}" -eq 0 ]; then
  if [ "$CHANGED" -eq 1 ]; then
    echo "check-prose: this branch adds no prose"
    FILES_MODE=0
    [ "$COMMITS" -eq 0 ] && exit 0
  else
    echo "check-prose: no tracked files matched, which cannot be right." >&2
    echo "             Is \`git\` on the PATH, and is this a checkout?" >&2
    exit 2
  fi
fi

if [ "${1:-}" = "--list" ]; then
  echo "check-prose: the shapes it looks for"
  for entry in "${SHAPES[@]}"; do
    printf '  %-34s %s\n' "${entry%%|*}" "${entry#*|}"
  done
  echo
fi

# In `--changed` mode, only the lines this branch *added* are read. Filtering by file instead would
# make touching a file inherit its backlog, which is how a gate like this teaches people to leave
# files alone.
added_lines() {
  git diff --unified=0 "$base" -- "$1" 2>/dev/null |
    awk '/^@@/ { split($3, a, ","); start = a[1] + 0; count = (a[2] == "" ? 1 : a[2] + 0);
                 for (i = 0; i < count; i++) print start + i }'
  # An untracked file is new whole.
  if git ls-files --others --exclude-standard --error-unmatch -- "$1" >/dev/null 2>&1; then
    awk '{ print NR }' "$1"
  fi
}

# **One judgement, and the caller says where the words came from.** A file's lines and a commit
# message's go through the same fourteen shapes and the same two suppressions. `judge` reads one
# `<line number>:<text>` from `grep` and reports it or drops it, taking the rest of what it needs
# from the `scan` that called it, and it runs only on a line a shape already matched.
judge() {
  local hit="$1" number where
  number="${hit%%:*}"
  if [ -n "$scope" ] && [[ "$scope" != *" $number "* ]]; then
    return
  fi
  # **Two things are named rather than used, and both are stripped before the line is judged.**
  #
  # A phrase inside quotation marks or a code span is a mention: the entry this enforces has to
  # quote the shapes it forbids, and so does anything pointing at it.
  #
  # And **a measurement may be dated**, which the entry says outright -- an architecture note's
  # body is full of `in 16.6 ms` and `after 2.5 s`. A number carrying a unit is a measurement,
  # not a release somebody is narrating from.
  #
  # Strip both and re-test: what is left is the line's own voice.
  if ! printf '%s' "${hit#*:}" |
    sed -E -e 's/"[^"]*"//g' -e 's/`[^`]*`//g' \
      -e 's/(since|until|before|after|in) [0-9]+\.[0-9]+ ?(ms|s|fps|Hz|dB|px|%|[KMG]i?B)//g' |
    grep -qiE "$pattern"; then
    return
  fi
  # The entry's own list of what creeps back names each category as a bold list heading, which
  # is a definition rather than a use.
  if printf '%s' "${hit#*:}" |
    grep -qE '^\s*-\s+\*\*(What something used to be|Chronology|Meta-commentary on the writing)\.\*\*'; then
    return
  fi
  if [ "$kind" = "message" ]; then
    # `%s%n%n%b` puts the subject on line 1 and the body from line 3, which is the shape a reader
    # sees in `git log` and the one a reword is made against.
    if [ "$number" -eq 1 ]; then
      where="subject"
    else
      where="body:$((number - 2))"
    fi
    printf '%s %s: %s\n    ^ %s\n' "$label" "$where" "${hit#*:}" "$rule"
  else
    printf '%s:%s\n    ^ %s\n' "$label" "$hit" "$rule"
  fi
  found=1
}

# `$1` labels a hit, `$2` is the line numbers a hit may fall on -- empty means all of them -- `$3` is
# what is being read, and `$4` is the file to read when it is one. A message arrives on stdin, which
# is the half with no file to open. It is called without a pipeline so that `found` is the caller's
# own.
#
# **The producer is spelled twice rather than wrapped in a function, and the reason is a measurement.**
# A process substitution around a shell function forks a shell which then forks `grep`: a second
# process per shape per file, 9,800 of them over the tree, and process creation is two thirds of what
# this script costs. Back to back over the same tree, the whole-tree form is 2m12s handing `grep` the
# file name and 3m14s with the producer behind a function.
scan() {
  local label="$1" scope="$2" kind="${3:-file}" src="${4:-}"
  local text="" entry rule pattern hit
  [ "$kind" = "message" ] && text=$(cat)
  for entry in "${SHAPES[@]}"; do
    rule="${entry%%|*}"
    pattern="${entry#*|}"
    if [ "$kind" = "message" ]; then
      while IFS= read -r hit; do
        [ -n "$hit" ] && judge "$hit"
      done < <(printf '%s\n' "$text" | grep -niE "$pattern")
    else
      while IFS= read -r hit; do
        [ -n "$hit" ] && judge "$hit"
      done < <(grep -niE "$pattern" -- "$src" 2>/dev/null)
    fi
  done
}

found=0

if [ "$FILES_MODE" -eq 1 ]; then
  for file in "${FILES[@]}"; do
    scope=""
    if [ "$CHANGED" -eq 1 ]; then
      scope=$(added_lines "$file" | sort -un | tr '\n' ' ')
      [ -z "${scope// /}" ] && continue
      scope=" $scope"
    fi
    scan "$file" "$scope" file "$file"
  done
fi

# **A branch with no commits of its own is the honest empty case**, the way a branch that adds no
# prose is: the base resolved, `git rev-list` answered, and the answer was none. A base that will not
# resolve has already exited 2 above.
if [ "$COMMITS" -eq 1 ]; then
  resolve_base --commits
  mapfile -t REVS < <(git rev-list "$base..HEAD")
  if [ "${#REVS[@]}" -eq 0 ]; then
    echo "check-prose: this branch adds no commits"
    [ "$FILES_MODE" -eq 0 ] && exit 0
  fi
  for rev in "${REVS[@]}"; do
    # A merge carries a written subject here rather than git's default, so it is read like any other.
    scan "$(git rev-parse --short "$rev")" "" message < <(git log -1 --format='%s%n%n%b' "$rev")
  done
fi

if [ "$found" -ne 0 ]; then
  cat >&2 <<'WHY'

check-prose: the lines above narrate how a rule was arrived at rather than stating it.

  `How a document in this repository is written`, docs/decisions/repository.md

The keep-test: would a reader who deleted the sentence either re-derive a wrong answer, or break
something silently? A sentence about a state that is gone fails it. This applies to code comments on
the same test -- why a line is the way it is earns its place; what it used to be does not.

A commit message states the fault the change answers and the rule that holds after it. The route the
session took to get there is what fails, and `git commit --amend` or `git rebase -i --reword` is the
fix while the branch is unmerged, which is the only range this reads.
WHY
  exit 1
fi

# `lint:prose` runs the two forms as two commands, so the clean line says which of them answered.
if [ "$FILES_MODE" -eq 0 ]; then
  [ "${#REVS[@]}" -eq 1 ] && echo "check-prose: 1 commit message clean" ||
    echo "check-prose: ${#REVS[@]} commit messages clean"
else
  echo "check-prose: clean"
fi
