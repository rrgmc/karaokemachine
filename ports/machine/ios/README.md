# The machine on iOS

The machine itself, on an iPhone and an iPad: the same synthesizer, catalog, display and API as every
other build, in an application bundle. `crates/machine/km-machine-ios` is the `staticlib` it links,
and `tools/port/machine/ios/` is what builds it.

**macOS only**, and it needs the full Xcode rather than the Command Line Tools.

## Once per machine

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
brew install xcodegen
xcodebuild -downloadPlatform iOS        # if the iOS platform is not installed
tools/setup/fetch-assets.sh             # the SoundFont; the font is fetched by the build
```

`xcode-select` does **not** have to be pointing at Xcode: the build exports `DEVELOPER_DIR` for its
own process rather than asking for a machine-wide `sudo`. Set `XCODE=` if Xcode is somewhere other
than `/Applications/Xcode.app`. All of these are checked before anything is built.

## Build and run

```sh
task build:ios                                      # RELEASE=1, DEVICE=1, NOAPP=1
open ports/machine/ios/KaraokeMachine.xcodeproj     # pick the device, press Run
```

`DEVICE=1` skips the simulator slice, which is a whole second SDL and the reason the flag exists.

## What is different from the Android application, and why

| Android | iOS | Because |
|---|---|---|
| `cdylib`, loaded by `SDLActivity` | `staticlib`, linked into the app binary | iOS loads no arbitrary dynamic library and forbids `fork` and `exec` |
| `SDL_main` exported by `karaokemachine` | exported by `km-machine-ios` | a `no_mangle` symbol in an upstream rlib need not reach a staticlib that never references it |
| paths asked of SDL | paths handed down by `main.m` | there is no `SDL_GetIOSInternalStoragePath`, and `directories` answers with a macOS path outside the container |
| assets unpacked on first run | assets read where they are | an APK's assets are not files; a bundle is a filesystem |
| a real `libSDL3.so` in the package | SDL bundled inside our own archive | nothing here loads a library at run time, so static is what the platform wants |
| logcat, because stdout goes to `/dev/null` | stderr, which Xcode's console shows | there is no reason for a `km-ioslog` beside `km-androidlog` |
| four scripts | two | there is no staging step: Xcode consumes an xcframework directly |
| a JavaVM published for cpal | nothing | cpal's CoreAudio backend asks for no context |
| `armeabi-v7a` and `arm64-v8a` | `arm64` twice, device and simulator | every iOS device this runs on is 64-bit |

## What to know before it goes wrong

- **Never edit the project in Xcode.** It is generated from `project.yml` by every build, and the
  next one discards the change. Edit `project.yml`.
- **`build.sh` compiles the app rather than stopping at the project.** `xcodegen` writes a project
  without reading a line of `main.m`, so a script that stopped there would report success over a
  target that does not build. `--no-app` opts out.
- **Signing is Automatic**, from the development team named in `project.yml`. That is an Apple
  Development certificate for putting a build on your own devices, a different thing from the
  Developer ID used for macOS releases.
- **The assets are a folder reference, not a group.** `project.yml` says `type: folder`, which is
  what makes Xcode copy the tree to the bundle root with its structure intact. A group of file
  references would flatten it into `Resources/` and the machine would find nothing.
- **The bundle carries its own font.** `km-display`'s list of system font paths has no iOS entry, so
  `assets/fonts/karaoke.ttf` is the arm that answers. `tools/setup/fetch-assets.sh --font` pins
  DejaVu Sans, the face the Linux tarball stages, and the build stages it with its license.
- **`Documents/` is where a package arrives.** `UIFileSharingEnabled` and
  `LSSupportsOpeningDocumentsInPlace` put it in the Files app and in Finder over USB. There is no
  `adb push` here.
- **The machine advertises nothing over mDNS.** Multicast needs an entitlement Apple reviews by
  hand, and its absence is silent. The iOS remote finds a machine by unicast sweep and needs no
  record; the Android remote has to be given the address.
