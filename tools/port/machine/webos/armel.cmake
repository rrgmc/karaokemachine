# CMake toolchain for an LG webOS 5 television, used inside the km-webos image only: Debian buster's
# armel cross-compiler (the set's softfp ABI, loader ld-linux.so.3) raised to ARMv7 with NEON.
#
# Libraries are searched outside the armel root on purpose. SDL3 loads Wayland and xkbcommon at run
# time and reads only their file names at build time, and the copies in the image are the build
# machine's own (wayland 1.18 under /opt/wayland, xkbcommon from Debian).
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR arm)
set(CMAKE_C_COMPILER arm-linux-gnueabi-gcc)
set(CMAKE_CXX_COMPILER arm-linux-gnueabi-g++)
set(CMAKE_C_FLAGS_INIT "-march=armv7-a -mfloat-abi=softfp -mfpu=neon -fPIC")
set(CMAKE_CXX_FLAGS_INIT "-march=armv7-a -mfloat-abi=softfp -mfpu=neon -fPIC")
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
set(CMAKE_FIND_ROOT_PATH /usr/arm-linux-gnueabi)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY BOTH)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE BOTH)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE BOTH)
