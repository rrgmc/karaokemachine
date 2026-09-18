# Android

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

## The build

**Every crate cross-compiles and links for `aarch64-linux-android` and `armv7-linux-androideabi`.**
Six of the seven needed **no changes at all**, which is the pure-Rust bet paying off exactly as
planned; `rusqlite`'s bundled SQLite compiled with the NDK's clang without comment.

Four things had to be fixed, and each is a comment in `tools/port/machine/android/build.sh`:

1. **CMake defaulted to the Visual Studio generator**, so building SDL3 for Android tried to compile a
   `.vcxproj` with MSBuild. Needs `CMAKE_GENERATOR=Ninja`.
2. **The `cmake` crate guessed `CMAKE_SYSTEM_PROCESSOR=arm64`**, which CMake's Android support rejects
   — it wants `aarch64`. Handing it the NDK's own toolchain file skips the guessing.
3. **`unable to find library -laaudio`.** AAudio is API 26+ and `cargo-ndk` defaults to 21. (The flag
   is `-P`; lowercase `-p` reaches cargo as `--package` and fails with "unknown package: 26".)
4. **`symbol __aeabi_memcpy8@@LIBC_N has undefined version LIBC_N`**, on 32-bit ARM only — see below.
   It lives in `.cargo/config.toml` rather than the build script, because it has to apply to any cargo
   invocation for that target.

A fifth, **a wall of `undefined symbol: operator new`**, was fixed and then unfixed, and the second
answer is the right one. The NDK does not link libc++ implicitly, unlike every desktop platform, so
`km-display/build.rs` was taught to link `c++_static` and add the NDK sysroot to the search path.
**That build script was later deleted outright**, and the reasoning is in *The first launch crashed*
below: SDL is built **shared** on Android, so its C++ runtime lives inside `libSDL3.so`, private and
unexported, and the library imports no C++ symbols at all — so it needs no C++ runtime and no extra
search path.

**SDL is linked shared on Android and static everywhere else, and the split has to be declared in
*both* `km-display` and `km-app`.** Cargo features are additive: one crate asking for a static SDL puts
a static SDL in the build regardless of what the other asks for, and getting it wrong in one silently
undid the split next door — the library came out with no `libSDL3.so` dependency at all. Verified by
symbol table rather than by assumption.

### A false alarm that looks exactly like the real one

`libSDL3.so` contains all 68 `Java_org_libsdl_app_*` functions and exports **none** of them, which
looks exactly like the bug that would make an APK die with `UnsatisfiedLinkError`.

It is not a bug. **SDL registers its native methods dynamically** from `JNI_OnLoad`, and `JNI_OnLoad`
*is* exported — which is why its version script lists only `SDL_*` and that one symbol, and why SDL
treats the version script as **required** on Android. Widening the export list would have been a
change away from what SDL intends.

Recorded because the symbol tables read like a fault and the correct behavior is not obvious from
them: anyone checking this the same way will reach the same wrong conclusion. **The check that settles
it is `JNI_OnLoad` being present in `.dynsym`, not the absence of `Java_*`.**

### The armv7 `cdylib` links — one declared version node

This mattered more than its size suggests: both Google TV devices run a 32-bit OS and load
`armeabi-v7a` and nothing else, so an arm64-only build is a build no television can run.

The cause, read off the files rather than inferred: Rust's prebuilt `compiler_builtins` defines the
ARM EABI memory helpers as **weak and unversioned**, while bionic exports the same twelve and tags
them `LIBC_N` from API 24 on. lld takes the version off the NDK stub, carries it onto the definition
that wins the link (ours), and then refuses because the *output* declares no such node.

**The fix is to declare the node and put nothing in it.** lld accepts several version scripts and
merges them, so rustc's generated one still does its job. The only trace is one inert
`.gnu.version_d` entry.

**Two plausible fixes are wrong, and both cost time.** `-Wl,--undefined-version` — what most search
results suggest — has no effect: it governs whether a version script may *name* an undefined symbol,
not whether a version node exists, confirmed inert with the flag passed last where it would win any
ordering race. And changing API level is a dead end: `LIBC_N` is absent below 24, but AAudio starts at
26.

## The APK

`ports/machine/android/` holds the Gradle project. **No `externalNativeBuild` block anywhere, on
purpose** — cargo builds the native side and Gradle only packages it, so one build system owns the
Rust and one owns the APK rather than Gradle driving CMake driving cargo. Staging copies the libraries
into `jniLibs/` and the build fails with a readable message if they are not there; the alternative is
an APK that installs and dies at launch.

SDL's Java is copied verbatim from the vendored template and is not to be edited.

**It needs a JDK 17 or 21.** Gradle 8.12 will not run on JDK 25 — `Unsupported class file major
version 69` while parsing the build script, its bundled Groovy predating that format. **A Gradle
*toolchain* does not help**: the failure is in the daemon JVM that compiles the build script, and a
toolchain only governs what compiles the app.

Three faults, all of which produce errors naming the wrong thing:

1. **`jniLibs.useLegacyPackaging` inside a `buildTypes` block.** Not a build-type property — Gradle
   reports `Could not get unknown property 'jniLibs' for BuildType`, which reads like a plugin version
   problem. Removed rather than relocated: the modern default already stores native libraries
   uncompressed and maps them from the APK, and the comment that justified the setting had the flag's
   meaning backwards — legacy packaging *compresses* and extracts at install, keeping two copies of a
   very large library on the device.
2. **A `--` inside an XML comment in the manifest.** Illegal in XML, and the merger reports only
   `Error parsing AndroidManifest.xml` with no line number unless you ask for `--stacktrace`.
3. **`CHANGE_WIFI_MULTICAST_STATE` implies `android.hardware.wifi` as *required*.** Nothing asked for
   it. Left alone it filters the app off any device with no Wi-Fi radio — an Ethernet-only television
   box being exactly the case worth keeping. **Invisible until there was an APK to inspect.**

## Assets ship in the APK

**Android assets are not files.** They live inside the APK and only the asset API can read them —
while everything that consumes them wants a *path*. So they are unpacked once, on first run, into the
directory the asset resolver already points at, and nothing downstream changes.

- **SDL does the reading, so there is no JNI and no `unsafe`.** `SDL_IOFromFile` on Android tries
  internal storage for a relative path and then falls back to the APK's asset system, and the `sdl3`
  crate wraps it as something implementing `io::Read` — so unpacking is `io::copy`.
- **A `MANIFEST` says what is in there**, because SDL can open an asset by name but cannot *list* an
  asset directory. Sizes are included so editing a file without adding or removing one still triggers
  a re-unpack, and each unpacked file's length is checked — a short read is otherwise silent and
  surfaces much later as a truncated SoundFont.
- **The manifest is the completion marker**, written **last**, so a failure part-way through leaves
  the directory looking unfinished and the next launch retries rather than trusting it.
- **An asset that leaves the manifest is deleted**, which for three releases it was not. App-private
  storage survives an upgrade, so unpacking that only ever adds and overwrites leaves every file a
  past build shipped on the device for ever. That is invisible until something *lists* a directory
  rather than naming what it wants — and the display lists the wallpapers folder, so replacing four
  gradient PNGs with a zip of seven photographs left eleven wallpapers on every device that had run
  the older build, four of them the ones the change was meant to retire. Found on a Google TV
  Streamer. The old manifest is the record of what to remove, and only paths it lists are touched,
  so a file somebody put there by hand is left alone; a removal that fails is logged rather than
  fatal, since withholding the manifest to force a retry would re-copy thirty megabytes on every
  launch for ever.

The staging script warns when no `.sf2` is present — the machine would come up on its test tone, which
sounds broken to anyone who does not know it is the documented fallback — and notes when the total
passes 64 MiB, since the device pays roughly twice because unpacking duplicates them.

## It runs on hardware

Deployed to a **Galaxy S23** (arm64) and a **Google TV Streamer** (armv7). The machine starts, renders
and stays up — and **AAudio opens *and starts***, which retires the load-bearing assumption of the
whole plan: cpal works on Android.

**The Streamer has no 64-bit ABI at all.** Not a 64-bit chip running a 32-bit userspace — the property
is empty. An arm64-only APK could not have run under any circumstances, so the `LIBC_N` version-node
fix was not insurance; it was the difference between having a product on this device and not.

**The remote's D-pad arrives as keyboard events**, and SDL never added a joystick device, so the worry
from the research note does not materialise. `required="false"` on touchscreen was equally
load-bearing: without it the app would not have installed.

| | |
|---|---|
| Display | 1920×1080 on the television, 2340×1080 on the phone |
| RSS with the 148 MB override bank | **313 MB**, graphics flat at 43 MB — comfortable in a 32-bit address space |
| BACK | stops a playing song, then exits from idle |

**Confirmed with a thumb on the actual remote.** Arrows move the highlight, OK activates it (`PAUSE` fired, and the API agreed), BACK stops a playing song without
leaving, and BACK from idle leaves cleanly — `shutting down`, `the machine stopped normally`, the
activity closed and the launcher back in front. **No joystick device appeared**, and `dumpsys input`
says why: the remote is `Google TV Remote Keyboard`, `Classes: KEYBOARD | DPAD | MIC | BATTERY |
EXTERNAL`, with no `GAMEPAD` — the two GAMEPAD-classed devices on the box are `virtual-search` and
`virtual-remote`. Two things only a real press could have found: the first D-pad press after the
transport strip fades is swallowed waking it (`display.rs`, where the reveal-then-act rule is
stated), and the idle number pad had to be switched back on for every Android — see [`The on-screen
number pad`](../decisions/interface.md#the-on-screen-number-pad).

### The first launch crashed, and the cause was ours

`SIGSEGV`, null read, during `dlopen` — before `SDL_main`. The library's `.init_array` held
**statically linked bionic**: the allocator, the trace helper and the DNS resolver, whose constructors
ran against libc state that already belonged to the real `libc.so`. `DT_NEEDED` listed SDL3, SDL3_ttf,
aaudio and log — and **no `libc.so` at all**.

The cause was the build script above, which added the NDK sysroot's `usr/lib/<triple>` to the link
search path so rustc could find `libc++_static.a`. **That directory also holds `libc.a`, `libm.a` and
`libdl.a`, while the shared stubs live in the per-API-level subdirectory below it** — so `-lc`, `-ldl`
and `-lm` all resolved to static archives.

Two things worth keeping. **A missing `DT_NEEDED` entry is a symptom**, and reading that list against a
correct Android library — rather than against the other ABI, which is how it was first checked and why
it was missed — would have found it before the device did. And **a `-L` on an NDK sysroot is never
harmless**: link the archive by absolute path instead.

### Logs had to be made visible first

Nothing the machine logs reaches anywhere by default: the default subscriber writes to a stdout the
zygote has pointed at `/dev/null`. **A device that fails silently cannot be debugged.**
`km-androidlog` routes events through `__android_log_write` — `liblog` is already linked for SDL, so
this costs no dependency. It compiles under `cfg(test)` on every platform, not just Android, so its
line buffering is actually exercised; that immediately caught a bug where every event emitted a
trailing blank line.

### Audio: the context nobody published

AAudio opens and closes 3 ms later, and a ten-second timeout gets the blame. `Disconnected` and
`Timeout` are two answers from the receive and must be two match arms: folded into one, the message
names the wrong cause and sends the reader ten seconds in the wrong direction.

Finding the panic needed a **panic hook that logs**, since a panic's report goes to the same discarded
stderr. With that in place: `android context was not initialized`.

cpal's Android backend reaches Java through `ndk_context`, which reads a JavaVM and an Activity from a
global that **something else is expected to have filled in** — normally `ndk-glue` or
`android-activity`, neither of which we use. The activity is SDL's, and **SDL keeps its JNI handles to
itself**, so the global stayed empty and the first caller panicked.

- **The VM comes from our own `JNI_OnLoad`.** Android calls that on every library load and hands over
  the `JavaVM*` directly: no JNI call, nothing to borrow, and it runs before `SDL_main`. The
  alternative — asking SDL for a `JNIEnv*` and calling `GetJavaVM` through the function table — is
  worse in this dependency set.
- **The Activity comes from `SDL_GetAndroidActivity`**, which returns a *local* reference. It is
  promoted to a global one and deliberately never released, because `ndk-context` keeps the pointer
  for the life of the process and cpal dereferences it on the audio thread.
- **Both crates are pinned with `=` to exactly what cpal resolves.** That is the point rather than
  caution: `ndk-context` keeps the context in a private static, so a different version would be a
  different crate, a different static, and an initialization cpal never sees.

### A GPU texture leak, and it was on every platform

The app was being killed about ninety seconds in. It looked like Samsung's background killer, then
like the 148 MB bank plus a 174 MB debug library. `dumpsys meminfo` settled it: **2.6 GB of graphics
memory, climbing**, at `oom_score_adj 0` — which is *foreground*, so this was the low-memory killer.

The `unsafe_textures` feature is on, which is what lets the app hold wallpaper textures without tying
them to the creator's lifetime. **The price is that `Texture` has no `Drop`** — and `draw_text` makes
one texture per string per frame, two when outlined, with nothing calling `destroy`. Every string
drawn, sixty times a second, leaked.

**Nothing failed, which is why it survived**: the frames are correct and the process merely grows. A
desktop session ends before it matters; Android's low-memory killer does not wait.

| | before | after |
|---|---|---|
| graphics memory | 2.6 GB, climbing | **~60–73 MB, flat** |
| total RSS | 3.7 GB | **372 MB** |

**This was a bug on every platform.** Android was simply the first place with a memory limit low
enough to notice.

A release build is also the sensible default for a device — the library goes from 174 MB to 10.5 MB.

### Every digit arrived twice

A tap on the on-screen keypad entered its digit twice, so no song number could be typed.

**SDL delivers both events for one tap.** `SDL_HINT_TOUCH_MOUSE_EVENTS` defaults on, so a finger
produces a `FingerDown` *and* a synthetic `MouseButtonDown` at the same point — and the render loop
handled those in two arms, each calling `press`. **The two arms exist for a good reason** (mouse
coordinates are window pixels, touch coordinates are normalized) **which is exactly why the
duplication was not obvious from reading them.**

The answer is to ignore the synthetic event rather than turn the hint off: a mouse event invented
from a touch carries a distinguishing id, and so does the reverse. The hint is left alone, because
mouse synthesis is what lets a touch drive anything that only understands a mouse; the fault is
handling one press through two paths.

### Samsung kills it after three seconds in the background

This is Samsung's own background management rather than a crash: there is no tombstone. A machine
meant to keep playing while the screen is elsewhere would need a foreground service — an appliance on
a television does not have the problem at all.

## Video on Android

The pinned LGPL ffmpeg is cross-built for both ABIs. **`km-video` itself needed no change at all** —
it is one file with no `cfg` and no platform assumption, so all of this is build plumbing.

**Shared libraries, not static, and the reason is the license.** The whole ffmpeg posture rests on
*shipping the shared libraries beside the binary is compliant under LGPL and would not be under GPL*;
static linking would move the APK to the relinking obligations and make Android the one carrier with a
different story.

Five things cost time, and **four of the five present as something other than what they are**:

1. **`TMPDIR` must be a POSIX path.** ffmpeg's configure does not merely write there, it *executes*
   there, and on Windows the variable holds a backslashed path its own shell eats. The failure names
   the right directory with every backslash missing, which reads like a permissions problem.
2. **`MSYS2_ARG_CONV_EXCL` must NOT be set**, which is the opposite of what this repository does
   everywhere else. It is set for `docker run` because those are container paths Windows must keep its
   hands off. Here the NDK's clang is a **native Windows** program, so MSYS's argument conversion is
   exactly what turns a POSIX `--sysroot` into something clang can open. With it set, configure dies
   at its first probe — while the same clang run by hand from the same shell compiles perfectly,
   because by hand nobody disabled the conversion.
3. **bindgen cannot find `stddef.h`, and `cargo-ndk` is why.** That header is a compiler builtin from
   clang's resource directory, which bindgen locates by asking the executable named by `CLANG_PATH` —
   and cargo-ndk sets it **without the `.exe`**, so clang-sys rejects it and no builtin include path
   is added. **The first attempt at a fix was worse than the problem**: correcting `CLANG_PATH` fixes
   Android and breaks every *host* build, because cargo's `[env]` is global. The bindgen variables are
   **target-suffixed**, so a host build never reads one; `force = true` is required either way,
   because cargo-ndk sets the same variable on the cargo it spawns.
4. **ffmpeg installs every `hwcontext_*.h` whatever was configured**, and `ffmpeg-sys-next` gates each
   hardware API on `__has_include`. For Vulkan it then parses a stub pinning a struct size at the
   64-bit layout, so on `armeabi-v7a` bindgen stops with a Vulkan error, **in a build that asked for no
   Vulkan, on the only ABI a television loads.** The script prunes the headers for hardware this build
   cannot back, which makes `__has_include` tell the truth rather than working around it.
5. **One `cargo ndk` per ABI.** `FFMPEG_DIR` is a single plain variable with no per-target spelling.
   armv7 is built **first**, so a 32-bit surprise arrives before several minutes of arm64 work.

**The decoder set is narrowed on Android, and it is not the size optimization it looks like.** With
the full native set, libavcodec's link line is over 25 KB of object paths, Windows caps a command line
at 32,767 characters, and the line is truncated mid-argument — clang then reports a missing source
file that is really a filename with three characters cut off. **The narrowed build is the one that
works**; that it is 2.2 MB per ABI rather than 10–13 is a consequence.

**Two Android packaging rules bit, and they are the same rule.** Gradle packages only `*.so` out of
`jniLibs`, so a versioned soname is dropped silently and the app dies at launch with nothing wrong in
the build log, and the license text staged beside the libraries it covers went the same way — it ships
in the **assets** tree instead. **That was caught by counting what arrived in the APK rather than what
was copied towards it, which is worth doing once per carrier.**

**What is not done.** Every song kind has played on a Streamer, 1080p30 included, and the decoder
does not starve there — see [the session below](#all-three-kinds-on-the-television-and-the-threaded-decoder).
`h264_mediacodec` is still compiled in and never selected, so switching remains a change in Rust
rather than another ffmpeg build, and nothing measured since argues for it. What is genuinely left
is nothing: the physical remote has now been pressed. Every earlier D-pad and BACK result on this device
has been an injected key event.

## All three kinds on the television, and the threaded decoder

A second device session, on the same Streamer, against a package built for it: two MIDI songs, the
1080p30 video, and six MP3+G pairs.

**The frame-threading fix works on `armeabi-v7a`, and this is where it most needed proving.** The
appliance and a 24-core desktop were where [`video.md`](video.md) settled it; the 80%-of-one-core
figure that made the fault inevitable was taken *here*, on four Cortex-A55s. The machine now prints
`the video decoder's threading threads=5 kind=Frame`, and `/proc/<pid>/task/*/comm` carries
`av:h264:df0` through `df4` — the five workers, visible on the device rather than inferred.

**The file that always starved played clean.** The same 1920x1080 H.264 High 29.97 fps song at
926 kbps that read `starved_ms` 871, 452 and 260 on three of three single-threaded runs played all
273 seconds and reported **nothing at all** — which is the pass, because a healthy song is silent by
design and only a fault warns. The machine returned to idle by itself, `queue_len` 0.

| | |
|---|---|
| Presentation, 1080p30 playing | **59.5 fps**, draw 2.6/10.1 ms, present 14.1/39.6 ms |
| Presentation, MP3+G playing | **59.9 fps**, draw **0.3**/1.7 ms |
| Presentation, idle | 60.0 fps, draw 3.3/4.8 ms |

**An MP3+G pair costs almost nothing to draw.** 0.3 ms against the MIDI path's few — the graphics
plane is a texture upload and no text is shaped at all, which is the opposite of the lyric ladder.
The CD+G plane letterboxes 4:3 inside the 16:9 panel and the tile wipe tracks the words.

**Memory is much tighter than the earlier row suggests, and the bank is why.** That 313 MB was
measured with the 148 MB FluidR3 override; this session ran the machine's own 274 MB override bank:

| | idle | playing 1080p |
|---|---|---|
| TOTAL RSS (`dumpsys meminfo`) | 442 MB | **552 MB** |
| Graphics (`GL mtrack`) | 52 MB | — |
| `VmSize` of a ~3 GB 32-bit space | 1.94 GB | **2.06 GB** |

**`VmSize` is the number to watch, not RSS.** Two thirds of the address space is committed at idle
and it passes 2 GB during a video, so the headroom a bigger bank eats is address space rather than
memory — which is the resource this device cannot be given more of. 52 MB of graphics against the
43 MB on record is the text cache earning its keep at a cost.

**The owner's page runs here, and pushing a real package at it found the defect it exists to avoid.**
`/admin/` serves, names the machine in its `<h1>`, lists all four packages with Move and Remove, and
its `Add songs` form works — but an 85 MB package stopped at 2,162,688 bytes, because the admin
router set no `DefaultBodyLimit` and inherited axum's 2 MB. Fixed; the same file now uploads whole in
7.7 seconds and answers `updated "tvtest" · 9 songs`. See
[`api.md`](api.md#there-are-two-caps-per-route-and-sharing-one-of-them-is-not-sharing-the-other) —
this is the platform that most needed it, since a television with no keyboard has no other way in.

`Move` refused while a song was playing, with the right reason (*"every song in it would be
renumbered under the queue"*), and moved 9 songs from bank 2 to 3 and back when idle. `Remove` took
a package out cleanly. `/discover` carries the owner's chosen name.

**One Android behavior worth recognizing rather than debugging.** Launch something over the machine
and Android freezes it — `ActivityManager: freezing <pid> com.rrgmc.karaokemachine` — and a
frozen process serves no HTTP, so the API stops answering while the display looks fine on a TV that
is showing something else. It is the cached-process freezer doing its job, not a fault, and
foregrounding the activity brings the API straight back. Only reachable by putting another app in
front, which on a real appliance does not happen.

**The frame meter's decode counters draw nothing here, and that is correct.** They appear only when
one is non-zero, and `skipped` ran at 0.44–0.65 a second on the appliance because an audio callback
there is 117 ms, so three and a half frames of a 30 fps video came due at every step. AAudio's
callback is far shorter, so nothing is ever waiting when a frame comes due. **Do not read the empty
rows as the counters being unwired on Android** — that was the first guess and it is wrong.

## Packages come from two folders

`Paths` gained an `extra_data_dir`, which is the app's **external** files directory on Android and
`None` everywhere else. The private folder is scanned **first**, and the order is load-bearing — a
collision resolves in favor of whatever was installed first, so scanning shared storage first would
let a dropped file displace what the machine already has, silently.

**Writing and scanning now disagree on purpose, and it is verified on the device.** A file the
machine is *handed* goes to `packages_write_dir()`, which prefers the external folder whenever
Android reports it writable — measured on a Streamer as `android external storage state state=3
writable=true`, and confirmed by uploading a package through the owner's page and finding it in
`/storage/emulated/0/Android/data/<package>/files/packages/`. Scanning order is unchanged. The
asymmetry is the point: a package reaches tens of gigabytes and internal storage is the smaller
volume, while nothing dropped onto shared storage may displace what is already installed.

**The one case where that combination surprises is an upgraded machine**, and it is worth knowing
before it is diagnosed as an upload that did nothing. A build older than this wrote handed-in
packages to the **private** folder, so an install that has run one may hold a private copy of an id
whose newer upload has just landed externally — and the private copy goes on winning the scan,
because it is scanned first. Nothing is lost and nothing is wrong, but the page says *updated* while
the catalog keeps the older bytes. It cannot arise on an install that has only ever run this
build, since nothing then puts a package in the private folder at all.

**Uninstall asks whether the file is in *any* scanned root**, not in the one: without that,
uninstalling a package on shared storage would not be remembered and the next start would put it
straight back — the exact failure that list exists to prevent, reintroduced by there being a second
folder.

`getExternalFilesDir` is the right one of three candidates. All-files access needs a prompt that is
hostile on a television and refused outright on some builds; the Storage Access Framework returns a
content URI every path in the application would have to learn about, and a picker is the wrong shape
for a box that starts itself under a screen. This one needs **no permission on any API level**, is an
ordinary path, and takes a plain `adb push` with no `run-as`. It can return null when external storage
is unmounted, and that is an ordinary answer meaning *one folder, not two*, never a failure.

## A package opened from a file manager

`Android 11` closed `/Android/data/<id>/files/` to third-party file managers and hides it over MTP,
so the folder above takes an `adb push` and nothing a person has to hand. Opening a `.kmpkg` is
therefore the route in for a device with no cable, and the decision is
[`On Android the document is a stream`](../decisions/interface.md#on-android-the-document-is-a-stream-and-it-is-the-only-route-in-without-a-cable).

**Nothing in the Rust side is specific to this.** The shell hands the machine a path through
`SDLActivity.onNativeDropFile`, which is the drop event `display.rs` already handles, so the package
travels the route `dropped::adopt` defines for every platform: opened for its manifest, copied into
`packages_write_dir()` under the name that manifest implies, installed, and reported in a band.
`DRAG_AND_DROP` stays false, because the constant governs whether the empty-catalog message *offers*
dragging a file onto a window and nothing on a phone does that — the split `ios.md` already argues.

Four things in `MainActivity` that are not obvious:

- **The manifest claims `application/octet-stream` as well as the real type**, and that is the claim
  that fires. Android has no MIME entry for `.kmpkg` and no way for an application to add one, so
  the Downloads provider, the Files app and a browser's download notification all describe a package
  as an unknown binary. Two `intent-filter` elements rather than one: within a single filter the
  schemes, hosts, paths and types merge into sets matching in any combination, so the broad type
  would widen the narrow one.
- **The launch intent is replaced with a bare `MAIN` before `super.onCreate`.** SDL's activity reads
  `getIntent().getData().getPath()` itself and sends it as a drop. That is right for a `file:` URI
  and useless for a `content:` one, whose path is a provider id such as
  `/document/primary:Download/vol1.kmpkg` — so a successful open would be answered by *not a
  package* about a file that is not there. Doing it this way leaves the vendored `org.libsdl.app`
  Java stock, which an SDL upgrade wants; patching SDL's own `onCreate` would not survive one.
- **`onNewIntent` exists because `singleInstance` does.** SDL overrides it nowhere, so a second
  package opened against a running machine would come to the front and do nothing. No warm hand-off
  is needed here, unlike the desktop's: there is one process and it already holds the catalog.
- **The stream is staged into `files/incoming/`, a sibling of `packages/` and not a child.** The
  same volume, so the copy `place` makes afterwards does not cross one; not the cache directory,
  which the system reclaims under pressure and a multi-gigabyte copy is the worst moment for that;
  and not inside `packages/`, where a half-written file would be in a folder the machine scans. The
  `.part` name and the rename are the rule `place` already follows. The shell sweeps that folder
  before each stage, because the machine never deletes a dropped file and a copy it did not make is
  not its to remove either.

**A name that is not `.kmpkg` is refused before anything is read**, by handing over the un-staged
path: `DropInstaller::submit` checks the extension before it touches disk, so the refusal costs
nothing. It is the ordinary case rather than the odd one, since the broad filter is offered every
unknown binary on the device.

**What a large package looks like is silence.** The band only appears once the drop is posted, which
is after the copy, so a package of several gigabytes shows nothing while it stages. Worth a progress
report if it becomes a complaint; it is not one yet.
