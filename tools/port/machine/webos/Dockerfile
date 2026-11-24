# The km-webos image: what cross-builds the webOS probe for an LG television on webOS 5.
# Built and run by tools/port/machine/webos/build.sh, which passes RUST_VERSION.
#
# The set is ARMv7 with the softfp calling convention (loader ld-linux.so.3) and glibc 2.28. Buster's
# glibc is 2.28 too, so nothing links against a symbol version the set lacks. This is the km-tizen
# image's arrangement, one Debian release older. Buster is past its support, so apt reads the archive.

FROM debian:buster

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

RUN printf 'deb http://archive.debian.org/debian buster main\n' > /etc/apt/sources.list \
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
