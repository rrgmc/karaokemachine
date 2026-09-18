#!/usr/bin/env bash
#
# The product version is written down twice, and this is what keeps the two copies equal.
#
#   tools/dev/check-version-pin.sh          # exit 1 and name every disagreement
#
# `Cargo.toml` holds it in `[workspace.package]` and every crate takes it with
# `version.workspace = true`. `tools/cmd/assets/Cargo.toml` has to hold its own copy, because the root
# manifest `exclude`s that directory and nothing -- not a workspace key, not a `[patch]`, not a
# `.cargo/config.toml` -- is inherited across an exclusion boundary. Two copies of one number is
# exactly the shape `tools/dev/check-toolchain-pin.sh` exists for one line further down the same
# manifests, and this is its sibling.
#
# **What it is protecting is a folder name.** `tools/dist/cmd.sh` names a staged folder from the
# version the binary itself reports, and `tools/dist/bin.sh` goes looking for that folder using the
# version a manifest reports. While the two workspaces disagreed, those were different numbers for
# `km-admin` and `km-wallpaper-pack`, and every script that walks `dist/` needed an arm per program to
# know it. `tools/dist/clean.sh` was given both arms; `bin.sh` was given one -- so `task dist:bin`
# staged km-admin, then said `nothing staged for km-admin` about the folder it had just written.
# The standing decision is `One version number for the whole repository` in docs/decisions/repository.md.
#
# `task check` runs it beside the other checks in this directory, ahead of fmt, because it reads four
# files and says one line. CI runs it in the `guards` job.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

self='check-version-pin'
found=0
assets=tools/cmd/assets

fail() {
  printf '%s: %s\n' "$self" "$1" >&2
  found=1
}

# -- the two roots -------------------------------------------------------------------------------

# `sed` rather than `cargo pkgid`, for the reason `check-toolchain-pin.sh` reads files too: this has
# to run before anything compiles, it must be able to say *which file* is wrong, and asking cargo
# about a manifest whose own version is the thing in question is a slower way to learn less.
#
# Anchored at the start of the line so a `version = "..."` under `[workspace.dependencies]` -- where
# every entry is indented or inline -- cannot be read as the workspace's own.
root_version() { # <manifest>  -> prints the version, or nothing
  sed -n 's/^version = "\([^"]*\)".*/\1/p' "$1" | head -1
}

version="$(root_version Cargo.toml)"

if [ -z "$version" ]; then
  echo "$self: Cargo.toml has no [workspace.package] version to read" >&2
  exit 1
fi

declared="$(root_version "$assets/Cargo.toml")"
if [ -z "$declared" ]; then
  fail "$assets/Cargo.toml declares no version, which should equal the root's ($version).
       Its members take it with 'version.workspace = true' and have nothing to inherit without it."
elif [ "$declared" != "$version" ]; then
  fail "$assets/Cargo.toml says version = \"$declared\" but the root says $version"
fi

# -- ...and the members, which must not carry one --------------------------------------------------

# Comparing the two roots is not enough on its own: a member that spells its own version out would
# pass that comparison while producing exactly the folder name this exists to prevent, and both
# members are capable of it.
for manifest in "$assets"/*/Cargo.toml; do
  [ -f "$manifest" ] || continue
  if [ -n "$(root_version "$manifest")" ]; then
    fail "$manifest names a version of its own; it should say 'version.workspace = true'"
  elif ! grep -q '^version\.workspace = true' "$manifest"; then
    fail "$manifest neither inherits the workspace version nor declares one"
  fi
done

if [ "$found" -ne 0 ]; then
  printf '\n%s\n' "The version is $version. It lives in two [workspace.package] tables -- Cargo.toml and
$assets/Cargo.toml -- and they move together; every crate under either takes it with
'version.workspace = true'. See 'Bumping the version' in BUILDING.md." >&2
  exit 1
fi

echo "$self: clean, one version everywhere -- $version"
