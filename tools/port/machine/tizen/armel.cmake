# CMake toolchain for a Samsung Tizen television, used inside the km-tizen image only: Debian's armel
# cross-compiler (the set's softfp ABI, loader ld-linux.so.3) raised to ARMv7 with NEON.
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR arm)
set(CMAKE_C_COMPILER arm-linux-gnueabi-gcc)
set(CMAKE_CXX_COMPILER arm-linux-gnueabi-g++)
set(CMAKE_C_FLAGS_INIT "-march=armv7-a -mfloat-abi=softfp -mfpu=neon -fPIC")
set(CMAKE_CXX_FLAGS_INIT "-march=armv7-a -mfloat-abi=softfp -mfpu=neon -fPIC")
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
set(CMAKE_FIND_ROOT_PATH /usr/arm-linux-gnueabi /usr/lib/arm-linux-gnueabi /opt/sdl3)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE BOTH)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE BOTH)
