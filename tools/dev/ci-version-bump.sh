#!/usr/bin/env bash
#
# Whether a pull request's code changes are the version number moving, and nothing else.
#
#   bash tools/dev/ci-version-bump.sh <base> <head> <code file>...
#
# Prints `true` or `false`. The `changes` job of `.github/workflows/ci.yml` passes it the files its
# `code` filter matched, and skips the build jobs on `true`. A release raises one number in two
# manifests, and cargo rewrites the workspace members in two lockfiles. Building every platform to
# prove that is seven minutes for nothing, and the push to `master` and the tag build it anyway.
#
# **`true` needs all of this**, and anything else is `false`:
#
#   - `[workspace.package] version` in the root `Cargo.toml` differs between base and head.
#   - Every code file is one of the two manifests or the two lockfiles.
#   - Every line changed in them is `version = "<old>"` going out or `version = "<new>"` coming in.
#
# A third-party crate moving in a lockfile changes its `checksum` line too, so it cannot pass for a
# member. The standing decision is `master takes pull requests, and CI is one required check` in
# docs/decisions/repository.md.

set -uo pipefail

cd "$(dirname "$0")/../.." || exit 2

if [ "$#" -lt 2 ]; then
  echo "usage: ci-version-bump.sh <base> <head> <code file>..." >&2
  exit 2
fi

base="$1"
head="$2"
shift 2

version_at() {
  git show "$1:Cargo.toml" 2>/dev/null | tr -d '\r' | awk '
    /^\[/ { on = ($0 == "[workspace.package]"); next }
    on && $1 == "version" && $2 == "=" { v = $3; gsub(/"/, "", v); print v; exit }'
}

old=$(version_at "$base")
new=$(version_at "$head")

if [ -z "$old" ] || [ -z "$new" ] || [ "$old" = "$new" ] || [ "$#" -eq 0 ]; then
  echo false
  exit 0
fi

for file in "$@"; do
  case "$file" in
    Cargo.toml | Cargo.lock | tools/cmd/assets/Cargo.toml | tools/cmd/assets/Cargo.lock) ;;
    *)
      echo false
      exit 0
      ;;
  esac

  other=$(git diff -U0 "$base" "$head" -- "$file" | tr -d '\r' |
    grep -E '^[-+]' | grep -vE '^(\+\+\+|---) ' |
    grep -cvxF -e "-version = \"$old\"" -e "+version = \"$new\"")
  if [ "$other" -ne 0 ]; then
    echo false
    exit 0
  fi
done

echo true
