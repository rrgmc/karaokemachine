#!/usr/bin/env bash
#
# A file a build job reads is a file whose change starts that job.
#
#   bash tools/dev/check-ci-paths.sh
#
# `.github/workflows/ci.yml` lets a pull request skip its build jobs when every changed file sits in
# an area no build reads. The `code` filter lists those areas and the `read` filter names the files
# inside them that a build does read. A file read from a skipped area and missing from `read` is a
# change that merges untested. This fails on three faults:
#
#   - Rust names a file by relative path, the file is in a skipped area, and `read` does not hold it.
#   - A build job runs a script that is in a skipped area, and `read` does not hold it.
#   - A pattern in `read` matches no tracked file, which is what a move leaves behind.
#
# **It reads the two filters out of the workflow, so the list lives in one place.** It wants one
# pattern a line under `code:`, and one brace list under `read:`.
#
# **What it cannot see is a path built at run time.** It matches a string literal that starts with
# `../`, which is how `include_str!`, `include_bytes!` and a test's `CARGO_MANIFEST_DIR` join reach
# out of a crate. A path assembled from parts has no such shape. The push to `master` runs every job
# and is what finds one of those.
#
# The standing decision is `master takes pull requests, and CI is one required check` in
# docs/decisions/repository.md.

set -uo pipefail

cd "$(dirname "$0")/../.." || exit 2

WORKFLOW=.github/workflows/ci.yml

# The patterns under one filter name, quotes off. `NF == 1` is what tells the filter `code:` from the
# job output `code: ${{ ... }}` above it.
patterns() {
  awk -v name="$1:" '
    NF == 1 && $1 == name { on = 1; next }
    on && $1 == "-" { p = $2; gsub(/\047/, "", p); print p; next }
    on { exit }' "$WORKFLOW" | tr -d '\r'
}

# A workflow glob as a shell pattern. Inside `[[ ]]` a `*` crosses a `/`, so `**/` and `/**` reduce
# to it.
shell_pattern() {
  printf '%s' "$1" | sed -e 's#\*\*/##g' -e 's#/\*\*#/*#g'
}

SKIPPED=()
while IFS= read -r pattern; do
  [ -n "$pattern" ] && SKIPPED+=("$(shell_pattern "${pattern#!}")")
done < <(patterns code)

READ_GLOBS=()
READ=()
while IFS= read -r pattern; do
  pattern="${pattern#\{}"
  pattern="${pattern%\}}"
  IFS=',' read -ra parts <<< "$pattern"
  for part in "${parts[@]}"; do
    READ_GLOBS+=("$part")
    READ+=("$(shell_pattern "$part")")
  done
done < <(patterns read)

# **Two empty lists are a broken run, not a clean one.** A filter renamed in the workflow would leave
# every path unskipped, and this would report clean having checked nothing.
if [ "${#SKIPPED[@]}" -eq 0 ] || [ "${#READ[@]}" -eq 0 ]; then
  echo "check-ci-paths: found no \`code\` or no \`read\` filter in $WORKFLOW." >&2
  exit 2
fi

matches() {
  local path="$1" pattern
  shift
  for pattern in "$@"; do
    # shellcheck disable=SC2053  # the right side is a pattern on purpose
    [[ "$path" == $pattern ]] && return 0
  done
  return 1
}

# True when a pull request changing only this path would skip the build jobs.
unbuilt() {
  matches "$1" "${SKIPPED[@]}" && ! matches "$1" "${READ[@]}"
}

# `a/b/../../c` as `c`, and empty when it climbs out of the repository.
normalize() {
  local part out=()
  local IFS=/
  for part in $1; do
    case "$part" in
      '' | .) ;;
      ..)
        [ "${#out[@]}" -eq 0 ] && return
        unset 'out[${#out[@]}-1]'
        ;;
      *) out+=("$part") ;;
    esac
  done
  printf '%s' "${out[*]}"
}

# The nearest folder at or above a file that holds a `Cargo.toml`, which is what
# `CARGO_MANIFEST_DIR` names.
crate_of() {
  local dir="${1%/*}"
  while [ -n "$dir" ] && [ "$dir" != "." ]; do
    [ -f "$dir/Cargo.toml" ] && { printf '%s' "$dir"; return; }
    [[ "$dir" == */* ]] || break
    dir="${dir%/*}"
  done
}

found=0

# One `grep` over every tracked Rust file. A literal is tried against the file's own folder, which
# is where `include_str!` starts, and against its crate's folder, which is where a test starts.
while IFS= read -r hit; do
  file="${hit%%:*}"
  rest="${hit#*:}"
  number="${rest%%:*}"
  line="${rest#*:}"
  [[ "$line" =~ ^[[:space:]]*// ]] && continue
  while [[ "$line" =~ \"((\.\./)+[^\"]*)\" ]]; do
    literal="${BASH_REMATCH[1]}"
    line="${line#*"${BASH_REMATCH[0]}"}"
    for base in "${file%/*}" "$(crate_of "$file")"; do
      [ -z "$base" ] && continue
      target=$(normalize "$base/$literal")
      [ -z "$target" ] && continue
      [ -e "$target" ] || continue
      # A folder takes its slash, so `tools/dev/remote` meets the pattern for what is inside it.
      [ -d "$target" ] && target="$target/"
      if unbuilt "$target"; then
        printf '%s:%s: reads %s\n' "$file" "$number" "$target"
        found=1
        break
      fi
    done
  done
done < <(git ls-files -z '*.rs' | xargs -0 grep -nE '"(\.\./)+[^"]*"' 2>/dev/null | tr -d '\r')

# A script a build job runs. `guards` is left out: it never skips, so what it runs needs no rule.
while IFS=$'\t' read -r job script; do
  [ -z "$script" ] && continue
  if unbuilt "$script"; then
    printf '%s: the %s job runs %s\n' "$WORKFLOW" "$job" "$script"
    found=1
  fi
done < <(awk '
  /^  [a-z-]+:[[:space:]]*$/ { job = $1; sub(/:$/, "", job); next }
  job != "" && job != "guards" && $0 !~ /^[[:space:]]*#/ {
    line = $0
    while (match(line, /tools\/[A-Za-z0-9_\/.-]+\.sh/)) {
      print job "\t" substr(line, RSTART, RLENGTH)
      line = substr(line, RSTART + RLENGTH)
    }
  }' "$WORKFLOW" | tr -d '\r' | sort -u)

for glob in "${READ_GLOBS[@]}"; do
  if [ -z "$(git ls-files -- ":(glob)$glob" | head -n 1)" ]; then
    printf '%s: the `read` pattern %s matches no tracked file\n' "$WORKFLOW" "$glob"
    found=1
  fi
done

if [ "$found" -ne 0 ]; then
  cat >&2 <<'WHY'

check-ci-paths: a build job reads a file that a pull request can change without a build.

  `master takes pull requests, and CI is one required check`, docs/decisions/repository.md

Add the file to the `read` filter of the `changes` job in .github/workflows/ci.yml, or move it out
of the areas the `code` filter skips. A `read` pattern that matches nothing names a file that moved.
WHY
  exit 1
fi

echo "check-ci-paths: clean, ${#READ_GLOBS[@]} files and folders read out of ${#SKIPPED[@]} skipped areas"
