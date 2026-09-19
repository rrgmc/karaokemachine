#!/usr/bin/env bash
#
# The program and platform labels a pull request's changed paths ask for.
#
#   git diff --name-only master... | tools/dev/pr-labels.sh     # print them, one per line
#   gh pr diff 12 --name-only | tools/dev/pr-labels.sh --apply 12   # and add them to PR 12
#
# **The paths are the source of the label.** A pull request has no form to answer, and the folders it
# changes already say which program and platform it touches. `tools/dev/labels.sh paths` holds the
# folder-to-label table, and its `check` asserts that each row names a declared label and a folder
# that still exists.
#
# **`--apply` only adds.** The type label is chosen when the pull request is opened, and a label put
# on by hand stays. A path no row matches gives no label, which is the ordinary case for `docs/`.
#
# **Nothing in the input is ever evaluated.** A path is compared with each prefix as a string.
#
# The standing decision is `A pull request carries its type, and the programs and platforms it
# touches` in docs/decisions/repository.md.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

self="${0##*/}"

pr=""
case "${1-}" in
  "") ;;
  --apply)
    pr="${2-}"
    [ -n "$pr" ] || { echo "$self: --apply wants a pull request number" >&2; exit 1; }
    ;;
  *)
    echo "usage: $self [--apply <pr>] < changed-paths" >&2
    exit 1
    ;;
esac

# Through `bash`, as the Taskfile and CI run every script in this directory: a script's executable
# bit does not survive a commit made on Windows.
rows="$(bash tools/dev/labels.sh paths)"

found=""
while IFS= read -r path; do
  [ -n "$path" ] || continue
  while IFS='|' read -r label prefix; do
    if [ "${path#"$prefix"}" != "$path" ]; then
      found+="$label"$'\n'
    fi
  done <<<"$rows"
done

labels="$(printf '%s' "$found" | sed '/^$/d' | sort -u)"
[ -n "$labels" ] || exit 0

printf '%s\n' "$labels"

if [ -n "$pr" ]; then
  command -v gh >/dev/null || { echo "$self: gh is not on PATH" >&2; exit 1; }
  gh pr edit "$pr" --add-label "$(printf '%s' "$labels" | paste -sd, -)"
fi
