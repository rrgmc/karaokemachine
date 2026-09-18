#!/usr/bin/env bash
#
# The compiler version is written down once, and this is what keeps that true.
#
#   tools/dev/check-toolchain-pin.sh          # exit 1 and name every disagreement
#
# `rust-toolchain.toml` holds the pin. Three tracked files have to repeat the number because no
# format lets them derive it -- cargo's `rust-version` in two manifests, and one sentence of
# docs/learning-rust.md -- and two more must NOT repeat it, because they are supposed to read the
# file instead. Both halves are checked here.
#
# The standing decision is `The Rust toolchain is pinned exactly` in docs/decisions/repository.md.
# This exists for the reason `tools/platform/linux/check.sh` asserts the two feature lists agree: a
# copy nothing compares is a copy that drifts, and the failure it produces is a build on a compiler
# nobody chose.
#
# `task check` runs it alongside the local-reference check, ahead of fmt, because it reads five files
# and says one line. CI runs it in the `guards` job.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
[ -f Cargo.toml ] && [ -d crates ] || { echo "${0##*/}: not at the repository root -- landed in $PWD" >&2; exit 1; }

self='check-toolchain-pin'
found=0

fail() {
  printf '%s: %s\n' "$self" "$1" >&2
  found=1
}

# -- The pin itself ------------------------------------------------------------------------------

# `channel = "1.98.1"`, and the quotes are required by the file format so this is not guesswork.
channel="$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml)"

if [ -z "$channel" ]; then
  echo "$self: rust-toolchain.toml has no channel to read" >&2
  exit 1
fi

# An exact three-part version, which is the decision rather than a formatting preference: `stable`
# is what this repository used until 2026-09-03 and what the decision moved away from, and `1.98`
# would let a patch release change the compiler without changing a tracked file.
if ! printf '%s' "$channel" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  fail "rust-toolchain.toml pins '$channel', which is not an exact x.y.z version.
       A floating channel lets a clippy lint added upstream fail CI on a branch that changed
       nothing relevant. See 'The Rust toolchain is pinned exactly' in docs/decisions/repository.md."
fi

# -- The three files that must carry the same number ---------------------------------------------

# `rust-version` in both workspace roots. The assets workspace has its own copy rather than
# inheriting, because the root `Cargo.toml` excludes it -- which is exactly why it is easy to miss.
# Its members inherit from it in turn, so this one line covers everything under `tools/cmd/assets`.
for manifest in Cargo.toml tools/cmd/assets/Cargo.toml; do
  declared="$(sed -n 's/^rust-version = "\([^"]*\)".*/\1/p' "$manifest")"
  if [ -z "$declared" ]; then
    fail "$manifest declares no rust-version, which should equal the pin ($channel)"
  elif [ "$declared" != "$channel" ]; then
    fail "$manifest says rust-version = \"$declared\" but the pin is $channel"
  fi
done

# One sentence of prose, and it is checked because it went stale once: it said 1.85 for as long as
# the manifests did. A grep for the whole assignment rather than the bare number, so that a version
# mentioned for some other reason in that document does not satisfy it.
if ! grep -qF "rust-version = \"$channel\"" docs/learning-rust.md; then
  fail "docs/learning-rust.md does not name rust-version = \"$channel\""
fi

# -- ...and the two that must not ----------------------------------------------------------------

# The Dockerfile takes the version from `tools/platform/linux/deb.sh`, which parses it out of
# rust-toolchain.toml. A literal here is a second pin that a bump would not move.
if grep -Ev '^[[:space:]]*#' tools/platform/linux/Dockerfile \
     | grep -Eq -- '--default-toolchain[[:space:]]+([0-9]|stable|beta|nightly)'; then
  fail "tools/platform/linux/Dockerfile names a toolchain literally; it should pass \"\$RUST_VERSION\""
fi

# The workflows install by running `rustup toolchain install` with no arguments, which resolves
# rust-toolchain.toml. `dtolnay/rust-toolchain` cannot read that file -- its `toolchain` input is
# required -- so re-adding the action necessarily reintroduces a copy of the number.
#
# Anchored on `uses:` rather than the bare name, so a comment explaining why may still name it.
for wf in .github/workflows/*.yml; do
  if grep -Eq '^[[:space:]]*-?[[:space:]]*uses:[[:space:]]*dtolnay/rust-toolchain' "$wf"; then
    fail "${wf} uses dtolnay/rust-toolchain, which cannot read rust-toolchain.toml.
       Install with 'rustup toolchain install --no-self-update' instead, which resolves the file."
  fi
done

if [ "$found" -ne 0 ]; then
  printf '\n%s\n' "The pin is $channel. Bumping it means rust-toolchain.toml, both rust-version keys and the
sentence in docs/learning-rust.md, together -- 'Bumping the Rust toolchain' in BUILDING.md." >&2
  exit 1
fi

echo "$self: clean, pinned to $channel"
