# The km-tizen image: everything needed to cross-build the machine for a Samsung Tizen television.
# Built and run by tools/port/machine/tizen/build.sh, which passes every build argument below.
#
# The set is ARMv7 with the softfp calling convention (loader ld-linux.so.3) and glibc 2.40, so this
# is Debian's armel port raised with -march=armv7-a -mfloat-abi=softfp -mfpu=neon. Bookworm's glibc
# is 2.36, older than the set's, so nothing links against a symbol version the set lacks.
#
# Three things are built into the image, and two paths are a contract other scripts compile against:
#   /opt/sdl3    SDL3 with the Tizen guards (sdl3-tizen.patch) and SDL3_ttf, static and PIC. The
#                Wayland driver loads its libraries at run time; GLES on; X11, KMSDRM, Vulkan and
#                desktop GL off. Includes, lib/libSDL3.a, lib/libSDL3_ttf.a and their .pc files.
#   /opt/ffmpeg  ffmpeg at the pin in tools/setup/ffmpeg-pin.sh, shared. Link time only: on the set,
#                its own libavcodec.so.61 and friends answer, so nothing here is ever shipped.
# and Rust at rust-toolchain.toml's pin with the armv7-unknown-linux-gnueabi target.
#
# The SDL sources are the crates sdl3-sys and sdl3-ttf-sys would otherwise build, at the versions and
# checksums Cargo.lock records, so the patched library is the one the crates expect.

FROM debian:bookworm

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# No armel package that needs libc6-dev:armel: the archive's armel and amd64 halves of it are often
# at different versions, and apt then refuses. crossbuild-essential-armel brings its own libc.
RUN dpkg --add-architecture armel && apt-get update && apt-get upgrade -y && apt-get install -y --no-install-recommends \
      ca-certificates curl xz-utils patch file \
      crossbuild-essential-armel cmake ninja-build pkg-config make \
      gcc libc6-dev libclang-dev clang libwayland-bin \
      libwayland-dev:armel libxkbcommon-dev:armel libasound2-dev:armel \
 && rm -rf /var/lib/apt/lists/*

COPY armel.cmake /opt/km-tizen/armel.cmake
ENV KM_ARMEL_CFLAGS="-march=armv7-a -mfloat-abi=softfp -mfpu=neon"

# -- SDL3, patched -----------------------------------------------------------------------------------

ARG SDL3_SRC_VERSION
ARG SDL3_SRC_SHA256
COPY sdl3-tizen.patch /opt/km-tizen/sdl3-tizen.patch
RUN set -e; mkdir -p /usr/src/sdl3 && cd /usr/src/sdl3 \
 && curl --proto '=https' -sSfL -o sdl3-src.crate \
      "https://static.crates.io/crates/sdl3-src/sdl3-src-${SDL3_SRC_VERSION}.crate" \
 && echo "${SDL3_SRC_SHA256}  sdl3-src.crate" | sha256sum -c - \
 && tar -xzf sdl3-src.crate --strip-components=1 && rm sdl3-src.crate \
 && (cd SDL && patch -p1 --forward < /opt/km-tizen/sdl3-tizen.patch) \
 # SDL's Wayland check asks pkg-config for egl. The set has libEGL; SDL carries the headers.
 && mkdir -p /opt/km-tizen/pc \
 && printf 'Name: egl\nDescription: EGL headers bundled with SDL\nVersion: 1.5\nCflags: -I/usr/src/sdl3/SDL/src/video/khronos\nLibs:\n' \
      > /opt/km-tizen/pc/egl.pc \
 && PKG_CONFIG_LIBDIR=/opt/km-tizen/pc:/usr/lib/arm-linux-gnueabi/pkgconfig:/usr/share/pkgconfig \
    cmake -S SDL -B build -G Ninja -DCMAKE_TOOLCHAIN_FILE=/opt/km-tizen/armel.cmake \
      -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/opt/sdl3 \
      -DSDL_SHARED=OFF -DSDL_STATIC=ON -DSDL_STATIC_PIC=ON \
      -DSDL_TEST_LIBRARY=OFF -DSDL_TESTS=OFF -DSDL_EXAMPLES=OFF \
      -DSDL_X11=OFF -DSDL_KMSDRM=OFF -DSDL_VULKAN=OFF -DSDL_RENDER_VULKAN=OFF -DSDL_OPENGL=OFF \
      -DSDL_WAYLAND=ON -DSDL_WAYLAND_SHARED=ON -DSDL_WAYLAND_LIBDECOR=OFF -DSDL_OPENGLES=ON \
      -DSDL_ALSA=ON -DSDL_ALSA_SHARED=ON -DSDL_PULSEAUDIO=OFF -DSDL_PIPEWIRE=OFF -DSDL_JACK=OFF \
      -DSDL_SNDIO=OFF -DSDL_IBUS=OFF -DSDL_DBUS=OFF -DSDL_LIBUDEV=OFF -DSDL_HIDAPI=OFF \
      -DSDL_CAMERA=OFF \
 && cmake --build build && cmake --install build && rm -rf build

# -- SDL3_ttf, with its own FreeType and no HarfBuzz or plutosvg, as the workspace asks ---------------

ARG SDL3_TTF_SRC_VERSION
ARG SDL3_TTF_SRC_SHA256
RUN set -e; mkdir -p /usr/src/sdl3-ttf && cd /usr/src/sdl3-ttf \
 && curl --proto '=https' -sSfL -o sdl3-ttf-src.crate \
      "https://static.crates.io/crates/sdl3-ttf-src/sdl3-ttf-src-${SDL3_TTF_SRC_VERSION}.crate" \
 && echo "${SDL3_TTF_SRC_SHA256}  sdl3-ttf-src.crate" | sha256sum -c - \
 && tar -xzf sdl3-ttf-src.crate --strip-components=1 && rm sdl3-ttf-src.crate \
 && cmake -S SDL_ttf -B build -G Ninja -DCMAKE_TOOLCHAIN_FILE=/opt/km-tizen/armel.cmake \
      -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/opt/sdl3 \
      -DSDL3_DIR=/opt/sdl3/lib/cmake/SDL3 -DBUILD_SHARED_LIBS=OFF \
      -DSDLTTF_VENDORED=ON -DSDLTTF_HARFBUZZ=OFF -DSDLTTF_PLUTOSVG=OFF -DSDLTTF_SAMPLES=OFF \
      -DFT_DISABLE_ZLIB=ON -DFT_DISABLE_BZIP2=ON -DFT_DISABLE_PNG=ON -DFT_DISABLE_HARFBUZZ=ON \
      -DFT_DISABLE_BROTLI=ON \
 && cmake --build build && cmake --install build && rm -rf build

# -- ffmpeg, link time only ------------------------------------------------------------------------

# The pin's cross configure line, less zlib, which would need zlib1g-dev:armel (see the apt line);
# a link-time ffmpeg does not care what the set's own was built with. Symbol versioning is off so the machine's references bind to the
# set's libavcodec.so.61 whether or not Samsung built it with versions. Every hwcontext_*.h is
# removed afterwards: none is backed by this build, and ffmpeg-sys-next's Vulkan stub asserts a
# 64-bit struct size that fails on 32-bit ARM (the same fix as tools/port/machine/android/ffmpeg.sh).
COPY --from=setup ffmpeg-pin.sh /opt/km-tizen/ffmpeg-pin.sh
RUN set -e; . /opt/km-tizen/ffmpeg-pin.sh; mkdir -p /usr/src/ffmpeg && cd /usr/src/ffmpeg \
 && curl --proto '=https' -sSfL -o ffmpeg.tar.xz "$FF_SRC_URL" \
 && echo "$FF_SRC_SHA256  ffmpeg.tar.xz" | sha256sum -c - \
 && tar -xJf ffmpeg.tar.xz --strip-components=1 && rm ffmpeg.tar.xz \
 && flags=(); for f in "${FF_SRC_CONFIGURE_CROSS[@]}"; do [ "$f" = --enable-zlib ] || flags+=("$f"); done \
 && ./configure --prefix=/opt/ffmpeg "${flags[@]}" \
      --enable-cross-compile --cross-prefix=arm-linux-gnueabi- --target-os=linux \
      --arch=arm --cpu=armv7-a --enable-neon --enable-pic --disable-symver \
      "--extra-cflags=$KM_ARMEL_CFLAGS -fPIC" \
 && make -j"$(nproc)" && make install \
 && rm -f /opt/ffmpeg/include/libavutil/hwcontext_*.h \
 && cd / && rm -rf /usr/src/ffmpeg

# -- Rust ------------------------------------------------------------------------------------------

ARG RUST_VERSION
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl --proto '=https' -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal \
      --default-toolchain "$RUST_VERSION" --component rustfmt,clippy \
      --target armv7-unknown-linux-gnueabi

# The target is soft-float by default. The set has VFPv4 and NEON, and these flags let the code use
# them while keeping the softfp calling convention. Measured working on the set.
ENV CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_LINKER=arm-linux-gnueabi-gcc \
    CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_RUSTFLAGS="-C target-cpu=cortex-a72 -C target-feature=-soft-float,+neon,+vfp4" \
    CC_armv7_unknown_linux_gnueabi=arm-linux-gnueabi-gcc \
    CXX_armv7_unknown_linux_gnueabi=arm-linux-gnueabi-g++ \
    AR_armv7_unknown_linux_gnueabi=arm-linux-gnueabi-ar \
    CFLAGS_armv7_unknown_linux_gnueabi="-march=armv7-a -mfloat-abi=softfp -mfpu=neon" \
    CXXFLAGS_armv7_unknown_linux_gnueabi="-march=armv7-a -mfloat-abi=softfp -mfpu=neon" \
    PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_LIBDIR_armv7_unknown_linux_gnueabi=/opt/sdl3/lib/pkgconfig:/usr/lib/arm-linux-gnueabi/pkgconfig:/usr/share/pkgconfig \
    FFMPEG_DIR=/opt/ffmpeg \
    BINDGEN_EXTRA_CLANG_ARGS_armv7_unknown_linux_gnueabi="--sysroot=/usr/arm-linux-gnueabi -mfloat-abi=softfp"
