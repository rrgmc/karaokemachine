# shellcheck shell=bash
#
# The Docker image tag for the Debian build image, derived from what the image is built from.
#
#   . tools/platform/linux/image-tag.sh     # from the repository root
#
# Sets three things: IMAGE, RUST_VERSION, and IMAGE_BUILD_ARGS to splice into a `docker build`.
#
# Sourced by check.sh, deb.sh, tarball.sh and prewarm.sh -- the four scripts that build the image.
# No shebang and no `set -euo pipefail`, for the reason tools/dist/common.sh gives: a sourced file
# must not change the caller's shell.
#
# **Why the tag is a hash rather than a constant.** All three scripts build from their own
# `tools/platform/linux` directory, and deb.sh and tarball.sh do it unconditionally on every run.
# One constant tag plus more than one checkout means the last build wins: a worktree on a branch
# that touched the Dockerfile silently re-tags the image another checkout is about to run, and
# `docker run "$IMAGE"` resolves the name *after* the peer may have moved it. A check-then-act on
# top -- `docker image inspect` then build -- lets two first runs both decide the image is missing.
#
# Naming the image after its own content removes the question rather than sequencing it. Identical
# build inputs produce one tag and share it, which is the caching that was wanted; different inputs
# produce different tags that cannot overwrite each other. A second build of an already-built tag is
# a layer-cache hit, so this costs nothing in the common case.
#
# **The three inputs are the whole image.** `tools/platform/linux/Dockerfile`, the `apt-deps.sh` it
# COPYs (Dockerfile:31) -- the only file that enters the build context -- and `rust-toolchain.toml`.
# Hashing the whole directory would churn the tag every time check.sh or deb.sh was edited, which
# changes nothing about the image.
#
# **`rust-toolchain.toml` is here because the compiler version is a build argument, not a literal.**
# deb.sh parses the pin out of that file and passes it as `RUST_VERSION`, so the Dockerfile's text no
# longer changes when the toolchain does -- and without this line a bump would leave the tag
# identical while the image it names still had the old compiler baked in, which is precisely the
# last-build-wins failure this file exists to remove. It is not in the build context and is not
# COPYed; it is hashed because it decides what gets installed.
#
# One thing this deliberately does not fix: Dockerfile's `cargo install cargo-deb` is unpinned on
# purpose, so two builds of identical inputs can still install different versions. Content-addressing
# the tag is about two checkouts not clobbering each other, not about reproducibility.
# The pin, parsed once here rather than in each of the four scripts that build the image --
# check.sh, deb.sh, tarball.sh and prewarm.sh. They all source this file already, so the version and
# the tag that has to cover it are decided in the same place and cannot disagree. Spliced at each
# `docker build` as "${IMAGE_BUILD_ARGS[@]}".
RUST_VERSION="$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml)"
IMAGE_BUILD_ARGS=(--build-arg "RUST_VERSION=$RUST_VERSION")

IMAGE="karaokemachine-deb:$(
  cat tools/platform/linux/Dockerfile tools/platform/linux/apt-deps.sh rust-toolchain.toml | {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum; else shasum -a 256; fi
  } | cut -c1-12
)"
