# The km-webos image: what cross-builds the webOS probe for an LG television on webOS 5.
# Built and run by tools/port/machine/webos/build.sh, which passes RUST_VERSION.
#
# The set is ARMv7 with the softfp calling convention (loader ld-linux.so.3) and glibc 2.28. Buster's
# glibc is 2.28 too, so nothing links against a symbol version the set lacks. This is the km-tizen
# image's arrangement, one Debian release older. Buster is past its support, so apt reads the archive.

FROM debian:buster

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

RUN printf 'deb http://archive.debian.org/debian buster main\ndeb http://archive.debian.org/debian-security buster/updates main\n' > /etc/apt/sources.list \
 && apt-get update && apt-get upgrade -y && apt-get install -y --no-install-recommends \
      ca-certificates curl file gcc-arm-linux-gnueabi libc6-dev-armel-cross \
 && rm -rf /var/lib/apt/lists/*

ARG RUST_VERSION
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl --proto '=https' -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal \
      --default-toolchain "$RUST_VERSION" --target armv7-unknown-linux-gnueabi

# The target is soft-float by default. The set has VFPv4 and NEON (Cortex-A55 cores under a 32-bit
# kernel), and these flags let the code use them while keeping the softfp calling convention.
ENV CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_LINKER=arm-linux-gnueabi-gcc \
    CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_RUSTFLAGS="-C target-cpu=cortex-a55 -C target-feature=-soft-float,+neon,+vfp4" \
    CC_armv7_unknown_linux_gnueabi=arm-linux-gnueabi-gcc \
    CFLAGS_armv7_unknown_linux_gnueabi="-march=armv7-a -mfloat-abi=softfp -mfpu=neon"

# -- SDL3, patched for webOS ---------------------------------------------------------------------
#
# /opt/sdl3 holds SDL3 static and PIC, with the Wayland driver loading its libraries at run time and
# GLES on; everything else is off. The set's libwayland predates 1.18, which SDL3 asks for. So the
# headers and the scanner are wayland 1.18's, built here for the build machine: 1.18's scanner
# still emits wl_proxy_marshal_constructor, which the set has, where 1.20's emits marshal_flags.
# sdl3-webos.patch makes the two 1.18 functions SDL needs, the proxy tags, optional.

RUN apt-get update && apt-get install -y --no-install-recommends \
      make pkg-config patch xz-utils ninja-build gcc g++-arm-linux-gnueabi libc6-dev \
      libffi-dev libexpat1-dev libxkbcommon-dev \
 && rm -rf /var/lib/apt/lists/*

ARG CMAKE_VERSION=3.31.6
ARG CMAKE_SHA256=5a1133ff103c71eb5120e2cc3de922733e7d8a26a98ae716397e8676adb367bf
RUN curl --proto '=https' -sSfL -o /tmp/cmake.tar.gz \
      "https://github.com/Kitware/CMake/releases/download/v${CMAKE_VERSION}/cmake-${CMAKE_VERSION}-linux-x86_64.tar.gz" \
 && echo "${CMAKE_SHA256}  /tmp/cmake.tar.gz" | sha256sum -c - \
 && mkdir /opt/cmake && tar -xzf /tmp/cmake.tar.gz -C /opt/cmake --strip-components=1 && rm /tmp/cmake.tar.gz
ENV PATH=/opt/cmake/bin:$PATH

ARG WAYLAND_SHA256=4675a79f091020817a98fd0484e7208c8762242266967f55a67776936c2e294d
RUN mkdir -p /usr/src/wayland && cd /usr/src/wayland \
 && curl --proto '=https' -sSfL -o wayland.tar.xz https://wayland.freedesktop.org/releases/wayland-1.18.0.tar.xz \
 && echo "${WAYLAND_SHA256}  wayland.tar.xz" | sha256sum -c - \
 && tar -xJf wayland.tar.xz --strip-components=1 \
 && ./configure --prefix=/opt/wayland --disable-documentation --disable-dtd-validation \
 && make -j"$(nproc)" && make install && cd / && rm -rf /usr/src/wayland
ENV PATH=/opt/wayland/bin:$PATH

COPY armel.cmake /opt/km-webos/armel.cmake
COPY sdl3-webos.patch /opt/km-webos/sdl3-webos.patch
ARG SDL3_SRC_VERSION
ARG SDL3_SRC_SHA256
RUN set -e; mkdir -p /usr/src/sdl3 && cd /usr/src/sdl3 \
 && curl --proto '=https' -sSfL -o sdl3-src.crate \
      "https://static.crates.io/crates/sdl3-src/sdl3-src-${SDL3_SRC_VERSION}.crate" \
 && echo "${SDL3_SRC_SHA256}  sdl3-src.crate" | sha256sum -c - \
 && tar -xzf sdl3-src.crate --strip-components=1 && rm sdl3-src.crate \
 && (cd SDL && patch -p1 --forward < /opt/km-webos/sdl3-webos.patch) \
 # SDL's Wayland check asks pkg-config for egl. The set has libEGL; SDL carries the headers.
 && mkdir -p /opt/km-webos/pc \
 && printf 'Name: egl\nDescription: EGL headers bundled with SDL\nVersion: 1.5\nCflags: -I/usr/src/sdl3/SDL/src/video/khronos\nLibs:\n' \
      > /opt/km-webos/pc/egl.pc \
 && PKG_CONFIG_LIBDIR=/opt/km-webos/pc:/opt/wayland/lib/pkgconfig:/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig \
    cmake -S SDL -B build -G Ninja -DCMAKE_TOOLCHAIN_FILE=/opt/km-webos/armel.cmake \
      -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/opt/sdl3 \
      -DSDL_SHARED=OFF -DSDL_STATIC=ON -DSDL_STATIC_PIC=ON \
      -DSDL_TEST_LIBRARY=OFF -DSDL_TESTS=OFF -DSDL_EXAMPLES=OFF \
      -DSDL_X11=OFF -DSDL_KMSDRM=OFF -DSDL_VULKAN=OFF -DSDL_RENDER_VULKAN=OFF -DSDL_OPENGL=OFF \
      -DSDL_WAYLAND=ON -DSDL_WAYLAND_SHARED=ON -DSDL_WAYLAND_LIBDECOR=OFF -DSDL_OPENGLES=ON \
      -DSDL_ALSA=OFF -DSDL_PULSEAUDIO=OFF -DSDL_PIPEWIRE=OFF -DSDL_JACK=OFF \
      -DSDL_SNDIO=OFF -DSDL_IBUS=OFF -DSDL_DBUS=OFF -DSDL_LIBUDEV=OFF -DSDL_HIDAPI=OFF \
      -DSDL_CAMERA=OFF \
 && (grep -rh "define SDL_VIDEO_DRIVER_WAYLAND_DYNAMIC" build/ || true) \
 && cmake --build build && cmake --install build && rm -rf build

# -- SDL3_ttf, with its own FreeType and no HarfBuzz or plutosvg, as the workspace asks ---------------

ARG SDL3_TTF_SRC_VERSION
ARG SDL3_TTF_SRC_SHA256
RUN set -e; mkdir -p /usr/src/sdl3-ttf && cd /usr/src/sdl3-ttf \
 && curl --proto '=https' -sSfL -o sdl3-ttf-src.crate \
      "https://static.crates.io/crates/sdl3-ttf-src/sdl3-ttf-src-${SDL3_TTF_SRC_VERSION}.crate" \
 && echo "${SDL3_TTF_SRC_SHA256}  sdl3-ttf-src.crate" | sha256sum -c - \
 && tar -xzf sdl3-ttf-src.crate --strip-components=1 && rm sdl3-ttf-src.crate \
 && cmake -S SDL_ttf -B build -G Ninja -DCMAKE_TOOLCHAIN_FILE=/opt/km-webos/armel.cmake \
      -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/opt/sdl3 \
      -DSDL3_DIR=/opt/sdl3/lib/cmake/SDL3 -DBUILD_SHARED_LIBS=OFF \
      -DSDLTTF_VENDORED=ON -DSDLTTF_HARFBUZZ=OFF -DSDLTTF_PLUTOSVG=OFF -DSDLTTF_SAMPLES=OFF \
      -DFT_DISABLE_ZLIB=ON -DFT_DISABLE_BZIP2=ON -DFT_DISABLE_PNG=ON -DFT_DISABLE_HARFBUZZ=ON \
      -DFT_DISABLE_BROTLI=ON \
 && cmake --build build && cmake --install build && rm -rf build

# -- The machine's other native needs ---------------------------------------------------------------
#
# ALSA's headers and link library for cpal, armel; the set has libasound.so.2. Installing them
# through multiarch pulls an armel libc that conflicts with the cross one, so the two packages are
# unpacked into /opt/alsa-armel and given a .pc of their own. DejaVu is the font the .ipk carries in
# assets/fonts, since which fonts the set has is not known.
RUN dpkg --add-architecture armel && apt-get update \
 && apt-get install -y --no-install-recommends fonts-dejavu-core \
 && cd /tmp && apt-get download libasound2:armel libasound2-dev:armel \
 && for d in libasound2*_armel.deb; do dpkg -x "$d" /opt/alsa-armel; done && rm -f libasound2*_armel.deb \
 && mkdir -p /opt/km-webos/pc-armel \
 && printf 'prefix=/opt/alsa-armel/usr\nlibdir=${prefix}/lib/arm-linux-gnueabi\nincludedir=${prefix}/include\nName: alsa\nDescription: ALSA, armel\nVersion: 1.1.8\nLibs: -L${libdir} -lasound\nCflags: -I${includedir}\n' \
      > /opt/km-webos/pc-armel/alsa.pc \
 && rm -rf /var/lib/apt/lists/*

ENV CXX_armv7_unknown_linux_gnueabi=arm-linux-gnueabi-g++ \
    AR_armv7_unknown_linux_gnueabi=arm-linux-gnueabi-ar \
    CXXFLAGS_armv7_unknown_linux_gnueabi="-march=armv7-a -mfloat-abi=softfp -mfpu=neon" \
    PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_LIBDIR_armv7_unknown_linux_gnueabi=/opt/sdl3/lib/pkgconfig:/opt/km-webos/pc-armel
