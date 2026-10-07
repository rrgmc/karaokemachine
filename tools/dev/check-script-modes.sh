#!/usr/bin/env bash
#
# A tracked file that opens with a shebang has mode 755.
#
#   bash tools/dev/check-script-modes.sh     # exit 1 and name every file that does not
#
# A shebang says the file is a program. Linux starts a program only when its mode allows it, and a
# script that calls another by path fails there with `Permission denied`. Windows has no such bit.
# Its filesystem reports every file as executable, and so does a folder it mounts into a container.
# A script written there gets mode 644 from `git add`, and runs on that machine all the same.
#
# **It reads the mode git holds, which is the same on every platform.** The working tree's mode
# says nothing on Windows.
#
# **A file that other scripts source carries no shebang**, and keeps mode 644. That is how this
# tells `tools/dist/common.sh` from a script somebody runs.
#
# **What it cannot see is a script with no shebang that a caller runs by path.** The kernel refuses
# that one too, and nothing in the file says it is a program.
#
# The standing decision is `A script that opens with a shebang is executable` in
# docs/decisions/repository.md. `task check` runs it beside the other checks in this directory. CI
# runs it in the `guards` job.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

self='check-script-modes'
export LC_ALL=C

# `#!` and then a path. A Rust file can open with `#![...]`, which is an attribute.
# `--cached` reads the index, where the modes below come from. `|| true`: no match is exit 1.
shebangs="$(git grep --cached -I -n -E '^#! ?/' -- . | awk -F: '$2 == 1 { print $1 }' | sort || true)"

plain="$(git ls-files -s | awk '$1 == "100644"' | cut -f2 | sort)"

bad="$(comm -12 <(printf '%s\n' "$shebangs") <(printf '%s\n' "$plain") | sed '/^$/d')"

if [ -n "$bad" ]; then
  while IFS= read -r file; do
    printf '%s: %s opens with a shebang and has mode 644\n' "$self" "$file" >&2
  done <<< "$bad"
  printf '\n%s\n' "Give each one mode 755, which works on Windows too:

  git update-index --chmod=+x <file>

A file that other scripts source takes no shebang instead. See 'A script that opens with a shebang
is executable' in docs/decisions/repository.md." >&2
  exit 1
fi

count="$(printf '%s\n' "$shebangs" | sed '/^$/d' | wc -l | tr -d ' ')"
echo "$self: clean, $count files open with a shebang and each has mode 755"
