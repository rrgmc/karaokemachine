# The build environment for a Debian package of the machine.
#
# Built and driven by tools/platform/linux/deb.sh; see "Shipping a Debian package" in docs/ARCHITECTURE.md.
#
# Why a container rather than the WSL Ubuntu sitting on the same box: a binary linked against
# Ubuntu 26.04's glibc 2.43 refuses to start on Debian 13, which has 2.41 -- and "the .deb does not
# run on Debian" is the one way this deliverable can be wrong while looking finished. The base image
# *is* the compatibility floor, stated in one line, and the image is also the only place the Linux
# toolchain has to exist: nothing is installed into the developer's own machine.
#
# Change this tag to change what the package targets. bookworm (Debian 12, glibc 2.36) builds a
# package that also runs on trixie; trixie's will not run on bookworm.
FROM debian:13-slim

# Everything that does not depend on this workspace, in one layer: what any Rust build needs
# (dpkg-dev is here for dpkg-shlibdeps, which turns the binary's links into the package's Depends
# field) plus the four the portable tarball wants.
#
# **These were two layers, with the tarball's four appended at the very bottom of this file** so that
# adding them would not invalidate rustup and `cargo install cargo-deb` for anybody who only ever
# built a package. That reasoning was right and its conclusion is now inverted, because the layer
# ordering below changed: this list is the *stable* one -- patchelf, nasm, xz-utils and a font have
# not changed since they were added and have no reason to -- so it belongs high, where it is
# protected, and `apt-deps.sh` belongs low, where it is not. Merging them also saves an `apt-get
# update` on every cold build.
#
# * patchelf rewrites RPATH on the staged files, which is how the tarball finds the ffmpeg beside it
#   without a launcher script wrapping the binary. tools/platform/linux/ffmpeg-lgpl.sh also uses it to read
#   DT_NEEDED, which is a more exact question than parsing ldd.
# * nasm is ffmpeg's assembler. Without it configure quietly builds a C-only decoder, which is not a
#   failure -- it is a 1080p file that drops frames on the appliance, found much later.
# * xz-utils unpacks the ffmpeg release, which is a .tar.xz.
# * fonts-dejavu-core is here as a *source file*, not as a runtime dependency: the tarball copies
#   DejaVuSans.ttf into its own assets as `fonts/karaoke.ttf`. The .deb can name a font package and
#   a tarball cannot, and km-display's system-font list is Debian- and Arch-shaped, so on Fedora it
#   finds nothing and the screen stays black with the reason only in the log.
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl build-essential pkg-config cmake ninja-build \
      dpkg-dev fakeroot \
      patchelf nasm xz-utils fonts-dejavu-core \
 && rm -rf /var/lib/apt/lists/*

# The same workaround CI sets, for the same reason: SDL3_ttf vendors a FreeType whose
# `cmake_minimum_required(VERSION 3.0)` CMake 4 refuses. trixie still ships CMake 3, so this is not
# needed today -- it is here so that a future base image bumping CMake does not break the build for a
# reason that has nothing to do with this project. A no-op when nothing needs it.
ENV CMAKE_POLICY_VERSION_MINIMUM=3.5

# Debian's own rustc is not used: rust-toolchain.toml pins an exact version, and the point of a
# pinned toolchain is that the build does not drift with whatever the distribution froze. `--profile
# minimal` keeps the image small; rustup adds the components rust-toolchain.toml asks for on first
# use.
#
# **The version is an argument rather than a literal, and this is the only reason it is not simply
# read from the file:** rust-toolchain.toml is not in the build context -- only `apt-deps.sh` is, and
# deliberately so -- and COPYing it in here would put a frequently-touched file above the rustup
# layer, which is the cache mistake the COPY note further down exists to describe. So `deb.sh` parses
# the pin and passes it, `tools/platform/linux/image-tag.sh` hashes the file into the image tag so a
# bump cannot reuse an image built on the old compiler, and `tools/dev/check-toolchain-pin.sh` fails
# if a literal version reappears on the line below.
#
# `ARG` sits immediately above the RUN rather than at the top of the file on purpose: an argument
# invalidates every layer after it is declared, and only rustup and what follows depends on this.
ARG RUST_VERSION
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN test -n "$RUST_VERSION" \
      || { echo "RUST_VERSION is required; tools/platform/linux/deb.sh reads it from rust-toolchain.toml" >&2; exit 1; } \
 && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --no-modify-path --profile minimal --default-toolchain "$RUST_VERSION"

# The packaging tool. --locked so a fresh image build cannot pick up a broken new dependency of
# cargo-deb itself, which would look like a failure of this project.
#
# Deliberately not pinned to a major version: this image has never been built (see "Docker on this
# box" in CLAUDE.local.md), so a pin would be a guess about which major is current, and a wrong guess
# fails the image build for a reason that has nothing to do with this project. Pin it once the build
# has run and the working version is known.
RUN cargo install cargo-deb --locked

# **This workspace's own C dependencies, and they are last on purpose.** The list is not written out
# here: the same script CI runs installs them, so the image and the CI runner cannot drift apart.
# That mattered the moment `video` existed -- this image had no ffmpeg, so `tools/platform/linux/check.sh`,
# which builds with video, could not have passed on it.
#
# `--video` deliberately: check.sh is meant to be the Linux third of CI on a developer's machine, and
# a local check that silently skipped the one feature with a C dependency would be checking the easy
# part. It costs about 40 MB in an image that already carries a Rust toolchain.
#
# **Why the COPY moved down here**, from above rustup where it used to sit. Docker's layer cache is
# keyed on the parent layer's ID and the instruction string -- and, for a COPY, the checksum of what
# is copied. `-t` is a name applied at the end and has no bearing on cache lookup. So with the COPY
# up top, editing apt-deps.sh missed that layer and therefore rebuilt rustup, `cargo install
# cargo-deb` and everything after it: several minutes, for a change that has nothing to do with any
# of them. Down here, an edit re-runs these two instructions and nothing else.
#
# Note this is *not* defeated by tools/platform/linux/image-tag.sh hashing both files into the tag. A new tag
# is a new name, not a cold cache: every layer above still matches on instruction and parent, so it
# still hits. The tag changing is what stops two checkouts clobbering each other; the ordering is
# what stops an unrelated rebuild. Different jobs.
COPY apt-deps.sh /tmp/apt-deps.sh
RUN /tmp/apt-deps.sh --video \
 && rm -rf /var/lib/apt/lists/* /tmp/apt-deps.sh

# Where the build writes. Kept off the mounted source tree so a Linux build cannot collide with the
# Windows target/ directory in the same checkout, and so it can live in a named volume and be reused.
ENV CARGO_TARGET_DIR=/build/target

# **Incremental compilation is off, and the volume is why.** Incremental exists to make the *second*
# build of an edited crate cheap, and nothing here is edited twice: a container starts from a
# bind-mounted checkout, builds once, and ends. What it leaves behind instead is a per-crate cache
# under the debug profile that grows without bound across checkouts and toolchains -- a directory of
# very many small files, written on every run and read back on none.
#
# It reaches `check.sh` alone. The release profile the package is built from has incremental off by
# default, so the .deb and the tarball are unaffected either way.
#
# Set here rather than in check.sh's `docker run -e`, because the rule is a property of this build
# environment rather than of one caller, and an environment every caller shares should say it once.
# Last in the file for the cache reason the COPY note above gives: an ENV this low invalidates
# nothing above it, so adding it costs no rebuild.
ENV CARGO_INCREMENTAL=0

WORKDIR /src
