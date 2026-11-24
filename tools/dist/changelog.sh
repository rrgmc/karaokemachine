#!/usr/bin/env bash
#
# The changelog's unreleased entries are files in `changelog.d/`, one per change, and a release
# gathers them into `CHANGELOG.md`.
#
#   bash tools/dist/changelog.sh check               # exit 1 and name every fragment that is malformed
#   bash tools/dist/changelog.sh preview             # print the section the next release would write
#   bash tools/dist/changelog.sh release X.Y.Z [day] # write that section into CHANGELOG.md, and
#                                                    # git rm the fragments it took
#
# **A fragment is `changelog.d/<name>.<type>.md`.** `<name>` is lowercase letters, digits and
# hyphens -- the branch's name does. `<type>` is one of Keep a Changelog's six sections: `added`,
# `changed`, `deprecated`, `removed`, `fixed` or `security`. The file holds one or more list items
# exactly as they will appear in the changelog: each opens `- ` in the first column, and its
# continuation lines are indented two spaces. No heading; the type is the heading.
#
# **This is why it is files and not a section.** Every pull request that edited `## [Unreleased]`
# edited the same few lines, so any two of them conflicted. A new file in a folder conflicts with
# nothing. The standing decision is `Unreleased entries are a file each, and a release gathers them`
# in docs/decisions/repository.md.
#
# **Within a section, entries come in the order their files were first committed**, which is the
# order their pull requests merged. A fragment not yet committed comes last. Ties fall to the name.
#
# `release` puts the new section above the newest released one, under the `## [Unreleased]`
# paragraph that points here, and leaves that paragraph alone. `day` defaults to today, UTC.
# `task check` and CI's `guards` job run `check`.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

self='changelog'
dir='changelog.d'
changelog='CHANGELOG.md'
types=(added changed deprecated removed fixed security)
headings=(Added Changed Deprecated Removed Fixed Security)
export LC_ALL=C

usage() { sed -n '7,10p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2; exit 2; }

# Every fragment, in the folder. README.md is the folder's own explanation and is not one.
fragments() {
  local f
  for f in "$dir"/*; do
    [ -e "$f" ] || continue
    [ "${f##*/}" = README.md ] && continue
    printf '%s\n' "$f"
  done
}

# The problems with one file, a line each; nothing when it is well formed.
problems() {
  local f="$1" base="${1##*/}" type
  if [[ ! "$base" =~ ^[a-z0-9][a-z0-9-]*\.([a-z]+)\.md$ ]]; then
    echo "$f: the name is not <name>.<type>.md, with <name> in lowercase letters, digits and hyphens"
    return
  fi
  type="${BASH_REMATCH[1]}"
  if [[ " ${types[*]} " != *" $type "* ]]; then
    echo "$f: '$type' is not one of: ${types[*]}"
  fi
  if [ ! -s "$f" ] || ! grep -q '[^[:space:]]' "$f"; then
    echo "$f: it is empty"
    return
  fi
  if [ -n "$(tail -c1 "$f")" ]; then
    echo "$f: the last line has no newline"
  fi
  awk -v f="$f" '
    /\r$/ { print f ":" NR ": a carriage return -- save it with LF line endings"; exit }
    !started && /^[[:space:]]*$/ { next }
    !started && !/^- / { print f ":" NR ": the first line must open a list item with \"- \""; exit }
    { started = 1 }
    /^#/ { print f ":" NR ": a heading -- the type in the file name is the heading"; exit }
    /^[^-[:space:]]/ || /^-[^ ]/ { print f ":" NR ": not a list item, and not indented under one"; exit }
  ' "$f"
}

check() {
  local f bad=0 out count=0
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    count=$((count + 1))
    out="$(problems "$f")"
    if [ -n "$out" ]; then
      printf '%s: %s\n' "$self" "$out" >&2
      bad=1
    fi
  done < <(fragments)
  if [ "$bad" -ne 0 ]; then
    printf '\n%s\n' "A fragment is changelog.d/<name>.<type>.md, holding list items as they appear in the
changelog. changelog.d/README.md has an example." >&2
    exit 1
  fi
  echo "$self: clean, $count unreleased entries in $dir/"
}

# The fragments of one type, oldest commit first: "<seconds> <path>", then the path alone.
ordered() {
  local type="$1" f when
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    [[ "$f" == *".$type.md" ]] || continue
    when="$(git log --diff-filter=A --format=%ct -- "$f" 2>/dev/null | tail -n1)"
    printf '%s %s\n' "${when:-9999999999}" "$f"
  done < <(fragments) | sort -k1,1n -k2,2 | cut -d' ' -f2-
}

# The body of the section: a `### Heading` per type that has entries.
section() {
  local i f first=1 files
  for i in "${!types[@]}"; do
    files="$(ordered "${types[$i]}")"
    [ -n "$files" ] || continue
    [ "$first" -eq 1 ] || echo
    first=0
    printf '### %s\n\n' "${headings[$i]}"
    while IFS= read -r f; do
      # The item alone: leading and trailing blank lines off.
      awk 'NF { started = 1 } started' "$f" | awk '{ lines[NR] = $0 } NF { last = NR } END { for (i = 1; i <= last; i++) print lines[i] }'
    done <<< "$files"
  done
}

preview() {
  check >/dev/null
  local body
  body="$(section)"
  if [ -z "$body" ]; then
    echo "$self: no unreleased entries in $dir/" >&2
    exit 0
  fi
  printf '%s\n' "$body"
}

release() {
  local version="${1:-}" day="${2:-$(date -u +%F)}" body tmp
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "$self: give the version as X.Y.Z" >&2; usage; }
  [[ "$day" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || { echo "$self: give the day as YYYY-MM-DD" >&2; exit 2; }
  if grep -qF "## [$version]" "$changelog"; then
    echo "$self: $changelog already has a section for $version" >&2
    exit 1
  fi
  check >/dev/null
  body="$(section)"
  if [ -z "$body" ]; then
    echo "$self: no unreleased entries in $dir/ -- a release is not cut without one" >&2
    exit 1
  fi

  tmp="$(mktemp)"
  # Above the first released section; at the end when there is none yet.
  awk -v head="## [$version] - $day" -v body="$body" '
    !done && /^## \[[0-9]/ { print head "\n\n" body "\n"; done = 1 }
    { print }
    END { if (!done) print "\n" head "\n\n" body }
  ' "$changelog" > "$tmp"
  mv "$tmp" "$changelog"

  local f n=0
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    if git ls-files --error-unmatch -- "$f" >/dev/null 2>&1; then
      git rm -q -- "$f"
    else
      rm -- "$f"
    fi
    n=$((n + 1))
  done < <(fragments)
  echo "$self: wrote $version, $day into $changelog from $n entries, and removed them from $dir/"
}

case "${1:-}" in
  check) check ;;
  preview) preview ;;
  release) shift; release "$@" ;;
  *) usage ;;
esac
