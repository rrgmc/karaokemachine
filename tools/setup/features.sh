# shellcheck shell=bash
#
# The workspace's cargo features, named once, for the commands that want all of them.
#
#   . tools/setup/features.sh          # sets KM_FEATURES and KM_FEATURES_VIDEO
#
# Sourced, never executed -- so no shebang and no `set -euo pipefail`, the same rule
# tools/dist/common.sh and tools/setup/ffmpeg-pin.sh follow: a sourced file that sets shell options changes
# the caller's shell.
#
# **This exists because `--all-features` is wrong on Linux and cannot be made right.** It turns on
# `km-package-builder/desktop` **and, since M21, `km-remote/desktop`** -- both of which pull in
# `wry`, which links libwebkit2gtk at load time -- and `tools/platform/linux/apt-deps.sh` deliberately does
# not install that, because a Linux build of either tool never has a window. That is a standing
# decision (`The package builder's window` and `The remote's window, and its portable core` in
# docs/decisions/), not a gap to fill: a build carrying wry would not *start* on a machine without the
# library, and `--browser` cannot rescue it because the failure is in the dynamic loader before
# `main`. So the one flag that means "everything" asks for something the platform is deliberately
# unable to provide, and the fix is to stop asking.
#
# **Neither `desktop` feature appears in the lists below, and that is the whole mechanism.** The
# exclusion is by omission rather than by a rule anybody has to remember, so adding a window to a
# third tool means doing nothing here -- but adding a `desktop` to either list would silently
# reintroduce the fault on Linux only.
#
# It failed in `javascriptcore-rs-sys`'s build script naming a missing `.pc` file, which reads like a
# machine that needs a package rather than like a feature that should never have been on. That is why
# it went unnoticed in two places at once.
#
# The same list is spelled a third time in `.cargo/config.toml`, because a cargo alias cannot source
# a shell file. `tools/platform/linux/check.sh` asserts the three agree rather than trusting them to, which is
# the same bargain `apt-deps.sh` makes with the Dockerfile and CI.
#
# **Adding a feature to any crate means adding it here.** That is the cost of being explicit, and it
# is the cost CLAUDE.md already accepts for CI: "Feature-explicit, never `--all-features`".

# The two testing features, which are off by default because neither should ship in a release binary,
# and which the integration tests, the `write_fixtures` example and the in-memory machine behind
# `dev_server` all need. Both lists below carry them.
#
# **On its own this is the exception rather than the everyday list** -- it is what a machine with
# no ffmpeg and no libclang runs: a fresh clone, CI's Windows and macOS jobs, and the `-no-video`
# aliases that check the video-less tree still compiles. `KM_FEATURES_VIDEO` is what `cargo km-test`,
# `cargo km-lint` and `task check` run. See the `Video is the default build` decision in
# docs/decisions/song-sources.md.
#
# The names describe what is in each list, not which one is the default.
#
# `.github/workflows/ci.yml` runs the cargo aliases rather than sourcing this file, because its
# Windows and macOS steps run in `pwsh`, where `. tools/setup/features.sh` is a parse error. The
# aliases are the second copy, and check.sh asserts them against the lines here.
KM_FEATURES="km-song/testing,km-api/testing"

# ...and video: **the everyday list**, and what the unsuffixed commands run. `tools/platform/linux/check.sh`'s
# Docker image (which runs `apt-deps.sh --video`) and the `km-build`, `km-test` and `km-lint` aliases
# in `.cargo/config.toml`, which check.sh asserts against this line and CI's Linux job runs.
# Every feature `--all-features` reaches except the one that cannot build on Linux.
#
# The price of it being the default is that these commands need ffmpeg's development libraries and
# libclang -- `tools/setup/fetch-ffmpeg.sh`, or `task ffmpeg`, once per machine. That is a statement about
# the *commands*: the cargo `video` feature is still off by default, so a bare
# `cargo build --workspace` needs neither.
#
# `km-video/ffmpeg` is named although the workspace dependency entry already turns it on for every
# consumer: with `--workspace`, `km-video` is also built as a member in its own right, and this is
# what gives that build a real ffmpeg rather than the empty library a bare workspace build compiles.
# `km-stream/ffmpeg` is the same spelling for the same reason, one crate over: that one encodes where
# `km-video` decodes, and a member built without it is a crate with its encoder configured out.
KM_FEATURES_VIDEO="$KM_FEATURES,km-video/ffmpeg,km-stream/ffmpeg,karaokemachine/video,km-pack/video,km-package-builder/video"

# ...and what an **Android** build turns on, which is deliberately much shorter than either of the
# two above and is a third entry rather than a reuse of one.
#
# Three differences, each a reason it cannot be `KM_FEATURES_VIDEO`:
#
#   * It builds `-p karaokemachine --lib`, not `--workspace`. km-pack and km-package-builder are
#     packaging tools that no APK contains, so naming their features would ask cargo to build two
#     command-line programs for an ABI that cannot run one.
#   * `km-video/ffmpeg` is not named and must not be. That spelling exists for `--workspace` builds,
#     where km-video is compiled as a member in its own right; here it arrives through the root
#     Cargo.toml's workspace dependency entry the moment `karaokemachine/video` pulls km-video in.
#   * Neither testing feature belongs in a shipped APK.
#
# So it is one word, and the comment is longer than the value because the value looks like an
# oversight until you know why. See "Video on Android" in docs/ARCHITECTURE.md.
#
# **No `.cargo/config.toml` alias mirrors this one**, which is why `tools/platform/linux/check.sh` gains no
# fourth assertion: a cargo alias cannot express `cargo ndk`, so there is no second spelling of this
# list anywhere for it to drift against.
KM_FEATURES_ANDROID="video"

# ...and what the machine's *iOS* shell turns on, which is one word for the same three reasons the
# Android line above is one word: it builds `-p km-machine-ios --lib` rather than `--workspace`, so
# the packaging tools' features would ask for two command-line programs a bundle cannot hold;
# `km-video/ffmpeg` arrives through the root `Cargo.toml`'s workspace entry the moment
# `karaokemachine/video` pulls km-video in; and neither testing feature belongs in a shipped
# application.
#
# `km-machine-ios`'s own `video` feature passes straight through to `karaokemachine/video`. Turning
# it on is what makes `ffmpeg-sys-next` link, so it goes together with the four xcframeworks
# `tools/port/machine/ios/frameworks.sh` wraps: without them the app links against nothing and the
# failure is an undefined-symbol wall at the point Xcode links. `build.sh --no-video` is what turns
# both off at once.
#
# **No `.cargo/config.toml` alias mirrors this one either**, for the reason given below: the iOS
# build needs an `IPHONEOS_DEPLOYMENT_TARGET`, a `FFMPEG_DIR` per slice and a `--target` per slice
# that an alias cannot carry.
KM_FEATURES_IOS="video"

# ...and what the two *remote* shells turn on, which is nothing at all, on each.
#
# **Empty is a statement, which is why both are written down.** Each builds
# as `-p km-remote-<platform> --lib` and takes no feature of its own, and the one thing that could
# plausibly belong here -- how that shell looks for a machine -- is named in its own manifest
# instead: `km-remote-android` takes `km-remote-core` with `features = ["mdns"]` and
# `km-remote-ios` with `features = ["sweep"]`, because the workspace entry turns that crate's
# default off so every host has to say. Repeating either here would be a second spelling of a
# decision that already has one, which is exactly the drift this file exists to prevent.
#
# They earn entries so that the day a shell does need a feature there is one place it goes, and so
# that a reader comparing them with the line above sees the difference is deliberate.
#
# No `.cargo/config.toml` alias mirrors either, for the reason given above: a cargo alias cannot
# express `cargo ndk`, and the iOS build needs an `IPHONEOS_DEPLOYMENT_TARGET` and a `--target` per
# slice that an alias cannot carry.
KM_FEATURES_ANDROID_REMOTE=""
KM_FEATURES_IOS_REMOTE=""
