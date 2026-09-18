#!/usr/bin/env bash
#
# The platform and program labels an issue body asks for.
#
#   tools/dev/issue-labels.sh < body.md              # print them, one per line
#   tools/dev/issue-labels.sh --apply 12 < body.md   # and put them on issue 12
#
# **The form's answer is the source of the label.** GitHub renders a dropdown as its label under a
# `###` heading and joins a multiple choice with a comma, so the body carries the answer in a shape
# this reads: the first line under `### Platform` and the one under `### Program`.
# `tools/dev/labels.sh` holds the option-to-label table, and its `check` asserts the two still agree.
#
# **An option the table does not name is a failure rather than a silent skip**, because an edit to
# the form's dropdown is otherwise invisible until somebody wonders why an issue has no label.
#
# **A facet whose heading is absent is left alone.** `--apply` reconciles, so an answer edited to drop
# a platform drops the label with it, and a body that never carried the heading has nothing to
# reconcile against: a label put on by hand stays. `_No response_` is an answer, and it means none.
#
# **Nothing in the body is ever evaluated.** An issue body is written by anybody, and the workflow
# hands it over through the environment for the same reason.
#
# The standing decision is `An issue carries the platform and the program it is about` in
# docs/decisions/repository.md.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

self="${0##*/}"
labels_sh="tools/dev/labels.sh"

# Through `bash`, as the Taskfile and CI run every script in this directory: a script's executable
# bit does not survive a commit made on Windows, so a checkout on a runner has none and calling one
# by its path is a `Permission denied` that only a runner sees.
labels_table() {
  bash "$labels_sh" table
}

issue=""
case "${1-}" in
  "") ;;
  --apply)
    issue="${2-}"
    [ -n "$issue" ] || { echo "$self: --apply wants an issue number" >&2; exit 1; }
    ;;
  *)
    echo "usage: $self [--apply <issue>] < body" >&2
    exit 1
    ;;
esac

body="$(cat)"

has_heading() { # $1 heading
  printf '%s\n' "$body" | grep -qxF "### $1"
}

# The first line under a heading that is not blank.
answer_under() { # $1 heading
  printf '%s\n' "$body" | awk -v h="### $1" '
    $0 == h                 { want = 1; next }
    want && /^[[:space:]]*$/ { next }
    want                    { print; exit }
  '
}

label_for() { # $1 kind, $2 option
  labels_table | awk -F'|' -v k="$1" -v o="$2" '$3 == k && $4 == o { print $1; exit }'
}

contains() { # $1 newline-separated list, $2 item
  [ -n "$1" ] || return 1
  printf '%s\n' "$1" | grep -qxF -- "$2"
}

trim() { # $1 text
  local s="$1"
  s="${s#"${s%%[![:space:]]*}"}"
  s="${s%"${s##*[![:space:]]}"}"
  printf '%s' "$s"
}

# The labels one facet's answer asks for.
facet_wants() { # $1 kind, $2 heading
  local answer option label
  answer="$(answer_under "$2")"
  [ "$answer" != "_No response_" ] || return 0
  local IFS=','
  for option in $answer; do
    option="$(trim "$option")"
    [ -n "$option" ] || continue
    label="$(label_for "$1" "$option")"
    if [ -z "$label" ]; then
      echo "$self: the body answers $2 with \"$option\" and $labels_sh has no label for it" >&2
      exit 1
    fi
    printf '%s\n' "$label"
  done
}

# A heading the body does not carry is a facet this leaves alone, so it never reaches `answered`.
# Anywhere below it, an unnamed option fails the command substitution and takes the script with it.
want=""
answered=""
for facet in "platform:Platform" "program:Program"; do
  kind="${facet%%:*}"
  heading="${facet##*:}"
  has_heading "$heading" || continue
  answered="${answered}${kind}"$'\n'
  got="$(facet_wants "$kind" "$heading")"
  [ -z "$got" ] || want="${want}${got}"$'\n'
done
want="$(printf '%s' "$want" | sed '/^$/d')"

if [ -z "$issue" ]; then
  [ -z "$want" ] || printf '%s\n' "$want"
  exit 0
fi

command -v gh >/dev/null || { echo "$self: gh is not on PATH" >&2; exit 1; }

have="$(gh issue view "$issue" --json labels --jq '.labels[].name')"

add=""
while IFS= read -r label; do
  [ -n "$label" ] || continue
  if ! contains "$have" "$label"; then
    add="${add:+$add,}$label"
  fi
done <<<"$want"

# Only a facet the body answered is reconciled, so a label applied by hand under a heading the body
# does not carry survives an edit.
remove=""
while IFS='|' read -r name _ kind _ _; do
  case "$kind" in platform|program) ;; *) continue ;; esac
  contains "$answered" "$kind" || continue
  contains "$have" "$name" || continue
  if ! contains "$want" "$name"; then
    remove="${remove:+$remove,}$name"
  fi
done < <(labels_table)

args=()
[ -z "$add" ] || args+=(--add-label "$add")
[ -z "$remove" ] || args+=(--remove-label "$remove")

if [ "${#args[@]}" -eq 0 ]; then
  echo "$self: issue $issue already carries the labels its body asks for"
  exit 0
fi

gh issue edit "$issue" "${args[@]}"
