#!/usr/bin/env bash
#
# Cross-compiles the pinned LGPL ffmpeg for Android, one prefix per ABI.
#
#   tools/port/machine/android/ffmpeg.sh                          # both ABIs, into the asset cache
#   tools/port/machine/android/ffmpeg.sh armeabi-v7a              # just one
#   tools/port/machine/android/ffmpeg.sh --print-dir arm64-v8a    # print that ABI's prefix and build nothing
#   tools/port/machine/android/ffmpeg.sh --force                  # rebuild even if a complete prefix is there
#
# Run once per machine per ABI; `tools/port/machine/android/build.sh` calls it and it is a no-op after the first
# time. About two minutes per ABI.
#
# **Which release, which bytes and most of which flags are not decided here.** They are in
# tools/setup/ffmpeg-pin.sh, shared with tools/setup/fetch-ffmpeg.sh (macOS) and tools/platform/linux/ffmpeg-lgpl.sh (the
# Debian container). This file is the third caller of that pin and follows the same rule its header
# states: **what is shared is the definition, not the procedure.** What is decided here is everything
# about *this* environment -- the NDK's clang, the per-ABI architecture flags, the content-addressed
# prefix in the asset cache, and the checks at the end that read the result back.
#
# **Android departs from the pin in two ways, and both are named in FF_ANDROID_EXTRA below.**
# MediaCodec is compiled in but never asked for, so the hardware decoder is a Rust change away rather
# than a second ffmpeg build on every machine. And the decoder set is **narrowed to the packaging
# profile**, which is the larger departure: it abandons the pin's "every decoder ffmpeg implements
# itself" rule on this platform alone, because with the full set libavcodec's link line overruns
# Windows' 32,767-character command-line limit and cannot be built here at all. The full reasoning,
# the exact failure it produces and what it costs are with the flags. Measured result: 2.2 MB per
# ABI rather than 10-13 MB.
#
# Because the configure line therefore differs from the pin's, the extras are folded into the prefix
# hash -- so changing either departure cannot quietly reuse a directory built the other way.
#
# **Shared libraries, not static, and the reason is the license rather than convenience.** This
# project only ever decodes, and the whole ffmpeg posture -- tools/setup/ffmpeg-pin.sh's header,
# tools/platform/linux/ffmpeg-lgpl.sh's, the `Video in a macOS release` decision -- rests on the sentence
# *shipping the shared libraries beside the binary is compliant under LGPL and would not be under
# GPL*. Static linking into libkm_app.so would move the APK from that case to LGPL-2.1 section 6's
# relinking obligations, and would make Android the one carrier here with a different license story.
# It is also already the Android pattern in this repository: SDL is linked shared on Android alone
# (see the target split in crates/machine/karaokemachine/Cargo.toml and crates/playback/km-display/Cargo.toml), and
# tools/port/machine/android/stage.sh already copies libSDL3.so and libSDL3_ttf.so into jniLibs.
#
# **Nothing has to patch configure for the soname, and that was checked rather than assumed.**
# ffmpeg's own `android)` case already does it -- verified against ffmpeg 7.1.5:
#
#     android)
#         disable symver
#         SLIB_INSTALL_NAME='$(SLIBNAME)'
#         SLIB_INSTALL_LINKS=
#         SHFLAGS='-shared -Wl,-soname,$(SLIBNAME)'
#
# so the installed files are `libavcodec.so` with SONAME `libavcodec.so`, and there are no
# `libavcodec.so.61` files and no version symlinks. That matters more than it looks: the Android
# Gradle plugin packages only files ending in `.so` from jniLibs, so a versioned name is dropped at
# packaging time and the app dies at launch with UnsatisfiedLinkError and nothing wrong in the build
# log. The check at the bottom asserts it, so a later pin bump cannot reintroduce it silently.

set -euo pipefail

cd "$(dirname "$0")/../../../.."

. tools/setup/ffmpeg-pin.sh
. tools/port/ndk.sh

ABIS=()
PRINT_DIR=0
FORCE=0
for arg in "$@"; do
  case "$arg" in
    --print-dir) PRINT_DIR=1 ;;
    --force) FORCE=1 ;;
    -*) echo "ffmpeg(android): unknown option $arg" >&2; exit 2 ;;
    *) ABIS+=("$arg") ;;
  esac
done
[ ${#ABIS[@]} -gt 0 ] || ABIS=("arm64-v8a" "armeabi-v7a")

# Progress goes nowhere under --print-dir, whose stdout is a value another script captures. Errors
# always go to stderr, so a caller reading the value still sees them. The same split
# tools/setup/fetch-ffmpeg.sh makes, for the same reason.
say() { [ "$PRINT_DIR" -eq 1 ] || printf '%s\n' "$*"; }
die() { printf 'ffmpeg(android): %s\n' "$*" >&2; exit 1; }

# Scratch trees to remove however this script ends. `KM_KEEP_WORK=1` keeps them, with
# configure.log, make.log and ffbuild/config.log inside -- every interesting failure in a
# cross-build is one of those three files, and they are the first thing a tidy run deletes.
CLEANUP_DIRS=()
cleanup() {
  [ -n "${KM_KEEP_WORK:-}" ] && return 0
  for d in ${CLEANUP_DIRS+"${CLEANUP_DIRS[@]}"}; do rm -rf "$d"; done
}
trap cleanup EXIT

if command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
elif command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
else
  die "need shasum or sha256sum to verify the download"
fi

# -- what Android adds to the pin ------------------------------------------------------------------

# **MediaCodec is compiled in and deliberately not used.**
#
# `h264_mediacodec` is Android's hardware H.264 decoder reached through ffmpeg's own API, so having
# it available costs a DT_NEEDED on libmediandk/libandroid and a few hundred KB, and costs nothing at
# run time until something asks for it by name -- which km-video does not. It is here so that *if*
# software decode turns out not to keep up on a 32-bit Cortex-A55 television, switching is a change
# in Rust rather than a second ffmpeg build on every developer's machine.
#
# `--enable-jni` is what `--enable-mediacodec` needs, and it brings an obligation with it that the
# Rust side would have to meet before the decoder could ever be opened: `av_jni_set_java_vm()` must
# be called first. km-app already captures the JavaVM in crates/machine/karaokemachine/src/androidctx.rs.
FF_ANDROID_EXTRA=(
  --enable-jni
  --enable-mediacodec
  --enable-decoder=h264_mediacodec

  # ------------------------------------------------------------------------------------------
  # **The pin's decoder rule does not hold here, and this is the one place that is true.**
  #
  # tools/setup/ffmpeg-pin.sh builds *every decoder ffmpeg implements itself*, deliberately, so that
  # `--play ./clip.mp4` can audition the VP9 file somebody is deciding whether to package. That
  # rule is kept on all three desktop platforms and abandoned on Android, for a reason that
  # began as a size preference and turned out to be a hard build failure.
  #
  # **It cannot be built the other way on Windows.** With the full set, libavcodec's link line
  # is over 25 KB of object paths alone, and Windows' CreateProcess caps a command line at
  # 32,767 characters. The line is truncated mid-argument and clang reports
  #
  #     no such file or directory: 'libavcodec/bsf/mpeg4_unpack_bframe'
  #
  # -- which is `mpeg4_unpack_bframes.o` with its last three characters cut off. It reads like a
  # missing source file and is nothing of the kind; the object is there. Linux has no such limit,
  # so the full set could be built in a container, at the cost of making Docker and a second
  # ~1 GB NDK a prerequisite of an Android build that today needs neither.
  #
  # **What it costs is close to nothing, because of a decision already taken.** `The packaging
  # profile for a video` in docs/decisions/song-sources.md fixes what the machine can ever meet in a
  # package:
  # H.264 in 8-bit 4:2:0, at most 1080p30, AAC, in MP4. Anything outside that is re-encoded at
  # packaging time, on a desktop, by an ffmpeg that is not this one. So no song a package can
  # hold becomes unplayable. What is lost is auditioning an *arbitrary* loose file on the device
  # through the `debug.play_file` route -- which refuses with a clean "unsupported" rather than
  # misbehaving, and which is a developer's path on a platform with no command line.
  #
  # mp3 is included although the profile says AAC: an MP4 carrying MP3 audio is common enough in
  # the wild to be worth a few KB, and this is the one platform where a wrong guess is expensive
  # to correct. pcm_s16le costs nothing and covers the trivial case.
  #
  # Measured, armeabi-v7a, stripped: 2.2 MB for all four libraries, against an estimated 10-13 MB
  # for the full set. See the `What the Android build's ffmpeg can decode` decision in
  # docs/decisions/song-sources.md.
  --disable-everything
  --enable-decoder=h264,aac,aac_latm,mp3,mp3float,pcm_s16le
  --enable-parser=h264,aac,aac_latm,mpegaudio
  --enable-demuxer=mov,mp3,aac
  --enable-protocol=file
  # `h264_mp4toannexb` is not optional: MediaCodec is fed Annex-B, and an MP4's H.264 is not.
  # `extract_extradata` is what the parser needs to find SPS/PPS. Both are required by the
  # hardware path in §6 of the plan even though nothing selects it yet.
  --enable-bsf=h264_mp4toannexb,aac_adtstoasc,extract_extradata
)

# The build's identity, extended by this platform's own additions.
#
# `$FF_SRC_ID_CROSS` from the pin covers the release and the configure line this takes. It cannot see
# FF_ANDROID_EXTRA, and a prefix that ignored those would be a directory whose name lies about what
# is in it -- so hash the two together. Twelve hex digits, the same length and for the same reason
# the pin gives: this names directories in a private cache, not artifacts anybody publishes.
#
# **The cross line, because the NDK has no openh264 package to link.** `--enable-libopenh264` is asked
# for by name on a line with autodetection off, so configure refuses outright rather than building
# without it, and the id has to name the line that was actually used.
[ -n "$FF_SRC_ID_CROSS" ] || die "tools/setup/ffmpeg-pin.sh could not compute FF_SRC_ID_CROSS (no shasum/sha256sum?)"
FF_ANDROID_ID="$(printf '%s\n' "$FF_SRC_ID_CROSS" "${FF_ANDROID_EXTRA[@]}" | {
  if command -v shasum >/dev/null 2>&1; then shasum -a 256; else sha256sum; fi
} | cut -c1-12)"

# The same cache convention tools/setup/fetch-ffmpeg.sh and tools/setup/fetch-assets.sh use, and for the same
# reason: outside the repository, so `cargo clean` and a fresh clone both leave it alone and sibling
# worktrees share one copy. Per-ABI directories under one id, so both ABIs of one pin live together
# and two different pins cannot collide. The convention itself is tools/setup/asset-cache.sh.
. tools/setup/asset-cache.sh

# The unpacked source is scratch and the tarball is what is worth keeping, so they live apart. The
# tarball is the same bytes for every ABI and every configuration of a release, so it is shared
# rather than duplicated per id -- tools/platform/linux/ffmpeg-lgpl.sh makes the same split for the same
# reason, and this is where the 11 MB download does not happen four times.
SRC_CACHE="$CACHE/ffmpeg-src"

prefix_for() { printf '%s/ffmpeg-android/%s/%s\n' "$CACHE" "$FF_ANDROID_ID" "$1"; }

# --print-dir answers and builds nothing. `build.sh` uses it to set FFMPEG_DIR per ABI, so it must
# stay a bare value on stdout with no decoration.
if [ "$PRINT_DIR" -eq 1 ]; then
  [ ${#ABIS[@]} -eq 1 ] || die "--print-dir takes exactly one ABI"
  prefix_for "${ABIS[0]}"
  exit 0
fi

ndk_require || exit 1

# -- the source ------------------------------------------------------------------------------------

fetch_source() {
  mkdir -p "$SRC_CACHE"
  tarball="$SRC_CACHE/ffmpeg-$FF_SRC_VER.tar.xz"

  if [ -f "$tarball" ] && [ "$(sha256 "$tarball")" = "$FF_SRC_SHA256" ]; then
    say "-- source: cached"
    return 0
  fi

  say "-- source: fetching $FF_SRC_URL"
  # Downloaded to a name of this process's own and moved into place only once the checksum passes,
  # so two concurrent runs never write one file and a peer sees either no tarball or a whole
  # verified one. Lifted from tools/platform/linux/ffmpeg-lgpl.sh, which learned it the hard way.
  part="$tarball.part.$$"
  curl --proto '=https' --tlsv1.2 -sSfL -o "$part" "$FF_SRC_URL"
  got="$(sha256 "$part")"
  if [ "$got" != "$FF_SRC_SHA256" ]; then
    rm -f "$part"
    die "checksum mismatch for ffmpeg-$FF_SRC_VER.tar.xz
             expected $FF_SRC_SHA256
             got      $got"
  fi
  mv "$part" "$tarball"
}

# -- one ABI ---------------------------------------------------------------------------------------

build_abi() {
  abi="$1"
  prefix="$(prefix_for "$abi")"
  done_marker="$prefix/.km-complete"

  # A marker rather than a stamp: the path already says *what* this is, so the only question left is
  # whether the install finished. Written last, so it can only exist on a complete tree.
  if [ -f "$done_marker" ] && [ "$FORCE" -eq 0 ]; then
    say "-- $abi: cached at $prefix"
    return 0
  fi

  clang_target="$(ndk_clang_target "$abi")" || die "unknown ABI $abi"
  cc="$NDK_BIN/clang"
  [ -x "$cc" ] || [ -x "$cc.exe" ] || die "no clang at $cc"

  # Per-ABI architecture flags. Everything else comes from the pin.
  #
  # **No --disable-asm.** ffmpeg's ARM assembly is `.S`, assembled by clang itself with no nasm or
  # yasm anywhere, and it is exactly where 32-bit decode speed lives -- which is the whole question
  # on a television. (ffmpeg-sys-next's own Android branch disables asm for x86 only, and for a
  # different reason: position-dependent code.)
  case "$abi" in
    arm64-v8a)
      arch_flags=(--arch=aarch64 --cpu=armv8-a)
      # **16 KB page alignment, and it is not automatic here.** NDK r28 aligns to 16 KB by default
      # for ndk-build and for its CMake toolchain -- neither of which is in play in a bare
      # ./configure build driven by the clang driver. cargo-ndk adds the flag only for NDK <= 27,
      # on the assumption that r28's own build systems already did it. So nothing would set it for
      # us. Android 15+ devices with 16 KB pages refuse to load a library without it, and
      # `targetSdk 36` is where that starts to matter.
      extra_ldflags="-Wl,-z,max-page-size=16384"
      ;;
    armeabi-v7a)
      # `--enable-thumb` and NEON: the NDK's clang driver for armv7a already implies softfp and
      # NEON at API 26, so these are belt and braces rather than load-bearing.
      arch_flags=(--arch=arm --cpu=armv7-a --enable-neon --enable-thumb)
      # 32-bit only, so 16 KB pages cannot arise: every device with them is 64-bit.
      extra_ldflags=""
      ;;
    *) die "unsupported ABI $abi -- this builds arm64-v8a and armeabi-v7a" ;;
  esac

  say "== ffmpeg $FF_SRC_VER (LGPL, native decoders only) for $abi"
  say "   clang --target=$clang_target$ANDROID_PLATFORM"

  work="$SRC_CACHE/build.$abi.$$"
  rm -rf "$work"
  mkdir -p "$work"
  # Registered for the EXIT trap rather than cleaned by a RETURN one. **A RETURN trap does not fire
  # when the script dies**, which is precisely the case that matters: a failing configure or make
  # exits the subshell non-zero, `set -e` ends the script, and the half-built tree is left behind.
  # Three of them accumulated here before this was noticed, each ~800 MB of unpacked source and
  # objects. An EXIT trap fires on the error path and the success path alike.
  CLEANUP_DIRS+=("$work")
  [ -n "${KM_KEEP_WORK:-}" ] && say "-- keeping $work"
  tar -xJf "$SRC_CACHE/ffmpeg-$FF_SRC_VER.tar.xz" -C "$work"
  src="$work/ffmpeg-$FF_SRC_VER"
  staged="$work/install"

  configure=(
    "--prefix=$prefix"
    "${FF_SRC_CONFIGURE_CROSS[@]}"
    "${FF_ANDROID_EXTRA[@]}"
    --enable-cross-compile
    --target-os=android
    "--sysroot=$NDK_SYSROOT"
    "${arch_flags[@]}"
    "--cc=$cc --target=$clang_target$ANDROID_PLATFORM"
    "--ar=$NDK_BIN/llvm-ar"
    "--nm=$NDK_BIN/llvm-nm"
    "--ranlib=$NDK_BIN/llvm-ranlib"
    "--strip=$NDK_BIN/llvm-strip"
    --enable-pic
    "--extra-cflags=-fPIC -O3 -fno-strict-aliasing"
  )
  [ -n "$extra_ldflags" ] && configure+=("--extra-ldflags=$extra_ldflags")

  (
    cd "$src"
    # **A POSIX TMPDIR, because configure does not merely write there -- it executes there.**
    # ffmpeg's sanity test compiles and runs a probe under $TMPDIR, and on Windows that variable
    # holds `C:\Users\...`, whose backslashes configure's own shell eats: the failure reads
    # `chmod: cannot access 'C:UsersyouAppDataLocalTemp/ffconf.../test'`, which names the right
    # directory with every separator missing and looks like a permissions problem rather than a
    # quoting one. Pointing it inside the scratch tree fixes it and keeps the probes with the rest
    # of the build, so the trap that removes $work removes these too.
    TMPDIR="$src/ffbuild-tmp"
    export TMPDIR
    mkdir -p "$TMPDIR"
    # **MSYS2_ARG_CONV_EXCL must NOT be set here, and that is the opposite of what this repository
    # does everywhere else.** tools/platform/linux/check.sh sets it for `docker run` because the paths there
    # are container paths that Windows must keep its hands off. Here the reverse is true and it cost
    # a build to learn: the NDK's clang is a **native Windows** executable, so MSYS's argument
    # conversion is exactly what turns `--sysroot=/c/Users/...` into the `C:/Users/...` that clang
    # can actually open. With the conversion disabled, configure fails at its first probe with
    # `C compiler test failed` -- while the same clang, run by hand from this same shell, compiles
    # perfectly, because by hand nobody disabled the conversion. Leave it unset.
    #
    # configure itself is a shell script run by this bash, so its own arguments are never converted;
    # only the native tools it goes on to invoke see rewritten paths, which is precisely the split
    # wanted.
    ./configure "${configure[@]}" >configure.log 2>&1 || {
      echo "ffmpeg(android): configure failed for $abi; last 40 lines of its log:" >&2
      tail -40 configure.log >&2
      exit 1
    }
    # Held back, stderr included: an ffmpeg compile's stream of warnings would otherwise be the
    # loudest thing an Android build prints. Replayed on failure, which is the bargain every
    # staging script in this repository makes.
    "$NDK_MAKE" -j"$(nproc 2>/dev/null || echo 4)" >make.log 2>&1 || {
      echo "ffmpeg(android): make failed for $abi; last 40 lines of its log:" >&2
      tail -40 make.log >&2
      exit 1
    }
    "$NDK_MAKE" install DESTDIR="$staged" >>make.log 2>&1 || {
      echo "ffmpeg(android): make install failed for $abi; last 40 lines of its log:" >&2
      tail -40 make.log >&2
      exit 1
    }
  )

  # Installed through DESTDIR and moved into place: --prefix stays the final path so the .pc files
  # and every baked-in path are right, and the move makes the result appear all at once, so a peer
  # never sees a half-populated prefix.
  mkdir -p "$(dirname "$prefix")"
  if ! mv -T "$staged$prefix" "$prefix" 2>/dev/null; then
    if [ -f "$done_marker" ]; then
      say "-- $abi: another build finished $FF_ANDROID_ID first; keeping it"
      return 0
    fi
    die "could not move the $abi build into $prefix"
  fi

  prune_hwcontext_headers "$prefix"
  verify_abi "$abi" "$prefix"

  cp "$src/COPYING.LGPLv2.1" "$prefix/" 2>/dev/null || true
  printf '%s\n' "$FF_SRC_VER" >"$prefix/VERSION"
  : >"$done_marker"
  say "-- $abi: built into $prefix"
}

# -- make the install describe the library it actually is --------------------------------------------

# **ffmpeg installs every `hwcontext_*.h` whatever was configured, and here that is actively
# harmful.** This build has no Vulkan, no VAAPI, no QSV, no CUDA and no D3D -- `--disable-autodetect`
# saw to that, and the verify step below proves it by reading DT_NEEDED. But the headers describing
# all of them are installed anyway, so the installed tree advertises capabilities the four libraries
# beside it cannot provide.
#
# That is ordinarily harmless and here it stops the build, on 32-bit only. `ffmpeg-sys-next`'s
# `hwcontext_wrapper.h` gates each hardware API on `__has_include(<libavutil/hwcontext_*.h>)`, so an
# installed header is taken as "this build has it"; for Vulkan it then parses the crate's own stub
# for `<vulkan/vulkan.h>`, which pins the struct size with
#
#     _Static_assert(sizeof(VkPhysicalDeviceFeatures2) == 240, ...)
#
# That is the **64-bit** layout: 4-byte enum, 8-byte pointer, 220 bytes of flags, padded to 240. On
# `armeabi-v7a` the pointer is 4 bytes and the struct is 228, so the assertion fails and bindgen
# stops -- with an error about Vulkan, in a build that asked for no Vulkan, for a television that has
# none. arm64 never sees it, which is exactly why this surfaced only on the ABI that matters most.
#
# Removing the headers for hardware this build does not have is therefore not a workaround dressed
# up: it makes `__has_include` tell the truth, and the bindings then describe the library that was
# actually built. MediaCodec is kept because FF_ANDROID_EXTRA really does enable it, and `hwcontext.h`
# because `AVHWDeviceType` lives there and is part of the ordinary API.
prune_hwcontext_headers() {
  prefix="$1"
  kept=0
  removed=0
  for header in "$prefix"/include/libavutil/hwcontext_*.h; do
    [ -e "$header" ] || continue
    case "$(basename "$header")" in
      hwcontext_mediacodec.h) kept=$((kept + 1)) ;;
      *) rm -f "$header"; removed=$((removed + 1)) ;;
    esac
  done
  say "-- pruned $removed hwcontext header(s) this build cannot back, kept $kept"
}

# -- what the result has to be ---------------------------------------------------------------------

# Assert the things this file exists for, rather than trusting the flags to have meant what they say.
# tools/platform/linux/ffmpeg-lgpl.sh does the same at the end of its build and for the same reason: a
# configure that silently found a system library, or an install that quietly emitted versioned
# names, would otherwise be discovered on a device as a library that will not load.
#
# `llvm-readelf` rather than `patchelf` or `ldd`: it ships with the NDK, so it is present wherever
# this script can run at all, and it reads a cross-built ELF without needing to load it.
verify_abi() {
  abi="$1"
  prefix="$2"
  readelf="$NDK_BIN/llvm-readelf"

  # 1. No versioned sonames. This is the one that would fail at launch rather than at build time:
  #    AGP packages only `*.so` out of jniLibs, so `libavcodec.so.61` is dropped silently.
  if ls "$prefix"/lib/*.so.* >/dev/null 2>&1; then
    die "$abi: versioned libraries in $prefix/lib -- Android cannot carry these.
             ffmpeg's own android) case in configure is supposed to prevent this; check it."
  fi

  # 2. Nothing crept in that --disable-autodetect was supposed to keep out. The allowlist is the
  #    Android equivalent of the Linux script's: bionic's own, the NDK's public libraries, and our
  #    own siblings. libmediandk/libandroid are here because FF_ANDROID_EXTRA asked for MediaCodec.
  for lib in "$prefix"/lib/lib*.so; do
    [ -e "$lib" ] || continue
    while read -r dep; do
      case "$dep" in
        libc.so|libm.so|libdl.so|libz.so|liblog.so|libandroid.so|libmediandk.so) ;;
        libav*|libsw*) ;;
        "") ;;
        *) die "$abi: $(basename "$lib") depends on $dep, which is outside the allowlist.
             --disable-autodetect found something it should not have." ;;
      esac
    done < <("$readelf" -d "$lib" 2>/dev/null |
             sed -n 's/.*(NEEDED).*Shared library: \[\(.*\)\]/\1/p')
  done

  # 3. The right machine. A configure that silently built for the host would otherwise produce a
  #    prefix that links and then cannot load.
  want_machine="AArch64"
  [ "$abi" = "armeabi-v7a" ] && want_machine="ARM"
  got_machine="$("$readelf" -h "$prefix/lib/libavcodec.so" 2>/dev/null |
                 sed -n 's/^ *Machine: *//p')"
  case "$got_machine" in
    *"$want_machine"*) ;;
    *) die "$abi: libavcodec.so reports machine '$got_machine', expected $want_machine" ;;
  esac

  # 4. 16 KB page alignment, arm64 only -- see the flag above for why nothing sets this for us.
  if [ "$abi" = "arm64-v8a" ]; then
    if ! "$readelf" -l "$prefix/lib/libavcodec.so" 2>/dev/null |
         awk '/^ *LOAD/ { if ($NF == "0x4000") found = 1 } END { exit !found }'; then
      die "$abi: libavcodec.so LOAD segments are not 16 KB aligned.
             Android 15+ devices with 16 KB pages will refuse to load it."
    fi
  fi
}

# -- the one thing the Rust side needs that is not a library -----------------------------------------

# **Teaches bindgen where clang's own headers are, on Windows only, working around a cargo-ndk bug.**
#
# `ffmpeg-sys-next` runs bindgen and ships no pre-generated bindings, so libclang has to parse the
# NDK's headers. It gets the sysroot from cargo-ndk and fails anyway, on this:
#
#     sysroot/usr/include/stdint.h:35:10: fatal error: 'stddef.h' file not found
#
# `stddef.h` is not in any sysroot -- it is a *compiler* builtin, shipped in clang's resource
# directory. bindgen normally locates that by asking the clang executable named by `CLANG_PATH`, and
# cargo-ndk 4.1.2 sets that variable to `<ndk>/.../bin/clang`, **without the `.exe`** (see
# `ndk_tool` in its cargo.rs). clang-sys then says
#
#     `CLANG_PATH` env var set but is not a full path to an executable
#
# and, having no clang, adds no builtin include path. The warning and the fatal error are the same
# fault reported twice, which is why the error names a header nobody wrote.
#
# **The include path is added rather than `CLANG_PATH` corrected, and the difference is the whole
# reason this is safe.** Pointing `CLANG_PATH` at the NDK's clang.exe does fix the Android build --
# and breaks every *host* build on the machine, because cargo's `[env]` is global: a Windows
# `cargo km-build` then resolves the NDK's clang-19 resource directory and dies inside `xmmintrin.h`.
# That was tried, and it is why this now writes `BINDGEN_EXTRA_CLANG_ARGS_<triple>` instead. Those
# variables are **target-suffixed**, so a host build never reads one, and the blast radius is exactly
# the two Android triples.
#
# It has to be cargo's `[env]` with `force = true` rather than an exported variable: cargo-ndk sets
# the same variable on the cargo process it spawns, so a plain export is overwritten. `force` is what
# beats a real environment variable, and it is why these entries are marked and the two
# `tools/setup/fetch-ffmpeg.sh` writes are not -- those exist to be overridable, these exist to win.
#
# Because forcing replaces cargo-ndk's value rather than adding to it, the sysroot and the
# architecture include it supplied have to be repeated here; they are the two it sets.
#
# Unix needs none of this: there `clang` with no extension is a real executable, cargo-ndk's value is
# valid, and bindgen resolves the resource directory unaided.
write_cargo_config() {
  case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) ;;
    *) return 0 ;;
  esac

  # The NDK's own builtin headers, matching the sysroot beside them. Found rather than named, so an
  # NDK upgrade that moves clang 19 to 20 needs no edit here.
  res="$(ls -1d "$NDK_BIN/../lib/clang"/* 2>/dev/null | sort -V | tail -1)"
  [ -n "$res" ] || { say "-- no clang resource dir in the NDK; leaving cargo config alone"; return 0; }

  win() { if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
  w_sysroot="$(win "$NDK_SYSROOT")"
  w_res="$(win "$res")"

  config="${CARGO_HOME:-$HOME/.cargo}/config.toml"
  begin="# >>> karaokemachine android (written by tools/port/machine/android/ffmpeg.sh) >>>"
  end="# <<< karaokemachine android <<<"
  # **What the strip has to match is every marker this script has EVER written, and matching them
  # exactly is what kept going wrong.** The begin marker carries this script's own path, so each time
  # the script moves an existing block stops matching, is not removed, and a second one is inserted
  # beside it -- the same two keys defined twice inside one `[env]` table, which is a config cargo
  # refuses to read at all. Not an Android build broken: every build on the machine.
  #
  # It has moved twice now (`tools/android/`, then `tools/app-android/`, now here), and the second
  # time was caught by reading rather than by anything failing, because the machine that breaks is
  # somebody else's. So the strip matches the marker's stable **prefix** instead of a list of past
  # spellings, and a third move needs no edit here. The end marker names no path and already
  # terminated all of them.
  begin_pfx="# >>> karaokemachine android (written by "
  mkdir -p "$(dirname "$config")"
  [ -f "$config" ] || : >"$config"

  # Rewritten in place each run, so an NDK upgrade is picked up by re-running this script rather than
  # by remembering to edit a file. Everything outside the markers is preserved untouched.
  tmp="$config.km.$$"
  awk -v bp="$begin_pfx" -v e="$end" '
    index($0, bp) == 1 { skip = 1 } { if (!skip) print } $0 == e { skip = 0 }
  ' "$config" >"$tmp"

  # **Inserted inside the existing `[env]` table rather than appended with an `[env]` of its own.**
  # TOML forbids defining one table twice, and tools/setup/fetch-ffmpeg.sh has very likely written an
  # `[env]` already -- two of them is a config cargo refuses to read at all, which would break every
  # build on this machine rather than only an Android one. Comments are legal inside a table, so the
  # markers go there with the key.
  # One line per Android triple. The include directory in each is the one cargo-ndk would have
  # supplied (`<sysroot>/usr/include/<arch triple>`), which differs from the Rust triple for 32-bit
  # ARM: `arm-linux-androideabi`, not `armv7-`.
  entries=""
  for pair in "aarch64_linux_android:aarch64-linux-android" \
              "armv7_linux_androideabi:arm-linux-androideabi"; do
    var="BINDGEN_EXTRA_CLANG_ARGS_${pair%%:*}"
    arch="${pair##*:}"
    entries="$entries$var = { value = \"--sysroot=$w_sysroot -I$w_sysroot/usr/include/$arch -I$w_res/include\", force = true }
"
  done

  if grep -q '^\[env\]' "$tmp"; then
    awk -v b="$begin" -v e="$end" -v ent="$entries" '
      /^\[env\]$/ && !done {
        print
        print b
        printf "%s", ent
        print e
        done = 1
        next
      }
      { print }
    ' "$tmp" >"$tmp.2" && mv "$tmp.2" "$tmp"
  else
    {
      printf '%s\n' "$begin"
      printf '[env]\n'
      printf '%s' "$entries"
      printf '%s\n' "$end"
    } >>"$tmp"
  fi

  mv "$tmp" "$config"
  say "-- cargo config: bindgen include paths pinned for both Android triples in $config"
}

# -- go --------------------------------------------------------------------------------------------

fetch_source
for abi in "${ABIS[@]}"; do
  build_abi "$abi"
done

write_cargo_config

say
say "ffmpeg $FF_SRC_VER is ready for: ${ABIS[*]}"
say "now:  tools/port/machine/android/build.sh"
