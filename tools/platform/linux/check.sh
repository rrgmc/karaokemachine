#!/usr/bin/env bash
#
# Runs the checks CI's Linux job runs, in a container, on this machine.
#
#   tools/platform/linux/check.sh            # fmt, clippy and the whole test suite
#   tools/platform/linux/check.sh --quick    # clippy and tests, skipping the format check
#   tools/platform/linux/check.sh --shell    # a shell in the image, for poking at a failure
#
# **Why this exists.** This is developed on Windows, and three things only fail on Linux: a
# `Path` that reads `\` as an ordinary character, a dependency that wants a system library nobody
# here has, and a `#[cfg]` that is wrong about which platform it is on. All three have happened, and
# each cost a CI round trip to discover -- and CI is not always available, whether because a run
# takes ten minutes or because an account has run out of minutes. The Debian image that
# `tools/platform/linux/deb.sh` already builds has a full Rust toolchain and every library the workspace
# links, so the same checks can run here in about a minute against a warm cache.
#
# It is not a replacement for CI. macOS has no local equivalent, and the Windows checks are what
# `cargo km-test` on this machine already is -- the same feature list, since the video build is the
# default one everywhere it is named. It is the Linux third.
#
# `tools/cmd/assets` is checked separately because it is a second workspace, excluded on purpose;
# see the `exclude` note in the root Cargo.toml.
#
# Requirements: Docker. On Windows that means Docker Desktop running.

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/platform/linux/image-tag.sh          # sets IMAGE + RUST_VERSION, hashed from Dockerfile, apt-deps.sh, rust-toolchain.toml
VOLUME="karaokemachine-deb-build"   # shared with deb.sh, so the cache is already warm
QUICK=0
SHELL_ONLY=0

for arg in "$@"; do
  case "$arg" in
    --quick) QUICK=1 ;;
    --shell) SHELL_ONLY=1 ;;
    *) echo "check: unknown option $arg" >&2; exit 2 ;;
  esac
done

# See the note in deb.sh: MSYS rewrites Unix-looking paths before a Windows executable sees them,
# which mounts the wrong directory entirely.
export MSYS2_ARG_CONV_EXCL='*'

host_path() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}

if ! docker version >/dev/null 2>&1; then
  echo "check: cannot reach the Docker daemon." >&2
  echo "       On Windows, start Docker Desktop and wait for it to say it is running." >&2
  exit 1
fi

# Built only if it is missing. The image is the same one deb.sh uses, so this normally prints a
# handful of cached layers and moves on.
#
# The check-then-act is safe because the tag names its own content (see
# tools/platform/linux/image-tag.sh): two checkouts can both find the image absent and both build it,
# and the worst case is two builds producing an identical image -- wasted work rather than a wrong
# answer. With a fixed tag either could re-tag it under the other.
if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "== image (first run)"
  docker build "${IMAGE_BUILD_ARGS[@]}" -t "$IMAGE" tools/platform/linux
  echo
fi

if [ "$SHELL_ONLY" = "1" ]; then
  # Records the owner before handing over the prompt, because anything you build at that prompt lands
  # in the shared /build/target exactly as a scripted run would -- and an unrecorded writer is the one
  # thing that makes the release paths' skipped clean unsafe. `exec bash` after it so the shell is
  # still the container's foreground process and still gets the tty.
  exec docker run --rm -it \
    -e "KM_CHECKOUT=$(host_path "$PWD")" \
    -v "$(host_path "$PWD")":/src \
    -v "$VOLUME":/build \
    -w /src "$IMAGE" bash -c 'tools/platform/linux/clean-checkout.sh --record-only; exec bash'
fi

# `--locked` throughout: a check that quietly resolved a different dependency tree than the one
# committed would not be checking what is committed.
script='
set -euo pipefail
cd /src
# **Records that this checkout is what is about to write into the shared /build/target, and cleans
# nothing.** check.sh has never cleaned and still does not -- for a check, a stale artifact is a
# baffling compiler error you recognize and clear by hand, which is a cost worth paying to keep the
# warm cache. But the release paths now *skip* their clean when the stamp says they own the volume,
# and a check that wrote another checkout'"'"'s artifacts in here without saying so would make that skip
# unsafe. See the invariant in clean-checkout.sh: every writer records, only releases react.
tools/platform/linux/clean-checkout.sh --record-only
if [ "'"$QUICK"'" != "1" ]; then
  echo "== formatting"
  cargo fmt --all --check
  cargo fmt --manifest-path tools/cmd/assets/Cargo.toml --all --check
fi
# **Every feature except one, named rather than swept up by `--all-features`.** The list and the
# reason it cannot be `--all-features` on Linux are in tools/setup/features.sh; this sources it rather than
# spelling it, so there is one place to add a feature to.
. tools/setup/features.sh

# **Both alias lists have to be checked, because a cargo alias cannot source a shell file.**
# `.cargo/config.toml` spells the same two lists a third time, and a copy nothing compares is a copy
# that drifts -- which is exactly how `--all-features` went on being wrong in two places for a day
# without anybody noticing. Same bargain apt-deps.sh makes: one definition, and the consumers that
# cannot read it are asserted against it.
#
# **The unsuffixed names are the video ones** since the `Video is the default build` decision, so
# `km-test` here is the video alias and `km-test-no-video` is a separate entry checked below.
#
# **The list is bounded by a quote or a space on each side, and that boundary is the whole reason the
# two do not match each other**: the no-video list is a prefix of the video one, and what follows it
# there is a comma. An alias is one string rather than an array of arguments, so the list ends at the
# closing quote where it is last and at a space where `-- -D warnings` follows it.
check_alias() {
  if ! grep -q "^$1 = .*[\" ]$2[\" ]" .cargo/config.toml; then
    echo "check: .cargo/config.toml'"'"'s $1 does not carry the matching list from tools/setup/features.sh" >&2
    echo "       expected: $2" >&2
    grep "^$1 = " .cargo/config.toml >&2 || echo "       (the alias is missing entirely)" >&2
    exit 1
  fi
}
for alias in km-test km-lint km-build; do
  check_alias "$alias" "$KM_FEATURES_VIDEO"
done
# ...and the twins that decline video. **Nothing checked the plain list until now**, which was
# tolerable while it was the default and spelled in one obvious place; the rename is exactly the
# moment that gap becomes easy to widen without noticing. `km-build-no-video` was the last one left
# out -- it carried no features at all, so there was nothing to assert and nothing to notice when
# that made it compile a different tree from the one its test twin runs. All six are checked now.
for alias in km-test-no-video km-lint-no-video km-build-no-video; do
  check_alias "$alias" "$KM_FEATURES"
done

echo "== clippy"
cargo clippy --workspace --features "$KM_FEATURES_VIDEO" --all-targets --locked -- -D warnings
cargo clippy --manifest-path tools/cmd/assets/Cargo.toml --workspace --all-targets --locked -- -D warnings
echo "== tests"
cargo test --workspace --features "$KM_FEATURES_VIDEO" --locked
cargo test --manifest-path tools/cmd/assets/Cargo.toml --workspace --locked
'

docker run --rm -i \
  -e "KM_CHECKOUT=$(host_path "$PWD")" \
  -v "$(host_path "$PWD")":/src \
  -v "$VOLUME":/build \
  "$IMAGE" bash -c "$script"

echo
echo "linux checks passed"
