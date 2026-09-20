#!/usr/bin/env bash
#
# The issue labels this repository declares, and the commands that put them on GitHub.
#
#   tools/dev/labels.sh list                        # the table, for a person
#   tools/dev/labels.sh table                       # the same rows, for a script
#   tools/dev/labels.sh paths                       # the folder each pull request label comes from
#   tools/dev/labels.sh check                       # or: task lint:labels
#   tools/dev/labels.sh sync [--prune] [--dry-run]  # declare them on GitHub
#
# **A label names the platform and the program, because that is what an issue list is asked.** The
# bug form requires both, and an answer that lives only in the body cannot be filtered on: "what is
# broken on Android" means opening every issue to find out. `.github/workflows/issue-labels.yml`
# reads the answer and applies the label, through `tools/dev/issue-labels.sh`.
#
# **The table below is the only place a label is written down**, and `check` is what keeps a dropdown
# option and a label from parting company. The guard lives in this file rather than in a
# `check-*.sh` of its own, because a guard reading the table from a second file is the drift the
# table exists to prevent.
#
# **The type labels keep GitHub's stock names and colors.** `bug` and `enhancement` are what the two
# forms apply, and a name every reader of a GitHub repository already knows is worth keeping. The
# four stock labels this table leaves out go with `--prune`: `good first issue` and `help wanted`
# offer work to a crowd that is not here, `question` cannot arrive while blank issues are off, and
# `invalid` says what closing the issue says.
#
# **`dependencies` is in the table although nothing here applies it.** Dependabot applies it and
# creates it again when it is gone, so a table without it is one `--prune` fights every week.
#
# **A pull request takes the same labels from the paths it changes.** `paths` below maps a folder to
# a label, `tools/dev/pr-labels.sh` reads it, and `.github/workflows/pr-labels.yml` applies the result.
# `check` asserts that each of those labels is declared and each folder still holds a tracked file.
#
# The standing decisions are `An issue carries the platform and the program it is about` and
# `A pull request carries its type, and the programs and platforms it touches`, both in
# docs/decisions/repository.md.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

self="${0##*/}"
bug_form=".github/ISSUE_TEMPLATE/bug_report.yml"
feature_form=".github/ISSUE_TEMPLATE/feature_request.yml"

# name|color|kind|the bug form's option, verbatim|description
#
# A row of kind `platform` or `program` is applied from the form; one of kind `type` is applied by
# the form's own `labels:` line or by hand, and one of kind `external` by something that is not this
# repository.
table() {
  cat <<'ROWS'
bug|d73a4a|type||Something isn't working
enhancement|a2eeef|type||New feature or request
documentation|0075ca|type||Improvements or additions to documentation
duplicate|cfd3d7|type||This issue or pull request already exists
wontfix|ffffff|type||This will not be worked on
dependencies|0366d6|external||Dependabot's own, and applied by it
windows|c5def5|platform|Windows|On Windows
macos|c5def5|platform|macOS|On macOS
linux|c5def5|platform|Linux|On Linux
android|c5def5|platform|Android|On Android
quest|c5def5|platform|Meta Quest|On a Meta Quest headset
ios|c5def5|platform|iOS|On iOS
machine|d4c5f9|program|The machine (karaokemachine)|The machine itself
remote|d4c5f9|program|The remote (KM Remote)|The offline remote
package-builder|d4c5f9|program|km-package-builder|The curation tool
admin|d4c5f9|program|km-admin|The picture and bank tool
tools|d4c5f9|program|km-pack or another command-line tool|km-pack and the other command-line tools
api|d4c5f9|program|The HTTP API|The HTTP API
ROWS
}

# label|path prefix
#
# The folder a pull request label comes from. A path matches a row when it starts with the prefix,
# and a path no row matches gives no label. `crates/platform/`, `docs/`, `site/`, `icon/`,
# `.github/` and the rest of `tools/` serve every program, so a label from them would sort nothing.
paths() {
  cat <<'ROWS'
machine|crates/machine/karaokemachine/
machine|crates/machine/km-queue/
machine|crates/machine/km-banks/
machine|crates/machine/km-admin-pages/
machine|crates/machine/km-machine-ios/
machine|crates/playback/
machine|crates/song/
machine|ports/machine/
api|crates/machine/km-api/
remote|crates/remote/
remote|ports/remote/
package-builder|tools/cmd/km-package-builder/
admin|tools/cmd/assets/km-admin/
tools|tools/cmd/km-pack/
tools|tools/cmd/km-lyrics/
tools|tools/cmd/km-carols/
tools|tools/cmd/assets/km-wallpaper-pack/
windows|tools/platform/windows/
linux|tools/platform/linux/
macos|tools/platform/macos/
android|ports/machine/android/
android|ports/remote/android/
quest|ports/machine/android/app/src/headset/
android|crates/remote/km-remote-android/
android|crates/platform/km-androidlog/
ios|ports/machine/ios/
ios|ports/remote/ios/
ios|crates/machine/km-machine-ios/
ios|crates/remote/km-remote-ios/
ROWS
}

# The label a form option maps to, empty when the table has none.
label_for() { # $1 kind, $2 option
  table | awk -F'|' -v k="$1" -v o="$2" '$3 == k && $4 == o { print $1; exit }'
}

# The row a label sits on, empty when the table does not name it.
row_for() { # $1 label
  table | awk -F'|' -v n="$1" '$1 == n { print $0; exit }'
}

# Every label the table applies from a form answer.
facet_labels() {
  table | awk -F'|' '$3 == "platform" || $3 == "program" { print $1 }'
}

# The options a dropdown offers, one per line, read out of the form itself.
form_options() { # $1 form file, $2 dropdown id
  awk -v want="    id: $2" '
    $0 == want            { block = 1; next }
    block && /^  - type:/ { exit }
    block && $0 == "      options:" { opts = 1; next }
    opts && /^        - / { sub(/^        - /, ""); print; next }
    opts                  { exit }
  ' "$1"
}

# The labels a form's own `labels:` line applies.
form_labels() { # $1 form file
  sed -n 's/^labels: *\[\(.*\)\] *$/\1/p' "$1" |
    tr ',' '\n' |
    sed 's/^[[:space:]]*//; s/[[:space:]]*$//' |
    sed '/^$/d'
}

contains() { # $1 newline-separated list, $2 item
  [ -n "$1" ] || return 1
  printf '%s\n' "$1" | grep -qxF -- "$2"
}

found=0
fail() {
  echo "$self: $1" >&2
  found=$((found + 1))
}

cmd_check() {
  local id kind options option label name rowkind opt

  for id in platform program; do
    kind="$id"
    options="$(form_options "$bug_form" "$id")"
    if [ -z "$options" ]; then
      fail "the bug form's $id dropdown offers nothing, so its shape moved and this check reads it wrong"
      continue
    fi

    while IFS= read -r option; do
      # A rendered multi-select joins its answers with a comma, so an option carrying one cannot be
      # told from two options.
      case "$option" in
        *,*) fail "the bug form's $id option \"$option\" carries a comma, which is what the rendered answer separates on" ;;
      esac
      label="$(label_for "$kind" "$option")"
      [ -n "$label" ] || fail "the bug form offers the $id \"$option\" and the table has no label for it"
    done <<<"$options"

    while IFS='|' read -r name _ rowkind opt _; do
      [ "$rowkind" = "$kind" ] || continue
      contains "$options" "$opt" ||
        fail "the table maps \"$opt\" to $name and the bug form's $id dropdown does not offer it"
    done < <(table)
  done

  local form
  for form in "$bug_form" "$feature_form"; do
    while IFS= read -r label; do
      [ -n "$(row_for "$label")" ] ||
        fail "$form applies the label $label and the table does not name it"
    done < <(form_labels "$form")
  done

  # A folder renamed without its row gives every later pull request there no label, and nothing else
  # would say so.
  local prefix
  while IFS='|' read -r label prefix; do
    [ -n "$(row_for "$label")" ] ||
      fail "the path table maps $prefix to the label $label and the table does not name it"
    [ -n "$(git ls-files -- "$prefix" | head -n 1)" ] ||
      fail "the path table maps $prefix to $label and no tracked file is under it"
  done < <(paths)

  if [ "$found" -ne 0 ]; then
    printf '\n%s\n' "A label is declared in tools/dev/labels.sh. The bug form's answer applies it to an issue
through .github/workflows/issue-labels.yml, and the changed paths apply it to a pull request through
.github/workflows/pr-labels.yml. See docs/decisions/repository.md." >&2
    exit 1
  fi

  echo "$self: clean, every platform and program the bug form offers has a label, and every path row names a label and a tracked folder"
}

cmd_list() {
  printf '%-16s %-7s %-9s %s\n' NAME COLOR KIND 'THE FORM OPTION IT COMES FROM'
  local name color kind opt _desc
  while IFS='|' read -r name color kind opt _desc; do
    printf '%-16s %-7s %-9s %s\n' "$name" "$color" "$kind" "$opt"
  done < <(table)
}

cmd_sync() {
  local prune=0 dry=0 arg
  for arg in "$@"; do
    case "$arg" in
      --prune) prune=1 ;;
      --dry-run) dry=1 ;;
      *) echo "$self: unknown argument $arg" >&2; exit 1 ;;
    esac
  done

  command -v gh >/dev/null || { echo "$self: gh is not on PATH" >&2; exit 1; }

  local name color kind opt desc
  while IFS='|' read -r name color kind opt desc; do
    if [ "$dry" -eq 1 ]; then
      printf 'declare %-16s %s  %s\n' "$name" "$color" "$desc"
    else
      gh label create "$name" --color "$color" --description "$desc" --force
    fi
  done < <(table)

  [ "$prune" -eq 1 ] || return 0

  local known
  known="$(table | cut -d'|' -f1)"
  while IFS= read -r name; do
    [ -n "$name" ] || continue
    if contains "$known" "$name"; then
      continue
    fi
    if [ "$dry" -eq 1 ]; then
      printf 'delete  %s\n' "$name"
    else
      gh label delete "$name" --yes
    fi
  done < <(gh label list --limit 200 --json name --jq '.[].name')
}

case "${1-}" in
  table) table ;;
  paths) paths ;;
  list) cmd_list ;;
  check) cmd_check ;;
  sync) shift; cmd_sync "$@" ;;
  *)
    echo "usage: $self {table|paths|list|check|sync [--prune] [--dry-run]}" >&2
    exit 1
    ;;
esac
