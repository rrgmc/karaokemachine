# iOS

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

The machine as an iOS application. The requirements row is
[`What the machine *is*, on iOS`](../decisions/distribution.md#what-the-machine-is-on-ios), and
[`android.md`](android.md) is the sibling to read beside this one: the two shells solve the same
problem and disagree about almost every mechanism.

## SDL builds from source for a phone

**`sdl3-sys` and `sdl3-ttf-sys` build SDL3 and SDL3_ttf from source for `aarch64-apple-ios` with no
patch and no toolchain file of ours**, which is the single answer the rest of this port rested on. A
cold build takes about a minute on an M2 Pro, and the result is what it claims to be:
`LC_BUILD_VERSION` reads platform 2 with `minos 15.0`, so the archives are iOS rather than macOS
built against an iOS SDK.

**Both come out static, and that is the arrangement iOS wants** rather than something to work around.
The workspace pins `sdl3` with `build-from-source-static`, and iOS takes that same entry through
`cfg(not(target_os = "android"))`. Android is the platform that needs a real `libSDL3.so` in the
package and therefore needs its own dependency block; a phone links the archive into the app binary,
so the desktop spelling is already right and no third block is needed.

**Two environment variables are load-bearing, and each fails in a way that names something else.**
`DEVELOPER_DIR` has to point at Xcode, because the Command Line Tools carry no iOS SDK and
`xcode-select` on a working machine may well point at them. `CMAKE_POLICY_VERSION_MINIMUM=3.5` is for
the FreeType vendored inside SDL3_ttf, which CMake 4 rejects while naming neither FreeType nor
SDL3_ttf.

## What the app binary has to be given

**The frameworks are asked of the build rather than guessed at.** `sdl3-sys` declares fifteen of
them, plus `m` and `pthread`, and Xcode supplies none of them on its own:

```
AVFoundation  AudioToolbox  CoreAudio     CoreBluetooth  CoreGraphics
CoreHaptics   CoreMedia     CoreMotion    CoreVideo      Foundation
GameController Metal        OpenGLES      QuartzCore     UIKit
```

**`cargo rustc -- --print native-static-libs` answers this only once something reaches SDL.** Against
a shim whose exports do not yet touch the machine it prints `-lSystem -lc -lm` and nothing else,
because rustc reports the requirements of the crates actually linked and an unreferenced dependency
is not one of them. The build scripts' own `cargo::rustc-link-lib` lines are the reliable source
while the shell is still being assembled, and the archive is the check that they arrived: `ar t` over
the staticlib finds no SDL member until the machine is genuinely pulled in.

Once it is, the full answer is seventeen frameworks after duplicates are dropped — the fifteen above,
`AVFAudio` from cpal's CoreAudio backend, and `CoreFoundation` under both — plus `-liconv`, which is
rusqlite's and is the one flag the offline remote's shell needs too. `-lSystem`, `-lc`, `-lm`,
`-lobjc` and `-lpthread` are in that answer and are deliberately not in `OTHER_LDFLAGS`: Xcode
supplies all five.

**SDL ends up inside our own archive, so nothing points Xcode at cargo's build directory.**
`cargo::rustc-link-lib=static=SDL3` carries the default `+bundle`, and for a `staticlib` crate type
that means rustc copies SDL's objects in: 242 members, with `SDL_Init` and `TTF_Init` defined. A
search path into a hashed `target/…/build/sdl3-sys-<hash>/out/lib` would otherwise have been the
alternative, and it is one no committed project file could spell.

## The entry point is the shim's, not the machine's

**`SDL_main` is exported by `km-machine-ios` and not by `karaokemachine`, and that is a linking
matter rather than a preference.** A `#[unsafe(no_mangle)]` symbol defined in an upstream rlib is not
guaranteed to reach a `staticlib` that never references it, and what that produces is an undefined
`_SDL_main` at the point Xcode links the app — a failure that names the shell rather than the
dependency graph. So the body lives in `karaokemachine::run_on_phone`, shared with Android, and each
platform's C symbol is defined in the crate that has to carry it.

**The shell is Objective-C, where the offline remote's is Swift with `@main`.** SDL owns
`UIApplicationMain` here: `SDL_RunApp` calls it with SDL's own delegate, which creates the window,
the Metal layer and the event pump the machine draws through. A Swift `@main` beside that would be
two applications competing for one process. SDL's headers are not reachable from the app target
either, since they stay in cargo's build directory, so `main.m` declares the one function it needs
from SDL and includes the crate's own header for the other.

**Logging installs once, from either side.** `km_machine_configure` runs before `SDL_main` and wants
to be able to complain about a directory, so both call `karaokemachine::install_logging`, which is
guarded by a `Once` — a second `.init()` on a `tracing` subscriber is a panic, not a no-op.

## It runs on hardware

**Deployed to an iPad (6th generation, A1893) on iPadOS 17.7, release build, signed with an Apple
Development certificate.** Installed and launched with `devicectl`, which needs no Xcode window; the
recipe is in `ports/remote/ios/`'s own notes and is the same for both ports, differing only in the
scheme and the bundle identifier. `build.sh --release --device-only --no-app` produces the
xcframework and the project, and one signed `xcodebuild` then produces what is installed — the
unsigned app `build.sh` compiles by default is the right thing for a check and the wrong thing to
install.

What the first run settled, in one launch:

| | |
|---|---|
| The display | Metal, vsync on, 2048×1536 at density 2.0 |
| The two directories | resolved inside the container, and `settings.json` written |
| The bank | 31 MB GeneralUser GS opened from the bundle at 44.1 kHz |
| ffmpeg | *video songs can be played by this build* — the four embedded frameworks resolved through `@rpath` at launch |
| The API | listening, and answering `/api/v1/discover` and `/` from another machine on the network |
| mDNS | no advertisement, as intended |

**The release build is the one to deploy, and not only for speed**: 67 MB against 179, from a 35 MB
staticlib against 556.

### A catalog, and a song playing

**155 songs installed from a package copied into `Documents/packages`, and a MIDI song played.**
`devicectl device copy to --domain-type appDataContainer` is what puts one there without the Files
app, which makes the whole route testable from a terminal. The machine found it at the next start,
installed it in place rather than copying it, and served it: `GET /api/v1/songs` answered with song
numbers, suitability and lyric previews from another machine on the network.

Playing it is the part that exercises the platform rather than the catalog. Queued and started over
the API, `/api/v1/state` reported `playing` with the position advancing in real time, which is the
synthesizer rendering through cpal into the session `main.m` configured.

**A video song plays, which is the whole ffmpeg posture proving itself.** A second package of 63
H.264 songs installed beside the first, and one played: `kind=video`, the position advancing
steadily, `km-video` reporting frame threading over three threads, and no decoder warning. That is
the four embedded frameworks resolving through `@rpath` at launch, libavformat opening an `mp4` out
of a package and libavcodec decoding it, on a device that loads no library from outside its bundle.

**All three kinds of song play**, which is the answer that matters: 154 MIDI, 64 video and 223
MP3+G in one catalog of 441, from three packages, each kind started over the API and each advancing
in real time — and confirmed on the screen as well as in the transport, which is the half an API
cannot answer for.

**An MP3+G song takes about four hundred milliseconds to become ready**, and a `play` sent inside
that window is not refused — it is simply not yet the thing being asked about. symphonia demuxes the
file and a fresh output stream opens at a 1024-frame period, and only then does the transport leave
`idle`. Long enough to read the state three times and conclude wrongly that nothing started.

**2 GB arrived over the tunnel and was installed where it lay**, not copied into the machine's own
packages folder — which is what keeps a container the size of its content rather than twice it.

**A package the machine cannot read is skipped, not fatal.** A stale one sitting beside the good
one failed with `missing field 'value'` — a manifest from before that field existed, which would
fail on a desktop too — and the run continued: `installed=1 removed=0 problems=1`, catalog served,
machine up. Worth knowing because a folder somebody drops files into will accumulate exactly that.

### The packages folder was named twice, and only a device said so

**`Paths::in_container` appended `packages` to the `Documents` directory, and `packages_dirs`
appends it again** — so the machine scanned `Documents/packages/packages`. Nothing failed. The
folder the Files app shows is `Documents/packages`, and a `.kmpkg` dropped there would have sat
beside a scan looking one level deeper, with the only evidence a startup line naming both folders.

The fix is that `extra_data_dir` holds the *container's* directory and the subdirectory is
`packages_dirs`'s to append, which is exactly how Android's public directory already works. A unit
test now pins it, because nothing else here could: the constructor is behind
`cfg(target_os = "ios")` at its only call site, and every other platform reaches the same function
with a directory that has no second half to double.

### Two lines UIKit prints at launch

**`UIApplicationSupportsIndirectInputEvents` reads like a warning and is a missing feature.** Without
it a trackpad or mouse reaches nothing on the screen, which on an iPad with a keyboard case is a
real loss over a grid of buttons. Touch is unaffected either way.

The other two are SDL's own and harmless: an unbalanced appearance transition on
`SDL_uikitviewcontroller`, and `km-display` reporting that the window system would not take an
icon — there being no window to put one on.

## Sound needs a session nobody else sets

**cpal's CoreAudio backend configures no `AVAudioSession`**, and the category an application gets
without one is `soloAmbient`: the ringer switch silences it, and playback stops the moment the
application leaves the foreground. So `main.m` sets `playback` and activates the session before SDL
starts, which is the same shape of gap as the JavaVM cpal expected somebody else to have published on
Android, found the same way — the audio is absent while every log line says the engine started.

**The category is declared and no background mode is**, so the application suspends when it leaves
the screen and its API goes quiet until it comes back. That is deliberate: the machine pauses off the
screen, so a background-audio mode would keep it awake to render silence, which measures at 4.9 %/h
against 1.6 %/h suspended. The suspend also carries the audio: iOS interrupts the session, and cpal's
iOS backend stops and resumes the output unit on that interruption, which is why the paused song
comes back at its position with sound. Deactivating the session from the application instead posts no
interruption to itself, leaving the unit running against a session that is gone and a transport that
answers while advancing nothing. See
[`The machine sleeps when it leaves the screen`](../decisions/audio.md#the-machine-sleeps-when-it-leaves-the-screen),
and for the socket
[`the listening socket does not survive a suspend`](#the-listening-socket-does-not-survive-a-suspend).

**`km-audio` needs nothing at all.** `decide` takes `linux: bool` as a parameter rather than reading
a `cfg`, and the only platform-specific code in that crate is the `/proc/asound` reading, which
already has a `not(target_os = "linux")` stub. iOS takes the non-Linux arms, which is the same path
Windows and macOS take.

**Reading a built `Info.plist` costs a rebuild if done wrong.** `plutil -extract <key> <fmt> <file>`
writes the extracted value back *over* the file unless `-o -` is given, so a check meant to read one
key replaces the bundle's whole manifest with that key's value. `plutil -p` is the safe spelling.

## ffmpeg cross-compiles, and travels as four embedded frameworks

**The pinned 7.1.5 builds for `aarch64-apple-ios` with the pin's own configure line unchanged**, so
this is a fourth caller of `ffmpeg-pin.sh` rather than a fourth opinion about what ffmpeg should be.
What the cross-compile adds is `--enable-cross-compile --target-os=darwin --arch=arm64` with a
sysroot and a version-min flag: `--sysroot` alone is not enough, because the version-min is what
stops the libraries claiming a macOS deployment target, and that surfaces as *building for iOS but
linking object built for macOS* when Xcode links the app rather than as anything ffmpeg says.

**`--install-name-dir=@rpath` does the work `install_name_tool -id` would otherwise do**, stamping
each library as `@rpath/lib<name>.<abi>.dylib` at link time. What still has to be rewritten is the
naming: a framework called `avcodec.framework` holds a binary called `avcodec`, and libavformat
holds references to `libavcodec.61.dylib` that would otherwise send the loader after a file the
bundle does not contain.

**An xcframework each, for the reason the Rust staticlib gets one.** A device framework cannot run
in a simulator and a checkout has one `Frameworks/`, so Xcode picks the slice by SDK. `embed: true`
with `codeSign: true` is what makes the LGPL posture legal rather than convenient: the libraries
travel *beside* the binary, which is compliant where linking them into it would not be, and iOS
refuses only a library loaded from outside the bundle.

**The decoder set is not narrowed.** Android cuts it to nine because libavcodec's link line runs past
Windows' 32,767-character command line; this build only ever runs on a Mac, so the pin's rule holds
and a video song that plays on the appliance plays here.

**The terms are copied by the assets step, not beside the libraries.** That is where Android lost
them once — Gradle packages only `*.so` out of `jniLibs` — and it is keyed off the wrapped
xcframeworks rather than off a flag, so a `--no-video` build carries no license for a decoder it
does not have. The check is what arrives in the built `.app`, which is 179 MB in debug with the
frameworks at 15 MB of it.

**The four are named in a generated fragment rather than in `project.yml`.** xcodegen refuses a
dependency whose xcframework is not on disk, and `--no-video` produces none of them, so a committed
list makes that build fail with four `There is no XCFramework found` errors naming a decoder nobody
asked for. `build.sh` writes `ffmpeg.yml` every run — the four when there is video, an empty list
when there is not — and xcodegen concatenates arrays across an include, so the fragment adds to the
target rather than replacing anything in it.

**bindgen needs the sysroot named per slice.** `ffmpeg-sys-next` runs it against ffmpeg's headers,
and without `BINDGEN_EXTRA_CLANG_ARGS_<triple>` carrying `--target` and `-isysroot` it parses them
with the host clang's idea of the target and fails inside `TargetConditionals.h`, long before
reaching anything of ffmpeg's. The variable spells the triple with underscores, which is the same
form `tools/port/machine/android/ffmpeg.sh` writes for its two.

## A package arrives as a URL, not a path

**SDL's UIKit delegate answers `application:openURL:` by sending a drop event**, so a package opened
from the Files app reaches the handler a dragged one already used and the machine needs no new
route. It carries `url.absoluteString` though, where every other platform sends a filesystem path,
so an *Open in* would hand `Catalog::install` a relative path beginning `file:` and be refused as a
file that is not a package — a message about the file rather than about the fault.
`display::dropped_path` strips the scheme and percent-decodes, on bytes rather than characters,
because an escape is one byte of UTF-8 and a name with an accent in it arrives as two that mean
something only together.

**`DRAG_AND_DROP` does not gate that handler**, which is what makes this work alongside Phase 5's
answer: the constant decides whether the empty-catalog message *offers* the route, and the arm
itself compiles everywhere.

**`register.rs` needs no iOS arm**, and reaches the same fallback Android does for a different
reason. A bundle declares its own types, so *Open in* works with nothing written anywhere; the macOS
arm exists only to nudge LaunchServices while a bundle is rebuilt in place, and there is no such
command on a phone. `--register` is unreachable there in any case, since it arrives through `cli`
and `SDL_main` is handed no argument vector.

## What the platform decides

**All six capability constants answer the same way Android's do, and each for its own reason** —
which is the point of their being six. `FULLSCREEN_IS_FIXED` because there is no desktop behind the
application; `KEY_HINTS` false because an iPad may have a keyboard and mostly has none, which is the
case that makes the constant a guess rather than a fact; `FILE_MANAGER` false because nothing here
reveals a folder, Files app or not; `WEB_BROWSER` false because `km-osopen` has only an `xdg-open`
arm to fall into; `WINDOW_STACKS` false because one application owns the screen.

**`DRAG_AND_DROP` is false while the drop handler still runs, and the two are not in tension.** The
constant decides whether the empty-catalog message offers the route; the handler is what an opened
document arrives through. Nothing on a phone drags a file onto a window, so the sentence would be
wrong — and the arm has to stay, or *Open in* reaches nothing.

**`number_pad` defaults on**, for Android's reason: it is the only way to enter a song number where
there is no keyboard. It stays a setting rather than a `cfg!` because an iPad with a keyboard
attached is an ordinary thing the platform cannot be asked about.

**mDNS is off by default and needed no new seam.** `run_advertiser` already returns early on
`config().advertise_mdns`, which the test harness sets through `without_mdns()`, so iOS takes the
switch that exists. It is a setting rather than a `cfg!` so that somebody who obtains
`com.apple.developer.networking.multicast` turns it on without a rebuild, and so that an
`NWListener` in Swift has somewhere to arrive. The reason it is off is that the entitlement is
granted only after a manual review and its absence is silent: the packets are dropped and the
application is not told.

## The listening socket does not survive a suspend

A suspended application is handed back a descriptor that accepts nothing. The process is otherwise
whole: it draws, it plays, and it goes on showing the address it worked out at startup, because
`run_connect_refresher` re-resolves against the address it was told was bound and has no way to ask
whether the socket is still there. A machine in that state answers a connection with a reset, which
is what an unused port answers, and nothing anywhere says so.

`km_api::listener::HeldListener` is what closes it. It implements `axum::serve::Listener`, so
`axum::serve` drives it exactly as it drives a `TcpListener`, and it takes the port again when
accepting fails. The stock implementation cannot: it treats every accept error as transient and
retries against the same descriptor, which is right for a full file table and wrong for a socket that
is gone.

**Accept failing is not the only way in, because on this platform it is not guaranteed to fail.** The
same SDL event watch that pauses the music on the way out raises a flag on the way back in, and
`Machine::settle_relisten` turns that into `Relisten::request` on the watchdog thread fifty
milliseconds later. The watch itself does one atomic store, for the reason it always has: it runs on
the platform's UI thread. The request is unconditional rather than guarded by a check, because
proving a socket works costs a connection to it and answers for that instant alone.

**`ApiState::is_listening` is what stops the recovery being papered over.** The connect refresher goes
on running while the machine is between sockets, and what it works out stays true of the interfaces,
so publishing it would paint a healthy address over the failure the listener put on the screen.

**`tap_io` with an empty closure is load-bearing** at the call site in `serve_with_shutdown`. A
handler reads the caller's address through `ConnectInfo<SocketAddr>`, and `axum` supplies that for its
own `TcpListener` and for any listener inside a `TapIo`. The third impl cannot be written in this
workspace: `Connected` and `SocketAddr` are both foreign, and a local type in a foreign type's
parameters does not satisfy the orphan rule.

## What the bundle holds

**The asset tree is a folder reference, and that word is the mechanism.** `type: folder` in
`project.yml` makes Xcode copy the directory to the bundle root with its structure intact, where a
group of file references would flatten it into `Resources/`. An `.app` keeps its executable at the
bundle root, so `Paths::asset_dirs_from` finds `assets` beside it with no code change.

**Counted in the bundle rather than counted towards it**, which is the check Android's lost
`COPYING.LGPLv2.1` taught: `build.sh` reports what `find` sees inside the built `.app`, not what the
staging step copied. A debug device build is about 163 MB, of which the bank is 31 MB.

**The font is fetched and pinned, because a Mac has no DejaVu.** The Linux tarball copies
`DejaVuSans.ttf` out of its Debian build container; there is no equivalent path here, and the faces
macOS does ship are not redistributable. So `fetch-assets.sh --font` pins the same face by digest
into the shared asset cache and the build stages it as `fonts/karaoke.ttf` with its license beside
it. `km-display`'s system-path list has no iOS entry and gains none: those paths are undocumented,
which is the way `PingFang.ttc` was learned.
