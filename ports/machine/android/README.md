# The Android app

A thin Gradle wrapper around the Rust machine. **There is no `externalNativeBuild` block anywhere in
this project, on purpose** — cargo builds the native side and Gradle only packages it. That keeps one
build system responsible for the Rust and one for the APK, instead of Gradle driving CMake driving
cargo.

## Building

```sh
tools/setup/fetch-assets.sh        # once: the GM SoundFont, which is not committed
tools/port/machine/android/ffmpeg.sh  # once per machine: the LGPL ffmpeg, per ABI (video builds only)
tools/port/machine/android/build.sh   # cargo -> libkm_app.so, libSDL3.so, libSDL3_ttf.so (both ABIs)
tools/port/machine/android/stage.sh   # copy them into app/src/main/jniLibs/<abi>/
tools/port/machine/android/assets.sh  # bundle assets/ + the dev remote into app/src/main/assets/
cd ports/machine/android && ./gradlew assembleDebug
```

`task build:android` is those four steps under one name, and `task build:android:native` is the three
before Gradle — the cargo half, which needs no JDK. `RELEASE=1` reaches all four and puts optimized
native libraries inside a release APK. `ARM64=1` is `--arm64-only` and **also reaches both**, for the
reason below. `NO_VIDEO=1` is `--no-video`, and `task ffmpeg:android` is the once-per-machine step.

**`--arm64-only` has to be given to `stage.sh` as well as to `build.sh`**, and typing it on the first
line alone is the trap. Skipping an ABI leaves the library an earlier both-ABI build put in the
target directory, so staging copies that one and the APK carries it. That is not merely wasted space,
because a television loads `armeabi-v7a` and nothing else. The APK would then be current on a phone
and months old on the television. `stage.sh` removes any ABI directory the run did not stage, so the
two halves cannot disagree, and its closing warning depends on that.

The Taskfile wraps these commands and is not a substitute for them: `task` is optional, and every line
above works with it absent.

**Video is on by default**, as it is in every other release command. See the `Video in a release
build` decision in `docs/decisions/`. It needs `tools/port/machine/android/ffmpeg.sh` to have run
once per ABI. `build.sh` refuses to start rather than quietly producing an APK that lists video songs
and cannot read them.

That script cross-compiles the same pinned LGPL ffmpeg 7.1.5 the macOS bundle and the Linux tarball
carry, into `~/.cache/karaokemachine/assets/ffmpeg-android/<id>/<abi>/`. It needs nothing installed
that an Android build did not already need, because the NDK supplies clang **and** a GNU make,
including on Windows. It also writes two `force`d entries into cargo's own `[env]`, without which
bindgen cannot find clang's builtin headers. The header of the script says why, and why the obvious
fix for that is a trap. `--no-video` skips all of it and stages no ffmpeg libraries.

`assets.sh` is what makes the app work on a device without pushing files by hand. Android assets live
inside the APK rather than on the filesystem, so `km_app`'s `androidassets` module unpacks them into
app-private storage on first run. Its module docs say why a `MANIFEST` is needed and why no JNI is.
Skip the step and the machine still starts, on its test tone with a system font and a generated
gradient.

Out comes `app/build/outputs/apk/flat/debug/app-flat-debug.apk`, about 78 MB with video and the
wallpaper pack. `task build:android:quest` produces the headset's `app-headset-debug.apk` beside it,
from the same native libraries.
The four ffmpeg libraries across both ABIs are only ~5 MB of that. The SoundFont (31 MB) and a
wallpaper pack (19 MB) are most of it; both are assets, and `assets.sh` prints the total.

`task build:android RELEASE=1` writes `app/build/outputs/apk/flat/release/app-flat-release.apk`, signed
with the key `KM_ANDROID_KEYSTORE` names and with `android:debuggable` off. That is the one to hand
somebody; see `How the Android applications are signed` in `docs/decisions/remotes.md`.

**`JAVA_HOME` must name a JDK 17 or 21.** Gradle 8.12 fails on JDK 25 with `Unsupported class file
major version 69`, and Android Studio's bundled JBR *is* 25. The JDK already on the machine is
therefore the wrong one. `winget install Microsoft.OpenJDK.21`. A Gradle toolchain does not help:
what fails is the JVM compiling the build script, not the one compiling the app.

`ANDROID_HOME` must point at the SDK, or `local.properties` must set `sdk.dir` (it is not committed,
being machine-specific).

`app/build.gradle` fails the build with a readable message if the libraries have not been staged. The
alternative is an APK that installs and then dies at launch with `UnsatisfiedLinkError`, which says
nothing about the cause.

Worth knowing when editing the manifest: **a `--` cannot appear inside an XML comment**. The manifest
merger reports it only as `Error parsing AndroidManifest.xml`, with no line number unless you pass
`--stacktrace`. Writing out a `cargo … --example …` command in a comment is what tripped it.

`app/src/main/jniLibs/` is gitignored: it holds build products, and the debug `libkm_app.so` is over
150 MB.

## What is copied from SDL, and what is ours

The Java under `app/src/main/java/org/libsdl/app/` is **SDL's, verbatim** from the `android-project`
template in the vendored SDL source (zlib license). It is not to be edited — if it needs changing,
the SDL version has moved and it should be re-copied.

Ours:

| File | Why it differs from SDL's template |
|---|---|
| `MainActivity.java` | Names the libraries to load, in order: `SDL3`, `SDL3_ttf`, `km_app`. SDL's default would look for `main`. |
| `AndroidManifest.xml` | `INTERNET` and `CHANGE_WIFI_MULTICAST_STATE` for the API and mDNS; `screenOrientation="userLandscape"`, so a rotation cannot rebuild the GL surface mid-song — but see below, because **the manifest does not settle this on its own**; touch marked `required="false"`, so the app installs on a television that has none. |
| `app/build.gradle` | No native build. `minSdk 26`, ABI filters matching what cargo actually builds, and the staging check. |

### The manifest's landscape lock needs the SDL hint beside it

`SDLActivity.setOrientationBis` ends every path in an unconditional `setRequestedOrientation(...)`,
called from `Android_CreateWindow`, so **SDL replaces whatever the manifest asked for**. With no
`SDL_HINT_ORIENTATIONS` set it chooses by *resizability*, not by width and height, and our window is
resizable. It therefore picked `SCREEN_ORIENTATION_FULL_USER`, and a Galaxy S23 started portrait at
1080x2340, with the manifest saying landscape all along.

`crates/machine/karaokemachine/src/display.rs` therefore sets that hint to `LandscapeLeft
LandscapeRight` before `SDL_Init`, which yields `SCREEN_ORIENTATION_USER_LANDSCAPE`, the manifest's
own value. **Neither half is redundant.** Delete the hint and a phone rotates again; delete the
manifest attribute and the activity is unlocked for the moment before the window exists. A television
cannot show you either mistake, having one fixed landscape mode and no rotation sensor.

### The screen staying on is SDL's, and the manifest must not claim it

`SDL_VideoInit` disables the screensaver unless `SDL_HINT_VIDEO_ALLOW_SCREENSAVER` says otherwise,
and the hint defaults to false. Its own comment is that "most things using SDL are games or media
players". On Android that reaches `Android_JNI_SuspendScreenSaver` →
`window.addFlags(FLAG_KEEP_SCREEN_ON)`. Verified on a Streamer: the app's window reports
`fl=KEEP_SCREEN_ON`. **We ask for none of this and need not.**

**Do not put `android:keepScreenOn="true"` in the manifest.** It does nothing. `keepScreenOn` is a
`View` attribute and an `<activity>` element has no such thing, so Android ignores it silently while
SDL does the job. A line that appears to cause the behavior it is merely adjacent to is worse than no
line. The next person to change the screensaver hint would edit the manifest and watch nothing
happen.

**Note the contrast with orientation above**, which is the same shape and the opposite outcome. There
SDL overrules the manifest into the *wrong* answer and has to be told; here it arrives at the right
one unaided. In both cases SDL, not the manifest, is what decides.

## Why `minSdk 26`

`libaaudio.so` does not exist below API 26 and the audio backend links against it. Recorded as a
decision in `docs/decisions/`.

## Known: the JDK version

Gradle 8.12 cannot run on **JDK 25**. It fails with `Unsupported class file major version 69` while
parsing the build script, because its bundled Groovy predates that class file format. Every JetBrains
JBR shipped with a current Android Studio is JDK 25, so a JDK **17 or 21** has to arrive separately:

```sh
winget install Microsoft.OpenJDK.21
JAVA_HOME=/c/"Program Files"/Microsoft/jdk-21* ./gradlew assembleDebug
```

A Gradle *toolchain* does not solve this. The failure is in the daemon JVM that compiles the build
script, not in what compiles the app, and a toolchain only governs the latter.

## What the hardware has done

A Galaxy S23 (arm64, API 36) has played all three song kinds out of one package. MIDI, a real MP3+G
pair, and a 1080p30 H.264 video. **The Google TV Streamer has done the same**, over `armeabi-v7a`,
and had its remote pressed by hand. What follows is how much of it the box can afford:

- **`armeabi-v7a` decodes video on a Streamer.** That is the ABI where the 32-bit surprises live, the
  `LIBC_N` link and the Vulkan stub's 64-bit size assertion. It fails neither at load nor at play. A
  Streamer has played several H.264 songs end to end, presenting an exact 60 fps with no dropped
  vsync.
- **Decode headroom on the Streamer's 32-bit Cortex-A55 is measured, and software decode wins.**
  1080p30 H.264 High at 926 kbps costs **80% of one core** single-threaded. That is the whole margin
  against a 250 ms ring, and a 273-second song stalls: 871 ms of silence and a ~4.5 s frozen picture.
  **Frame threading is what answers it.** The same file, on the same device, reports
  `threads=5 kind=Frame` and plays all 273 seconds with `starved_ms` **0**.

  The numbers and the method are in
  [`docs/architecture/video.md`](../../../docs/architecture/video.md). The shipped build therefore
  stays on software decode, `h264_mediacodec` stays unselected, and nothing measured since argues
  otherwise.

  **A phone settles nothing here.** A Snapdragon 8 Gen 2 does not notice 1080p30, and reading its
  success as evidence about the television would miss the whole point. The **synthesizer** on that
  same A55 is measured too, in
  [`docs/architecture/audio.md`](../../../docs/architecture/audio.md). It peaks near 82% of one core,
  so decode does not arrive on an idle machine.
- **An MP3+G pair has now played on the television too**, so every song kind has run on this device.
  It is the cheapest of the three to draw, at 0.3 ms a frame against the lyric ladder's few. A CD+G
  plane is a texture upload with no text shaped at all.
- **The physical remote has now been pressed, and nothing here is unverified any more.** Arrows move
  the highlight, OK activates it, BACK stops a playing song and then leaves from idle, and no
  joystick device is ever added. A real press found two things injection could not. The first D-pad
  press after the strip fades is swallowed waking it. The idle number pad had to come back on for
  every Android — see [`The on-screen number
  pad`](../../../docs/decisions/interface.md#the-on-screen-number-pad).
