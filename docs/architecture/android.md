# Android

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## The build

**Every crate cross-compiles and links for `aarch64-linux-android` and `armv7-linux-androideabi`.**
Six of the seven needed **no changes at all**, which is the pure-Rust bet paying off exactly as
planned. `rusqlite`'s bundled SQLite compiled with the NDK's clang without comment.

Four things had to be fixed, and each is a comment in `tools/port/machine/android/build.sh`:

1. **CMake defaulted to the Visual Studio generator**, so building SDL3 for Android tried to compile a
   `.vcxproj` with MSBuild. Needs `CMAKE_GENERATOR=Ninja`.
2. **The `cmake` crate guessed `CMAKE_SYSTEM_PROCESSOR=arm64`**, and CMake's Android support rejects
   that: it wants `aarch64`. Handing it the NDK's own toolchain file skips the guessing.
3. **`unable to find library -laaudio`.** AAudio is API 26+ and `cargo-ndk` defaults to 21. The flag
   is `-P`. Lowercase `-p` reaches cargo as `--package` and fails with "unknown package: 26".
4. **`symbol __aeabi_memcpy8@@LIBC_N has undefined version LIBC_N`**, on 32-bit ARM only; see below.
   It lives in `.cargo/config.toml` rather than the build script, because it has to apply to any cargo
   invocation for that target.

A fifth, **a wall of `undefined symbol: operator new`**, was fixed and then unfixed, and the second
answer is the right one. Unlike every desktop platform, the NDK does not link libc++ implicitly. So
`km-display/build.rs` was taught to link `c++_static` and add the NDK sysroot to the search path.
**That build script was later deleted outright**, and *The first launch crashed* below gives the
reasoning. SDL is built **shared** on Android, so its C++ runtime lives inside `libSDL3.so`, private
and unexported. The library imports no C++ symbols at all, so it needs no C++ runtime and no extra
search path.

**SDL is linked shared on Android and static everywhere else, and the split has to be declared in
*both* `km-display` and `km-app`.** Cargo features are additive. One crate asking for a static SDL
puts a static SDL in the build, whatever the other asks for. Getting it wrong in one silently undid
the split next door: the library came out with no `libSDL3.so` dependency at all. The symbol table
verified this, not an assumption.

### A false alarm that looks exactly like the real one

`libSDL3.so` contains all 68 `Java_org_libsdl_app_*` functions and exports **none** of them. That
looks exactly like the bug that would make an APK die with `UnsatisfiedLinkError`.

It is not a bug. **SDL registers its native methods dynamically** from `JNI_OnLoad`, and `JNI_OnLoad`
*is* exported. So its version script lists only `SDL_*` and that one symbol, and SDL treats the
version script as **required** on Android. Widening the export list would have been a change away
from what SDL intends.

Recorded because the symbol tables read like a fault and the correct behavior is not obvious from
them. Anyone checking this the same way will reach the same wrong conclusion. **The check that settles
it is `JNI_OnLoad` being present in `.dynsym`, not the absence of `Java_*`.**

### The armv7 `cdylib` links — one declared version node

This mattered more than its size suggests. Both Google TV devices run a 32-bit OS and load
`armeabi-v7a` and nothing else, so no television can run an arm64-only build.

The cause, read off the files rather than inferred: Rust's prebuilt `compiler_builtins` defines the
ARM EABI memory helpers as **weak and unversioned**. bionic exports the same twelve and tags them
`LIBC_N` from API 24 on. lld takes the version off the NDK stub and carries it onto the definition
that wins the link, which is ours. It then refuses, because the *output* declares no such node.

**The fix is to declare the node and put nothing in it.** lld accepts several version scripts and
merges them, so rustc's generated one still does its job. The only trace is one inert
`.gnu.version_d` entry.

**Two plausible fixes are wrong, and both cost time.** Most search results suggest
`-Wl,--undefined-version`, and it has no effect. It governs whether a version script may *name* an
undefined symbol, not whether a version node exists. It stayed inert even when passed last, where it
would win any ordering race. And changing API level is a dead end: `LIBC_N` is absent below 24, but
AAudio starts at 26.

## The APK

`ports/machine/android/` holds the Gradle project. **No `externalNativeBuild` block anywhere, on
purpose.** Cargo builds the native side and Gradle only packages it. So one build system owns the
Rust and one owns the APK, rather than Gradle driving CMake driving cargo. Staging copies the
libraries into `jniLibs/`, and the build fails with a readable message if they are not there. The
alternative is an APK that installs and dies at launch.

SDL's Java is copied verbatim from the vendored template and is not to be edited.

**Two APKs come out of the one project, and `src/main/` is what they share.** The `flat` flavour is
the phone and the television. The `headset` flavour is Meta Horizon OS, where a Kotlin
`ImmersiveActivity` owns a scene and hosts `MainActivity` on a panel. The native libraries and the
unpacked assets are staged into the main source set, so both flavours get them and
`tools/port/machine/android/stage.sh` knows about neither.

What the flavours differ in is small and all of it is in `app/build.gradle`. `headset` takes an
`applicationIdSuffix` of `.quest`, a `minSdk` of 34 where the shared default is 26, and
`arm64-v8a` alone. Its Spatial SDK dependencies are scoped `headsetImplementation`, so the flat APK
carries no Kotlin runtime at all. Each flavour's manifest holds one thing: how the machine is
launched. A home screen and a television's home row in one, an immersive scene in the other.

**`checkNativeLibs` keeps matching, and that is worth knowing rather than rediscovering.** AGP names
the merge task `merge<Flavour><BuildType>JniLibFolders`, and the `tasks.configureEach` predicate
tests the ends of the name rather than the whole of it.

**Three manifest entries in the headset flavour each fail silently when absent**, and none of them
reports itself to the application:

1. **`com.oculus.feature.PASSTHROUGH`.** Without it the scene draws a black void, however many times
   it calls `enablePassthrough`. `PT is: ON` in the log says only that the headset offers
   passthrough, and `numLayers` beside it says whether the scene submits a layer.
2. **`oculus.software.handtracking`.** Horizon OS refuses to start an immersive application when no
   controller is awake, and the refusal is a system dialog reading
   `app_launch_blocked_controller_required`. Nothing reaches the application, and the launch simply
   does not happen.
3. **`com.oculus.supportedDevices`.** A store listing needs it, and a sideload does not.

**A panel bends in place, and `PanelSceneObject.reshape()` is what does it.** Rebuilding the scene to
change a screen's shape takes the machine down with it, because `AppSystemActivity` does not survive
`recreate()`. A reshape leaves the song playing. A curved screen also costs nothing, running at 90
frames a second with no stale frames, the same as a flat one.

**`VRFeature` draws a controller's ray and no hand's.** `IsdkFeature`, from Meta's Interaction SDK,
is what a hand points with. A headset whose controllers are flat has no other way to reach the
keypad, so both features are registered. Spatial SDK 0.14.0 marks `IsdkFeature` deprecated and says
`VRFeature` registers it. The explicit registration stays until a headset shows hands working
without it.

**The screen's place is saved against a wall of the scanned room.** Spatial SDK 0.14.0 has no public
persistent spatial anchor: `Scene.createUserAnchor` is internal. `MRUKFeature` does hand over the
scanned room, and its wall anchors are stable across sessions. So `Placement.kt` stores the
screen's pose relative to the nearest wall, by that wall's UUID, in the `headset` preferences. A
restore multiplies the wall's current pose by the saved one.

**`IsdkGrabbable` moves a panel and `IsdkPanelResize` resizes it.** The resize runs in
`ResizeMode.Simple`, which writes the entity's `Scale` and leaves the panel's dp layout alone.
`ResizeMode.Relayout` would resize SDL's surface instead. `onSceneTick` watches the grab state and
the active resize corner, and saves once when both let go.

**The wall's facing is flipped towards the wearer rather than trusted.** The sign of a plane
anchor's forward axis is not measured here, so `Placement.onWall` points the normal at the viewer.
A panel shows its face along its own negative Z, so the screen's forward points into the wall.

**The queue panel is a WebView on `http://127.0.0.1:<port>/`.** The port comes from `api.bind` in
the machine's own `settings.json`, which lives in `Context.getFilesDir()`. A missing file means 8177,
and the panel retries every two seconds while the machine starts. Android blocks cleartext to
loopback as well, so the headset manifest names `res/xml/network_security_config.xml`. That file
permits `127.0.0.1` and `localhost` and nothing else, as the remote's application does.

**The controls are Compose, in Horizon OS's UI Set.** `meta-spatial-sdk-compose` gives a panel a
`ComposeView`, and `meta-spatial-sdk-uiset` gives the buttons. The Compose compiler plugin reaches
only Kotlin, and the `flat` flavour has none. Compose is pinned at the version the UI Set declares.

**The metrics overlay is in a debug build only.** `src/headsetDebug/` and `src/headsetRelease/` each
hold a `debugFeatures()`. The debug one returns `OVRMetricsFeature` with the scene's tick and object
counts, and the release one returns nothing. The dependency is `headsetDebugImplementation`, so a
release APK carries none of it. The overlay draws only while the OVR Metrics Tool runs.

**`singleInstance` survives embedding, and `MainActivity` keeps it.** A panel hosts an activity on a
virtual display, which looks like it should want the ordinary launch mode, and it does not. What
rides on that is the `.kmpkg` route, because `onNewIntent` exists only because of `singleInstance`.
It is the only way songs reach a headset without a cable.

**It needs a JDK 17 or 21.** Gradle 8.12 will not run on JDK 25. It reports `Unsupported class file major
version 69` while parsing the build script, because its bundled Groovy predates that format. **A
Gradle *toolchain* does not help.** The failure is in the daemon JVM that compiles the build script,
and a toolchain only governs what compiles the app.

Three faults, all of which produce errors naming the wrong thing:

1. **`jniLibs.useLegacyPackaging` inside a `buildTypes` block.** It is not a build-type property, and
   Gradle reports `Could not get unknown property 'jniLibs' for BuildType`, which reads like a plugin
   version problem. It was removed rather than relocated. The modern default already stores native
   libraries uncompressed and maps them from the APK. The comment that justified the setting had the
   flag's meaning backwards. Legacy packaging *compresses* and extracts at install, which keeps two
   copies of a very large library on the device.
2. **A `--` inside an XML comment in the manifest.** XML forbids it, and the merger reports only
   `Error parsing AndroidManifest.xml` with no line number unless you ask for `--stacktrace`.
3. **`CHANGE_WIFI_MULTICAST_STATE` implies `android.hardware.wifi` as *required*.** Nothing asked for
   it. Left alone, it filters the app off any device with no Wi-Fi radio. An Ethernet-only television
   box is exactly the case worth keeping. **Invisible until there was an APK to inspect.**

## Assets ship in the APK

**Android assets are not files.** They live inside the APK and only the asset API can read them,
while everything that consumes them wants a *path*. So the machine unpacks them once, on first run,
into the directory the asset resolver already points at. Nothing downstream changes.

- **SDL does the reading, so there is no JNI and no `unsafe`.** On Android, `SDL_IOFromFile` tries
  internal storage for a relative path and then falls back to the APK's asset system. The `sdl3`
  crate wraps it as something implementing `io::Read`, so unpacking is `io::copy`.
- **A `MANIFEST` says what is in there**, because SDL can open an asset by name but cannot *list* an
  asset directory. Sizes are included, so editing a file without adding or removing one still
  triggers a re-unpack. The unpacking checks each file's length, because a short read is otherwise
  silent. It surfaces much later as a truncated SoundFont.
- **The manifest is the completion marker**, written **last**. So a failure part-way through leaves
  the directory looking unfinished, and the next launch retries rather than trusting it.
- **An asset that leaves the manifest is deleted**, which for three releases it was not.
  App-private storage survives an upgrade. So unpacking that only ever adds and overwrites leaves
  every file a past build shipped on the device for ever. That is invisible until something *lists*
  a directory rather than naming what it wants, and the display lists the wallpapers folder. A zip
  of seven photographs replaced four gradient PNGs. Every device that had run the older build then
  showed eleven wallpapers, four of them the ones the change was meant to retire.

  A Google TV Streamer showed it. The old manifest is the record of what to remove, and the unpacking
  touches only the paths it lists. So a file somebody put there by hand is left alone. A removal that
  fails is logged rather than fatal. Withholding the manifest to force a retry would re-copy thirty
  megabytes on every launch for ever.

The staging script warns when no `.sf2` is present. The machine would then come up on its test tone,
which sounds broken to anyone who does not know it is the documented fallback. The script also
notes when the total passes 64 MiB, since unpacking duplicates the assets and the device pays roughly
twice.

## It runs on hardware

Deployed to a **Galaxy S23** (arm64) and a **Google TV Streamer** (armv7). The machine starts, renders
and stays up. And **AAudio opens *and starts***, which retires the load-bearing assumption of the
whole plan: cpal works on Android.

**The Streamer has no 64-bit ABI at all.** It is not a 64-bit chip running a 32-bit userspace: the
property is empty. An arm64-only APK could not have run under any circumstances. So the `LIBC_N`
version-node fix was not insurance; it was the difference between having a product on this device
and not.

**The remote's D-pad arrives as keyboard events**, and SDL never added a joystick device. So the
worry from the research note does not materialise. `required="false"` on touchscreen was equally
load-bearing: without it the app would not have installed.

| | |
|---|---|
| Display | 1920×1080 on the television, 2340×1080 on the phone |
| RSS with the 148 MB override bank | **313 MB**, graphics flat at 43 MB — comfortable in a 32-bit address space |
| BACK | stops a playing song, then exits from idle |

**Confirmed with a thumb on the actual remote.** Arrows move the highlight. OK activates it: `PAUSE`
fired, and the API agreed. BACK stops a playing song without leaving. BACK from idle leaves cleanly,
with `shutting down` and `the machine stopped normally`, the activity closed and the launcher back in
front.

**No joystick device appeared**, and `dumpsys input` says why. The remote is
`Google TV Remote Keyboard`, `Classes: KEYBOARD | DPAD | MIC | BATTERY | EXTERNAL`, with no
`GAMEPAD`. The two GAMEPAD-classed devices on the box are `virtual-search` and `virtual-remote`.

Only a real press could have found two things. First, the transport strip swallows the first D-pad
press after it fades, because that press wakes it; `display.rs` states the reveal-then-act rule.
Second, the idle number pad had to be switched back on for every Android; see [`The on-screen
number pad`](../decisions/interface.md#the-on-screen-number-pad).

### The first launch crashed, and the cause was ours

`SIGSEGV`, null read, during `dlopen`, before `SDL_main`. The library's `.init_array` held
**statically linked bionic**: the allocator, the trace helper and the DNS resolver. Their
constructors ran against libc state that already belonged to the real `libc.so`. `DT_NEEDED` listed
SDL3, SDL3_ttf, aaudio and log, and **no `libc.so` at all**.

The cause was the build script above. It added the NDK sysroot's `usr/lib/<triple>` to the link
search path so rustc could find `libc++_static.a`. **That directory also holds `libc.a`, `libm.a` and
`libdl.a`, while the shared stubs live in the per-API-level subdirectory below it.** So `-lc`, `-ldl`
and `-lm` all resolved to static archives.

Two things are worth keeping. **A missing `DT_NEEDED` entry is a symptom.** Reading that list against
a correct Android library would have found it before the device did. The first check read it against
the other ABI instead, and that is why it was missed. And **a `-L` on an NDK sysroot is never
harmless**: link the archive by absolute path instead.

### Logs had to be made visible first

Nothing the machine logs reaches anywhere by default. The default subscriber writes to a stdout the
zygote has pointed at `/dev/null`. **A device that fails silently cannot be debugged.**
`km-androidlog` routes events through `__android_log_write`, and `liblog` is already linked for SDL,
so this costs no dependency. It compiles under `cfg(test)` on every platform, not just Android, so its
line buffering is actually exercised. That immediately caught a bug where every event emitted a
trailing blank line.

### Audio: the context nobody published

AAudio opens and closes 3 ms later, and a ten-second timeout gets the blame. `Disconnected` and
`Timeout` are two answers from the receive and must be two match arms. Folded into one, the message
names the wrong cause and sends the reader ten seconds in the wrong direction.

Finding the panic needed a **panic hook that logs**, since a panic's report goes to the same discarded
stderr. With that in place: `android context was not initialized`.

cpal's Android backend reaches Java through `ndk_context`. That crate reads a JavaVM and an Activity
from a global that **something else is expected to have filled in**. Normally that is `ndk-glue` or
`android-activity`, and we use neither. The activity is SDL's, and **SDL keeps its JNI handles to
itself**. So the global stayed empty and the first caller panicked.

- **The VM comes from our own `JNI_OnLoad`.** Android calls that on every library load and hands over
  the `JavaVM*` directly: no JNI call, nothing to borrow, and it runs before `SDL_main`. The
  alternative is worse in this dependency set: asking SDL for a `JNIEnv*` and calling `GetJavaVM`
  through the function table.
- **The Activity comes from `SDL_GetAndroidActivity`**, which returns a *local* reference. The shell
  promotes it to a global one and deliberately never releases it. `ndk-context` keeps the pointer for
  the life of the process, and cpal dereferences it on the audio thread.
- **Both crates are pinned with `=` to exactly what cpal resolves.** That is the point rather than
  caution. `ndk-context` keeps the context in a private static. So a different version would be a
  different crate, a different static, and an initialization cpal never sees.

### Audio focus, and the first call from Rust into Java

`audiofocus.rs` is the seam, and it is built on every platform. The policy is a pure function, so
every branch of it is tested on a desktop. Only three things are Android's: a request, an abandon,
and the listener.

**Rust calls a method on the Activity rather than on a class it looked up.** JNI's `FindClass`
searches the system class loader when a thread the JVM did not start runs it. That loader knows
nothing of an application's own classes, and the watchdog thread is such a thread. Calling
`requestAudioFocus` on the Activity object asks that object's class directly, so there is no lookup
to get wrong. Both handles come back out of `ndk_context::android_context()`, which the subsection
above filled in for cpal, so this needed nothing new.

**`jni_str!` and `jni_sig!` rather than string literals.** `jni` 0.22 takes a `JNIStr` for a method
name and a `MethodSignature` for its type. Both are built at compile time, so a `&str` does not
compile. The helper therefore takes the name as a `&JNIStr` and the two callers pass the macro.

**The listener stores one integer.** It arrives on Android's main thread, which is the thread an ANR
is measured on. So it does what SDL's visibility watch does and leaves the work to `Machine::poll`.
`Java_com_rrgmc_karaokemachine_AudioFocus_onAudioFocusChange` is the exported symbol, name-mangled
the way `km-remote-android` exports its six.

### A GPU texture leak, and it was on every platform

Something killed the app about ninety seconds in. It looked like Samsung's background killer, then
like the 148 MB bank plus a 174 MB debug library. `dumpsys meminfo` settled it: **2.6 GB of graphics
memory, climbing**, at `oom_score_adj 0`. That is *foreground*, so this was the low-memory killer.

The `unsafe_textures` feature is on. It lets the app hold wallpaper textures without tying them to
the creator's lifetime. **The price is that `Texture` has no `Drop`.** `draw_text` makes one texture
per string per frame, two when outlined, with nothing calling `destroy`. Every string drawn, sixty
times a second, leaked.

**Nothing failed, which is why it survived**: the frames are correct and the process merely grows. A
desktop session ends before it matters; Android's low-memory killer does not wait.

| | before | after |
|---|---|---|
| graphics memory | 2.6 GB, climbing | **~60–73 MB, flat** |
| total RSS | 3.7 GB | **372 MB** |

**This was a bug on every platform.** Android was simply the first place with a memory limit low
enough to notice.

A release build is also the sensible default for a device. The library goes from 174 MB to 10.5 MB.

### Every digit arrived twice

A tap on the on-screen keypad entered its digit twice, so no song number could be typed.

**SDL delivers both events for one tap.** `SDL_HINT_TOUCH_MOUSE_EVENTS` defaults on. So a finger
produces a `FingerDown` *and* a synthetic `MouseButtonDown` at the same point. The render loop
handled those in two arms, each calling `press`. **The two arms exist for a good reason:** mouse
coordinates are window pixels, and touch coordinates are normalized. **That is exactly why the
duplication was not obvious from reading them.**

The answer is to ignore the synthetic event rather than turn the hint off. A mouse event invented
from a touch carries a distinguishing id, and so does the reverse. The hint stays, because mouse
synthesis lets a touch drive anything that only understands a mouse. The fault is handling one press
through two paths.

### Samsung kills it after three seconds in the background

This is Samsung's own background management rather than a crash: there is no tombstone. A machine
meant to keep playing while the screen is elsewhere would need a foreground service. An appliance on
a television does not have the problem at all.

## Video on Android

The pinned LGPL ffmpeg is cross-built for both ABIs. **`km-video` itself needed no change at all.**
It is one file with no `cfg` and no platform assumption, so all of this is build plumbing.

**Shared libraries, not static, and the reason is the license.** The whole ffmpeg posture rests on
one fact: *shipping the shared libraries beside the binary is compliant under LGPL and would not be
under GPL*. Static linking would move the APK to the relinking obligations. It would make Android
the one carrier with a different story.

Five things cost time, and **four of the five present as something other than what they are**:

1. **`TMPDIR` must be a POSIX path.** ffmpeg's configure does not merely write there; it *executes*
   there. On Windows the variable holds a backslashed path, and configure's own shell eats the
   backslashes. The failure names the right directory with every backslash missing, which reads
   like a permissions problem.
2. **`MSYS2_ARG_CONV_EXCL` must NOT be set**, which is the opposite of what this repository does
   everywhere else. It is set for `docker run`, because those are container paths Windows must keep
   its hands off. Here the NDK's clang is a **native Windows** program. So MSYS's argument conversion
   is exactly what turns a POSIX `--sysroot` into something clang can open. With it set, configure
   dies at its first probe. Yet the same clang, run by hand from the same shell, compiles perfectly,
   because by hand nobody disabled the conversion.
3. **bindgen cannot find `stddef.h`, and `cargo-ndk` is why.** That header is a compiler builtin from
   clang's resource directory, and bindgen locates it by asking the executable named by `CLANG_PATH`.
   cargo-ndk sets it **without the `.exe`**, so clang-sys rejects it and adds no builtin include
   path. **The first attempt at a fix was worse than the problem.** Correcting `CLANG_PATH` fixes
   Android and breaks every *host* build, because cargo's `[env]` is global. The bindgen variables
   are **target-suffixed**, so a host build never reads one.

   `force = true` is required either way, because cargo-ndk sets the same variable on the cargo it
   spawns.
4. **ffmpeg installs every `hwcontext_*.h` whatever was configured**, and `ffmpeg-sys-next` gates each
   hardware API on `__has_include`. For Vulkan it then parses a stub that pins a struct size at the
   64-bit layout. So on `armeabi-v7a` bindgen stops with a Vulkan error, **in a build that asked for
   no Vulkan, on the only ABI a television loads.** The script prunes the headers for hardware this
   build cannot back. That makes `__has_include` tell the truth rather than working around it.
5. **One `cargo ndk` per ABI.** `FFMPEG_DIR` is a single plain variable with no per-target spelling.
   armv7 is built **first**, so a 32-bit surprise arrives before several minutes of arm64 work.

**The decoder set is narrowed on Android, and it is not the size optimization it looks like.** With
the full native set, libavcodec's link line is over 25 KB of object paths. Windows caps a command
line at 32,767 characters, so the line is truncated mid-argument. clang then reports a missing
source file that is really a filename with three characters cut off. **The narrowed build is the
one that works.** That it is 2.2 MB per ABI rather than 10–13 is a consequence.

**Two Android packaging rules bit, and they are the same rule.** Gradle packages only `*.so` out of
`jniLibs`. So Gradle silently drops a versioned soname, and the app dies at launch with nothing
wrong in the build log. The license text staged beside the libraries it covers went the same way, so
it ships in the **assets** tree instead. **Counting what arrived in the APK, rather than what was
copied towards it, caught that. It is worth doing once per carrier.**

**What is not done.** Every song kind has played on a Streamer, 1080p30 included, and the decoder
does not starve there; see [the session below](#all-three-kinds-on-the-television-and-the-threaded-decoder).
`h264_mediacodec` is still compiled in and never selected. So switching remains a change in Rust
rather than another ffmpeg build, and nothing measured since argues for it. What is genuinely left
is nothing: the physical remote has now been pressed. Every earlier D-pad and BACK result on this
device has been an injected key event.

## All three kinds on the television, and the threaded decoder

A second device session, on the same Streamer, against a package built for it: two MIDI songs, the
1080p30 video, and six MP3+G pairs.

**The frame-threading fix works on `armeabi-v7a`, and this is where it most needed proving.**
[`video.md`](video.md) settled it on the appliance and a 24-core desktop. The 80%-of-one-core figure
that made the fault inevitable was taken *here*, on four Cortex-A55s. The machine now prints
`the video decoder's threading threads=5 kind=Frame`. `/proc/<pid>/task/*/comm` carries
`av:h264:df0` through `df4`: the five workers, visible on the device rather than inferred.

**The file that always starved played clean.** The song is 1920x1080 H.264 High 29.97 fps at
926 kbps. On three of three single-threaded runs it read `starved_ms` 871, 452 and 260. Now it played
all 273 seconds and reported **nothing at all**. That is the pass, because a healthy song is silent
by design and only a fault warns. The machine returned to idle by itself, `queue_len` 0.

| | |
|---|---|
| Presentation, 1080p30 playing | **59.5 fps**, draw 2.6/10.1 ms, present 14.1/39.6 ms |
| Presentation, MP3+G playing | **59.9 fps**, draw **0.3**/1.7 ms |
| Presentation, idle | 60.0 fps, draw 3.3/4.8 ms |

**An MP3+G pair costs almost nothing to draw.** It takes 0.3 ms against the MIDI path's few. The
graphics plane is a texture upload and no text is shaped at all, which is the opposite of the lyric
ladder. The CD+G plane letterboxes 4:3 inside the 16:9 panel and the tile wipe tracks the words.

**Memory is much tighter than the earlier row suggests, and the bank is why.** That 313 MB was
measured with the 148 MB FluidR3 override. This session ran the machine's own 274 MB override bank:

| | idle | playing 1080p |
|---|---|---|
| TOTAL RSS (`dumpsys meminfo`) | 442 MB | **552 MB** |
| Graphics (`GL mtrack`) | 52 MB | — |
| `VmSize` of a ~3 GB 32-bit space | 1.94 GB | **2.06 GB** |

**`VmSize` is the number to watch, not RSS.** Two thirds of the address space is committed at idle,
and it passes 2 GB during a video. So a bigger bank eats address space rather than memory, and this
device cannot be given more address space. 52 MB of graphics against the 43 MB on record is the text
cache earning its keep at a cost.

**The owner's page runs here, and pushing a real package at it found the defect it exists to avoid.**
`/admin/` serves and names the machine in its `<h1>`. It lists all four packages with Move and
Remove, and its `Add songs` form works. But an 85 MB package stopped at 2,162,688 bytes, because the
admin router set no `DefaultBodyLimit` and inherited axum's 2 MB.

With that fixed, the same file uploads whole in 7.7 seconds and answers
`updated "tvtest" · 9 songs`. See
[`api.md`](api.md#there-are-two-caps-per-route-and-sharing-one-of-them-is-not-sharing-the-other).
This is the platform that most needed it, since a television with no keyboard has no other way in.

`Move` refused while a song was playing, with the right reason (*"every song in it would be
renumbered under the queue"*). When idle, it moved 9 songs from bank 2 to 3 and back. `Remove` took
a package out cleanly. `/discover` carries the owner's chosen name.

**One Android behavior worth recognizing rather than debugging.** Launch something over the machine
and Android freezes it: `ActivityManager: freezing <pid> com.rrgmc.karaokemachine`. A frozen process
serves no HTTP. So the API stops answering, while the display looks fine on a TV that is showing
something else. It is the cached-process freezer doing its job, not a fault, and foregrounding the
activity brings the API straight back. Only another app in front can cause it, and on a real
appliance that does not happen.

**The frame meter's decode counters draw nothing here, and that is correct.** They appear only when
one is non-zero. On the appliance, `skipped` ran at 0.44–0.65 a second because an audio callback
there is 117 ms. So three and a half frames of a 30 fps video came due at every step. AAudio's
callback is far shorter, so nothing is ever waiting when a frame comes due. **Do not read the empty
rows as the counters being unwired on Android**: that was the first guess, and it is wrong.

## Packages come from two folders

`Paths` gained an `extra_data_dir`. It is the app's **external** files directory on Android and
`None` everywhere else. The machine scans the private folder **first**, and the order is
load-bearing. A collision resolves in favor of whatever was installed first. So scanning shared
storage first would let a dropped file displace what the machine already has, silently.

**Writing and scanning now disagree on purpose, and it is verified on the device.** A file the
machine is *handed* goes to `packages_write_dir()`. That prefers the external folder whenever
Android reports it writable. A Streamer measured it as `android external storage state state=3
writable=true`. Uploading a package through the owner's page confirmed it: the package arrived in
`/storage/emulated/0/Android/data/<package>/files/packages/`. Scanning order is unchanged.

The asymmetry is the point. A package reaches tens of gigabytes and internal storage is the smaller
volume. Meanwhile nothing dropped onto shared storage may displace what is already installed.

**The application id keys both folders, so the headset flavour has a pair of its own.** A Quest
holding both APKs holds two libraries, and a package pushed to one is absent from the other. That is
the cost of the `.quest` suffix, and it is what lets somebody compare the two screens with one
headset.

**The one case where that combination surprises is an upgraded machine.** It is worth knowing before
somebody diagnoses it as an upload that did nothing. A build older than this wrote handed-in packages
to the **private** folder. So an install that has run one may hold a private copy of an id whose
newer upload has just landed externally. The private copy goes on winning the scan, because the
machine scans it first.

Nothing is lost and nothing is wrong, but the page says *updated* while the catalog keeps the older
bytes. It cannot arise on an install that has only ever run this build, since nothing then puts a
package in the private folder at all.

**Uninstall asks whether the file is in *any* scanned root**, not in the one. Without that, the list
would not remember an uninstalled package on shared storage, and the next start would put it
straight back. That is the exact failure the list exists to prevent, reintroduced by a second
folder.

`getExternalFilesDir` is the right one of three candidates. All-files access needs a prompt that is
hostile on a television, and some builds refuse it outright. The Storage Access Framework returns a
content URI that every path in the application would have to learn about. And a picker is the wrong
shape for a box that starts itself under a screen.

This one needs **no permission on any API level** and is an ordinary path. It takes a plain
`adb push` with no `run-as`. It can return null when external storage is unmounted. That is an
ordinary answer meaning *one folder, not two*, never a failure.

## A package opened from a file manager

`Android 11` closed `/Android/data/<id>/files/` to third-party file managers and hides it over MTP.
So the folder above takes an `adb push` and nothing a person has to hand. Opening a `.kmpkg` is
therefore the route in for a device with no cable, and the decision is
[`On Android the document is a stream`](../decisions/interface.md#on-android-the-document-is-a-stream-and-it-is-the-only-route-in-without-a-cable).

**Nothing in the Rust side is specific to this.** The shell hands the machine a path through
`SDLActivity.onNativeDropFile`, which is the drop event `display.rs` already handles. So the package
travels the route `dropped::adopt` defines for every platform. It is opened for its manifest, copied
into `packages_write_dir()` under the name that manifest implies, installed, and reported in a band.
`DRAG_AND_DROP` stays false, and `ios.md` argues the same split. The constant governs whether the
empty-catalog message *offers* dragging a file onto a window, and nothing on a phone does that.

Four things in `MainActivity` that are not obvious:

- **The manifest claims `application/octet-stream` as well as the real type**, and that is the claim
  that fires. Android has no MIME entry for `.kmpkg` and no way for an application to add one. So
  the Downloads provider, the Files app and a browser's download notification all describe a package
  as an unknown binary. There are two `intent-filter` elements rather than one. Within a single
  filter, the schemes, hosts, paths and types merge into sets that match in any combination. So the
  broad type would widen the narrow one.
- **The launch intent is replaced with a bare `MAIN` before `super.onCreate`.** SDL's activity reads
  `getIntent().getData().getPath()` itself and sends it as a drop. That is right for a `file:` URI
  and useless for a `content:` one, whose path is a provider id such as
  `/document/primary:Download/vol1.kmpkg`. So the machine would answer a successful open with *not a
  package*, about a file that is not there. This way leaves the vendored `org.libsdl.app` Java stock,
  which an SDL upgrade wants. A patch to SDL's own `onCreate` would not survive one.
- **`onNewIntent` exists because `singleInstance` does.** SDL overrides it nowhere. So a second
  package opened against a running machine would come to the front and do nothing. No warm hand-off
  is needed here, unlike the desktop's: there is one process and it already holds the catalog.
- **The stream is staged into `files/incoming/`, a sibling of `packages/` and not a child.** It is
  the same volume, so the copy `place` makes afterwards does not cross one. It is not the cache
  directory, which the system reclaims under pressure, and a multi-gigabyte copy is the worst moment
  for that. And it is not inside `packages/`, where a half-written file would be in a folder the
  machine scans. The `.part` name and the rename are the rule `place` already follows.

  The shell sweeps that folder before each stage. The machine never deletes a dropped file, and a
  copy it did not make is not its to remove either.

**A name that is not `.kmpkg` is refused before anything is read.** The shell hands over the
un-staged path, and `DropInstaller::submit` checks the extension before it touches disk. So the
refusal costs nothing. It is the ordinary case rather than the odd one, since the broad filter is
offered every unknown binary on the device.

**What a large package looks like is silence.** The band only appears once the drop is posted, which
is after the copy. So a package of several gigabytes shows nothing while it stages. It is worth a
progress report if it becomes a complaint; it is not one yet.
