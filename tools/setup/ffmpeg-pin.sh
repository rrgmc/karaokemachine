# shellcheck shell=bash
#
# The pinned LGPL ffmpeg: which release, which bytes, and how it is configured.
#
# Sourced, never executed -- no shebang and no `set -euo pipefail`, for the reason
# tools/dist/common.sh gives: a sourced file that sets shell options changes the caller's shell in
# ways the caller did not ask for.
#
#   . tools/setup/ffmpeg-pin.sh          # from the repository root, which both callers cd to first
#
# Its two callers build the same ffmpeg in different places:
#
#   tools/setup/fetch-ffmpeg.sh          on a developer's Mac, into the asset cache, for FFMPEG_DIR
#   tools/platform/linux/ffmpeg-lgpl.sh     inside the Debian build container, into the shared build volume
#
# **What is shared here is the definition, not the procedure**, and that line is deliberate. The two
# environments genuinely differ -- one caches under $HOME and exports a variable, the other stamps a
# prefix in a Docker volume and then reads DT_NEEDED off the result to prove nothing external crept
# in -- and folding those together would mean restructuring a macOS path that cannot be run or tested
# from the machine this is usually developed on. What must never differ is *what gets built*: the
# release, the bytes, and the flags. That is what lives here.
#
# It exists because both were written independently, days apart, and arrived at the same version, the
# same checksum and the same `--disable-autodetect` line from opposite directions -- Homebrew's GPL
# build on one side, Debian's on the other. The agreement was reassuring and the duplication was not:
# they had already drifted by one flag before anybody noticed. See `The tarball's ffmpeg is built,
# not borrowed` in docs/decisions/distribution.md and `Video in a macOS release` in
# docs/decisions/song-sources.md.

# ---------------------------------------------------------------------------------------------
# Which release.
#
# **7.1.5 rather than the newest, and that is the load-bearing part of the pin.** It is exactly what
# Debian trixie ships (`libavcodec61` 7:7.1.5-0+deb13u1), which is what the appliance runs -- so the
# development machine is the lower bound rather than ahead of it, and every carrier decodes through
# the same code and reports the same libavutil version. Homebrew's 8.1.1 was the one thing breaking
# that rule: it lets code compile against API the appliance lacks, and the failure then appears only
# in Docker or in CI.
#
# There is no `.sha256` beside the tarball on ffmpeg.org -- only a GPG `.asc` -- so the hash is
# pinned here, exactly as the Windows zip's is in tools/setup/fetch-ffmpeg.sh. It is a *change detector*
# rather than a certificate: it does not prove anybody blessed these bytes, it guarantees that what a
# later build downloads is what these builds were tested against.
FF_SRC_VER="7.1.5"
FF_SRC_NAME="ffmpeg-$FF_SRC_VER"
FF_SRC_URL="https://ffmpeg.org/releases/$FF_SRC_NAME.tar.xz"
FF_SRC_SHA256="de668509caf9e35e3cd162473441fdb29538c6d96ed080292b3cf9e6fc5d558f"

# ---------------------------------------------------------------------------------------------
# How it is configured. `--prefix` is the caller's to add; everything else is here.
#
# **LGPL, because bundling means redistributing.** This project only ever *decodes* -- every
# `ff::codec::context::Context` in km-video becomes a `Decoder`, and km-pack re-encodes by shelling
# out to whatever `ffmpeg` is on PATH rather than through these libraries. Both platforms' stock
# ffmpeg is the GPL configuration: Homebrew's is `--enable-gpl --enable-version3`, and Debian says so
# in its own copyright file -- "some of the GPL licensed files are used, so the resulting binaries
# are licensed under GPL v2+". This workspace is `MIT OR Apache-2.0`, which is not GPL-2-compatible,
# so shipping either one beside this binary would produce a combined work this project cannot
# redistribute. It would also be far larger: 25 MB of unreachable encoders on macOS, and on Debian a
# closure of 93 shared libraries and 97 MiB, because x264, x265 and libjxl drag harfbuzz, fontconfig,
# pango, cairo and glib in behind them.
#
# **`--disable-autodetect` is the load-bearing flag**, and it was earned twice. On macOS it was added
# after watching a build without it link `/opt/homebrew/opt/libx11/lib/libX11.6.dylib` into
# libavcodec -- putting an absolute Homebrew path straight back into the closure the exercise existed
# to remove, and making the result depend on what happened to be installed on the building machine,
# so two developers would get materially different libraries from the same pinned source. On Linux it
# is the only thing standing between this and the GPL codecs sitting in the build image as -dev
# packages. With autodetection off, what comes out is a function of this list alone.
#
# The rule it expresses is worth stating as a rule: **every decoder ffmpeg implements itself, and
# none that needs a third-party library.** Not a hand-picked codec list, which would have to be
# revisited each time somebody auditioned a file it did not anticipate. It costs the product nothing,
# because `The packaging profile for a video` settled that what the machine ever has to decode is
# H.264 and AAC in MP4, all of it native code; the wider native set is there so `--play ./clip.mp4`
# can still audition the VP9 file somebody is deciding whether to package.
#
# **That rule is about decoders, and there is exactly one external library on the encoding side.**
# A machine streaming its screen has to produce H.264, because that is what a television, a browser
# and every player in between can be relied on to play — and ffmpeg implements no H.264 encoder of
# its own. x264 is GPL and therefore impossible here; libopenh264 is BSD-2 and, unlike x264, needs
# no `--enable-gpl`, so the LGPL posture this file exists to protect is unchanged. AAC needs nothing
# added: ffmpeg's own encoder is native.
#
# **What it costs each platform is a package.** Debian has `libopenh264-dev`, which
# `tools/platform/linux/apt-deps.sh` installs for `--video`; macOS has Homebrew's `openh264`, which
# `tools/setup/fetch-ffmpeg.sh` checks for beside `nasm`. Windows takes a prebuilt LGPL ffmpeg that
# carries it already and reaches none of this.
#
# The four sublibraries are switched off rather than merely unused: avdevice, avfilter, swscale and
# postproc are the ones that reach for ffmpeg's optional external dependencies, and `ffmpeg-next` is
# taken in the root Cargo.toml with `default-features = false` plus codec/format/software-resampling,
# so none of them was ever linked. `--disable-network` because every file this opens is a local one,
# which is `Nothing downloads` in docs/decisions/repository.md expressed as a build flag: the only protocol left is
# `file`.
#
# **zlib is the one external library asked for explicitly**, and it is asked for on both platforms
# for the same reason: several demuxers want it -- a `.mov` whose header is deflated, a Matroska
# track with compressed frames -- and it is `/usr/lib/libz` on a Mac and `libz.so.1` on any Linux,
# present everywhere and costing the closure nothing worth counting. This flag is the one the two
# builds disagreed about while they were separate files, so a file that plays out of the macOS bundle
# now plays out of the Linux tarball too.
FF_SRC_CONFIGURE=(
  --disable-gpl --disable-nonfree --disable-version3
  --enable-shared --disable-static
  --disable-programs --disable-doc --disable-debug
  --disable-avdevice --disable-avfilter --disable-swscale --disable-postproc
  --disable-network
  --disable-autodetect --enable-zlib
  # The one external library, and an encoder rather than a decoder. See above.
  --enable-libopenh264
)

# **A slice cross-compiled for a phone or a tablet takes the same line without the encoder**, because
# the encoder is a package the platform provides and neither of those platforms has one. What it
# costs is that a machine on a tablet does not stream its screen, which is what the decision above
# already says by naming Debian, macOS and Windows and no others.
#
# **Derived rather than typed.** A second list is a second thing to keep in step, and the flag that
# reaches every build is the ordinary case: a new one added above arrives here by being there.
FF_SRC_CONFIGURE_CROSS=()
for _ff_pin_flag in "${FF_SRC_CONFIGURE[@]}"; do
  [ "$_ff_pin_flag" = "--enable-libopenh264" ] || FF_SRC_CONFIGURE_CROSS+=("$_ff_pin_flag")
done
unset _ff_pin_flag

# ---------------------------------------------------------------------------------------------
# What this build *is*, as a short string.
#
# Everything above, hashed: change the release or any flag and this changes with it. It exists so a
# build can be stored at a path that names its own identity, which is what lets two checkouts share
# one Docker volume without fighting over one directory. Same pin, same directory, shared as intended;
# different pins, different directories, and neither can delete the other's. See "the prefix is
# content-addressed" in tools/platform/linux/ffmpeg-lgpl.sh.
#
# **`--prefix` is deliberately not in it.** The stamp this replaces hashed the whole configure line
# including the prefix, which was harmless while the prefix was a constant and becomes circular the
# moment the prefix is derived from the hash. It is also the right cut on its own terms: where a build
# was installed is not part of what it is.
#
# Twelve hex digits. This names directories inside one private volume, not artifacts anybody
# publishes, so the length is about being readable in a path rather than about collision resistance.
#
# `shasum` first and `sha256sum` second, which is the fallback tools/setup/fetch-assets.sh:37-43 and
# tools/setup/fetch-ffmpeg.sh:103-108 already use: one of the two is on every macOS, BSD and Linux, and it
# is not the same one. Only the Linux container reads this, but the file is sourced on macOS too, so
# reaching straight for `sha256sum` here would break `tools/setup/fetch-ffmpeg.sh` on the platform that
# does not ship it. Empty rather than fatal if neither exists -- a sourced file must not exit its
# caller's shell, so the one script that needs the value checks it instead.
#
# **There are two, because there are two configure lines.** A prefix names the build inside it, so a
# slice built without the encoder cannot be named by the line that asks for one. `FF_SRC_ID` names a
# build that carries the encoder and `FF_SRC_ID_CROSS` one that does not; each script takes the one
# that matches the list it passes to configure.
if command -v shasum >/dev/null 2>&1; then
  FF_SRC_ID="$(printf '%s\n' "$FF_SRC_VER" "${FF_SRC_CONFIGURE[@]}" | shasum -a 256 | cut -c1-12)"
  FF_SRC_ID_CROSS="$(printf '%s\n' "$FF_SRC_VER" "${FF_SRC_CONFIGURE_CROSS[@]}" | shasum -a 256 | cut -c1-12)"
elif command -v sha256sum >/dev/null 2>&1; then
  FF_SRC_ID="$(printf '%s\n' "$FF_SRC_VER" "${FF_SRC_CONFIGURE[@]}" | sha256sum | cut -c1-12)"
  FF_SRC_ID_CROSS="$(printf '%s\n' "$FF_SRC_VER" "${FF_SRC_CONFIGURE_CROSS[@]}" | sha256sum | cut -c1-12)"
else
  FF_SRC_ID=""
  FF_SRC_ID_CROSS=""
fi
