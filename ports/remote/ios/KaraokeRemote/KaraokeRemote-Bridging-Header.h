/*
 * What Swift sees of the Rust server.
 *
 * The header itself lives beside the crate, at `crates/remote/km-remote-ios/include/km_remote.h`, and is
 * copied into the xcframework by `tools/port/remote/ios/build.sh`. Six functions; see `Server.swift`.
 */

#import "km_remote.h"
