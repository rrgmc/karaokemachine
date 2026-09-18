#!/usr/bin/env bash
#
# Every value in `.cargo/config.toml` is a string, and this is what keeps it that way.
#
#   tools/dev/check-cargo-config.sh         # exit 1 and name every array
#
# **A worktree is the reason.** Cargo walks up from the directory it runs in, collects every
# `.cargo/config.toml` above it and merges them: a string takes the nearest file's value, an array is
# *joined* with the ancestor's. A worktree lives at `.claude/worktrees/<name>` and so is a descendant
# of the checkout, which hands it both copies -- so an array arrives doubled and `cargo km-build`
# expands to `build ... build ...` there while working perfectly here. The standing decision is
# `A worktree lives inside the checkout` in docs/decisions/repository.md.
#
# **A check rather than a comment, because the failure is invisible from the checkout.** Somebody
# adding an alias writes it in the form the Cargo Book shows first, runs it where they are, sees it
# work, and breaks every worktree in the repository. Nothing about the alias itself says otherwise,
# and cargo's own maintainers describe the two forms as looking interchangeable.
#
# **What the array form buys is an argument containing a space, and that is what this costs.** Cargo
# splits a string on spaces, so such an argument cannot be spelled in this file at all, and the check
# has nothing to detect: the array that would carry it is refused on the line above. A command
# needing one belongs in `Taskfile.yml`, which Task reads from one directory and never merges -- the
# same division the file already makes for anything needing logic.
#
# `task check` runs it beside the other checks in this directory, ahead of fmt, because it reads one
# file and says one line. CI runs it in the `guards` job.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

self="${0##*/}"
config=".cargo/config.toml"
[ -f "$config" ] || { echo "$self: no $config" >&2; exit 1; }

found=0
fail() {
  echo "$self: $1" >&2
  found=$((found + 1))
}

# Every assignment whose value opens a bracket, wherever in the file it sits: `[alias]` is the table
# with most of them, and `rustflags` under a `[target.<triple>]` is one that is not an alias and
# merges by the same rule.
while IFS= read -r line; do
  key="${line%% =*}"
  fail "$key is an array; write it as a string, so a worktree takes this file's value instead of appending to it"
done < <(grep -nE '^[A-Za-z_][A-Za-z0-9_.-]* *= *\[' "$config" | sed 's/^[0-9]*://')

if [ "$found" -ne 0 ]; then
  printf '\n%s\n' "Cargo joins arrays from a parent directory's config and replaces strings, and a worktree
under .claude/worktrees/ reads this file twice. See 'A worktree lives inside the checkout' in
docs/decisions/repository.md." >&2
  exit 1
fi

echo "$self: clean, every value is a string"
