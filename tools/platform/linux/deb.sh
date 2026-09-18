#!/usr/bin/env bash
#
# Builds a Debian package of the machine, in a container.
#
#   tools/platform/linux/deb.sh                 # build the image if needed, then the .deb
#   tools/platform/linux/deb.sh --no-video      # ...with the optional video feature turned off
#   tools/platform/linux/deb.sh --system-ffmpeg # ...linking the distribution's ffmpeg, not carrying one
#   tools/platform/linux/deb.sh --tools         # karaokemachine-tools instead: the three web tools
#   tools/platform/linux/deb.sh --rebuild       # rebuild the image from scratch first
#   tools/platform/linux/deb.sh --shell         # a shell in the build image, for poking at it
#   tools/platform/linux/deb.sh -v              # watch the image build and the package build
#
# **Quiet by default.** The phases and the package report are printed; `docker build`'s layer stream
# and cargo's compile stream are not. A step that fails replays everything it held back.
#
# Output: dist/karaokemachine/linux/karaokemachine_<version>-1_amd64.deb
#         dist/karaokemachine/linux/no-video/karaokemachine_<version>-1_amd64.deb        (--no-video)
#         dist/karaokemachine/linux/system-ffmpeg/karaokemachine_<version>-1_amd64.deb   (--system-ffmpeg)
#
# **The default package carries its own ffmpeg, and that is what lets an appliance install carry no
# X.** Debian's libavutil has libX11, libva-x11, libvdpau and libOpenCL as direct NEEDED entries, so
# a package linking it loads an X client library on a box with no X server. The build this repository
# makes links none of them (tools/platform/linux/ffmpeg-lgpl.sh asserts it), which is what lets the
# X11 libraries sit in Recommends: `apt-get install` gives a desktop its backend and
# `--no-install-recommends` -- what deploy.sh passes -- gives the appliance a box with no X on it.
#
# `--system-ffmpeg` is the other trade, and it is the one a distribution packager wants: no vendored
# libraries, and X on the box in exchange.
#
# **Video is on by default**, and on this platform it costs the build nothing at all: it needs
# nothing installed here and nothing added to the box's sources, because the build image already
# carries the four ffmpeg -dev packages and libclang (tools/platform/linux/Dockerfile runs apt-deps.sh
# --video), and the runtime libraries the package then Depends on are Debian's own. This is the one
# platform where video costs the *shipping* story nothing either -- Windows has to stage four DLLs
# beside the exe, macOS links them by absolute path, and a .deb just names them.
#
# The declined build goes in a subfolder rather than beside the ordinary one because both are called
# karaokemachine_<version>-1_amd64.deb -- cargo-deb names the file from the package and the version,
# and a cargo feature changes neither. Two builds into one directory would overwrite each other, and
# every consumer here picks the newest .deb in a directory. The marker is on the declined build for
# the same reason it is in the Windows folder name: the plain path should hold what the plain command
# produces.
#
# Why a container. The package has to link against the glibc of the distribution it is for, and this
# is developed on Windows -- so there is no host toolchain to use, and the WSL Ubuntu on the same box
# is *newer* than Debian stable, which would produce a package that installs and then refuses to
# start. tools/platform/linux/Dockerfile names the target distribution in its FROM line, and that line is the
# compatibility floor. Nothing Linux has to be installed on the developer's machine.
#
# Requirements: Docker. On Windows that means Docker Desktop running.

set -euo pipefail

cd "$(dirname "$0")/../../.."

. tools/dist/common.sh
DIST_SCRIPT=deb

. tools/platform/linux/image-tag.sh          # sets IMAGE + RUST_VERSION, hashed from Dockerfile, apt-deps.sh, rust-toolchain.toml
VOLUME="karaokemachine-deb-build"   # cargo registry + target cache, so a second run is quick
BUILD_ARGS=()
REBUILD=0
CMD=(tools/platform/linux/deb-in-container.sh)
VIDEO=1
SYSTEM_FFMPEG=0
TOOLS=0

for arg in "$@"; do
  case "$arg" in
    --rebuild) REBUILD=1; BUILD_ARGS+=(--no-cache --pull) ;;
    --shell) CMD=(bash) ;;
    --no-video) VIDEO=0 ;;
    # Link the distribution's ffmpeg rather than carrying one. What a packager rebuilding this for a
    # distribution wants, vendored libraries being what Debian policy tells them not to ship -- and
    # the build that accepts X on the box in exchange, Debian's libavutil having libX11 as a direct
    # NEEDED. The default carries its own and leaves an appliance with no X on it.
    --system-ffmpeg) SYSTEM_FFMPEG=1 ;;
    # Build `karaokemachine-tools` instead of the machine: km-package-builder, km-remote and
    # km-admin in one package, which the machine's Recommends. Defined in
    # tools/cmd/km-package-builder/Cargo.toml.
    --tools) TOOLS=1 ;;
    # The build logs, which are quiet by default -- here that is `docker build`'s layer stream and,
    # inside the container, cargo's. It travels in as an environment variable below for the same
    # reason KM_VIDEO does: deb-in-container.sh takes no arguments at all.
    -v|--verbose) DIST_VERBOSE=1 ;;
    *) echo "deb: unknown option $arg" >&2; exit 2 ;;
  esac
done

# Refused rather than ignored. A build with no video has no ffmpeg of either kind, so the two flags
# together describe nothing -- and silently dropping one of a pair somebody typed is how a run comes
# to produce something other than what was asked for.
if [ "$VIDEO" -eq 0 ] && [ "$SYSTEM_FFMPEG" -eq 1 ]; then
  echo "deb: --system-ffmpeg needs video; --no-video builds no ffmpeg of either kind." >&2
  exit 2
fi
# The tools package reaches the machine's ffmpeg through `$ORIGIN/../lib`, which exists only in the
# package that carries one. Against a system-ffmpeg machine there is no such directory and
# km-package-builder would have to Depend on Debian's libav itself -- a different package, not this
# one wearing a flag.
if [ "$TOOLS" -eq 1 ] && [ "$SYSTEM_FFMPEG" -eq 1 ]; then
  echo "deb: --tools builds against the machine's own ffmpeg; --system-ffmpeg leaves it none." >&2
  exit 2
fi

# MSYS (Git Bash) rewrites anything that looks like a Unix path before handing it to a Windows
# executable, which turns `/src` into `C:/Program Files/Git/src` and mounts the wrong thing entirely.
# Turned off wholesale, and host paths are then converted deliberately below.
export MSYS2_ARG_CONV_EXCL='*'

# `host_path` -- the bind-mount path conversion -- comes from tools/dist/common.sh, which
# tools/platform/linux/verify-deb.sh also sources for it.

if ! docker version >/dev/null 2>&1; then
  echo "deb: cannot reach the Docker daemon." >&2
  echo "     On Windows, start Docker Desktop and wait for it to say it is running." >&2
  exit 1
fi

# Built only when it is missing, the way check.sh:63 does it. The tag names its own content
# (tools/platform/linux/image-tag.sh), so "the image exists" and "the image is the one these inputs
# describe" are the same question, and building an already-built tag is a guaranteed layer-cache
# hit: correct, and pure overhead. On this box that
# overhead is not small, because a `docker build` is a container start and those cost seconds here
# rather than milliseconds. See "Docker speed on this box" in CLAUDE.local.md.
#
# `--rebuild` still builds unconditionally, which is the whole point of asking for it. The flag is
# tracked in REBUILD rather than inferred from BUILD_ARGS being non-empty: reading intent out of an
# array's length works until the day a second option appends a build argument, and then it fails
# silently in the direction of not rebuilding.
#
# The build itself is quiet, and on `--rebuild` that is the biggest single suppression in the
# repository: `--no-cache --pull` re-runs the Dockerfile's whole apt install, a rustup download and a
# `cargo install cargo-deb`. It is also the safest one to make quiet, because none of it is a report
# -- and `dist_run` replays every line of it if it fails.
if [ "$REBUILD" = "1" ] || ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  dist_step "image (building $IMAGE)"
  image_started=$SECONDS
  dist_run "docker build" docker build "${IMAGE_BUILD_ARGS[@]}" "${BUILD_ARGS[@]+"${BUILD_ARGS[@]}"}" -t "$IMAGE" tools/platform/linux
  printf '   built in %s\n' "$(dist_elapsed "$image_started")"
else
  dist_step "image $IMAGE (cached)"
fi
echo

# App, then platform -- the layout rule is in tools/dist/common.sh. `deb-in-container.sh` writes to `/out`
# and knows nothing about this path, so a layout change is a change to the mount below and nothing
# else. The declined build gets a subfolder of it, for the reason in the header.
OUT="$(dist_dir karaokemachine linux)"
# A different package goes to a folder of its own rather than beside the machine's: the two have
# different names, so nothing forces the separation the way the variants need it, and a reader
# looking for one artifact should not have to read filenames to find it.
[ "$TOOLS" -eq 1 ] && OUT="$(dist_dir karaokemachine-tools linux)"
[ "$VIDEO" -eq 0 ] && OUT="$OUT/no-video"
# The same answer the declined build gets, for the same reason: all three builds carry one package
# name, one version and one file name, so the folder is the only thing that tells them apart on disk.
[ "$SYSTEM_FFMPEG" -eq 1 ] && OUT="$OUT/system-ffmpeg"
mkdir -p "$OUT"

dist_step package
# The source tree is mounted read-write because cargo wants to write Cargo.lock timestamps and the
# asset check may fetch the SoundFont, but the build itself writes to /build (the named volume) via
# CARGO_TARGET_DIR -- so a Linux build cannot disturb the Windows target/ directory in the same
# checkout.
#
# The feature travels as an environment variable rather than an argument, so `--shell` puts you in a
# container that would build the same thing the automatic run just did.
# KM_CHECKOUT tells clean-checkout.sh which host tree this `/src` actually is, which is the one thing
# the shared build volume cannot work out for itself. Passed as an environment variable rather than
# an argument for the same reason KM_VIDEO is: `$CMD` is `--shell` sometimes, and a flag would have
# to survive that.
#
# **Deliberately not wrapped in `dist_run`.** This is the one long command here whose output is a
# report rather than a log: everything the container prints -- the `dpkg-deb --field` dump, the
# Depends assertions, the byte total, the install line -- is the answer this script exists to give.
# So the quieting happens *inside*, where deb-in-container.sh sources the same library and can tell its
# own report from cargo's compile stream.
docker run --rm -i \
  -e "KM_VIDEO=$VIDEO" \
  -e "KM_SYSTEM_FFMPEG=$SYSTEM_FFMPEG" \
  -e "KM_TOOLS=$TOOLS" \
  -e "DIST_VERBOSE=$DIST_VERBOSE" \
  -e "KM_CHECKOUT=$(host_path "$PWD")" \
  -v "$(host_path "$PWD")":/src \
  -v "$VOLUME":/build \
  -v "$(host_path "$PWD/$OUT")":/out \
  "$IMAGE" "${CMD[@]}"
