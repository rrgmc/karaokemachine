# Research: the machine on a Meta Quest 3

**Research only. Nothing implemented, nothing decided.** A platform is a product decision, so a
headset carrier needs an entry in `docs/decisions/distribution.md` beside
[`What the machine *is*, on Linux`](../decisions/distribution.md#what-the-machine-is-on-linux) and
[`What the machine *is*, on iOS`](../decisions/distribution.md#what-the-machine-is-on-ios) first.

Investigated 2026-09-19. **The headset claims here were measured on a Meta Quest 3**, and the marker
on each says how far it can be trusted.

**Summary.** The existing Android APK installs on a Quest 3 and runs as a flat Horizon OS panel. It
needs no code change, no manifest change and no new build. Every song kind plays. The words highlight
in time, the controllers drive the keypad, and a phone finds the machine over mDNS unaided.

One behaviour differs from a phone, and it follows this machine's own rule rather than the headset's.
Horizon OS stops the activity when the headset leaves the wearer's face. The machine pauses, holds
its position, and waits for somebody to press play.

The product problems are larger than the port. Only the wearer sees the words, and the room sees
nothing. A headset has no hardware mixer for the microphones. **An immersive build is out of reach
today**, because SDL carries OpenXR from 3.6.0, which falls near January 2027 on SDL's own cadence.
That route also moves the renderer to Vulkan.

| Marker | Meaning |
|---|---|
| **[repo]** | Read from this repository, or measured on a Quest 3. High confidence. |
| **[source]** | Read from SDL's source or its release tags. High confidence. |
| **[web]** | Public documentation and store listings. Second-hand. |
| **[inferred]** | Reasoning. A hypothesis to test. |

## 1. Horizon OS runs the APK unchanged

**[repo]** `adb install` takes the ordinary debug APK, and Horizon OS opens it as a flat panel of
1600x900 at 200 dpi. Nothing in `ports/machine/android/app/src/main/AndroidManifest.xml` mentions VR,
and that absence is what produces the panel.

**[repo]** The APK already met every condition the headset imposes. It builds `arm64-v8a`. It marks
GL ES 2.0 as not required and a touchscreen as not required. It asks for no `RECORD_AUDIO` and uses
no Play Services.

**[repo]** SDL recognises the hardware by itself, and the log names `Manufacturer: Oculus` and
`Model: Quest 3`. `ports/machine/android/app/src/main/java/org/libsdl/app/SDLActivity.java:1292`
carries SDL's own `isVRHeadset()` check and a Quest back-button workaround. So the panel gets
Quest-aware input handling with no work here.

**[repo]** Wireless debugging needs `adb tcpip 5555` over a cable once per boot. Horizon OS shows no
pairing-code screen, so the pairing flow in [`DEPLOYING.md`](../../DEPLOYING.md) does not reach it.

**[repo]** The remote's own APK installs and runs on the headset the same way, as a second panel. It
reaches a machine and drives it. So one headset can carry the machine and a remote at once, and each
gets a window of its own.

## 2. What the headset measured

**[repo]** Measured on a Quest 3 running Horizon OS, which reports Android 14 and SDK 34. Three test
packages went into the app's external packages folder, covering MIDI, MP3+G and video.

| Check | Result |
|---|---|
| Installs and opens as a panel | Yes, 1600x900 at 200 dpi |
| A MIDI song, words highlighting in time | Yes |
| An MP3+G song, graphics in time | Yes |
| A video song | Yes, smooth |
| The controllers reach the keypad | Yes, point and click |
| A phone remote finds it over mDNS | Yes, with no address typed |
| Audio through the headset speakers | Clean, and in step with the words |
| The remote's APK, as a second panel | Yes, and it drives a machine |
| Moving and resizing either panel | Yes, to any size the shell offers |
| Bluetooth headphone delay | Not measured |

**[repo]** The renderer is `opengles2` with vsync on, and SDL picks it by itself. Nothing in this
workspace pins `SDL_HINT_RENDER_DRIVER`.

**[repo]** The audio stream opens with a period of 1922 frames, which is 40 ms at 48 kHz. That period
sets the floor on how tight the sound can feel to somebody singing.

**[repo]** The video decoder takes 7 threads with frame threading, and the log reports no trouble at
all. [`docs/architecture/android.md`](../architecture/android.md) measured the same decoder starving
on four Cortex-A55 cores single-threaded, so the headset's silence here is the interesting part.

**[repo]** One warning appears at startup and means nothing:
`km_display::icon: the window system would not take an icon`. Horizon OS draws the window furniture
itself.

**[repo]** The machine logs `starting on a phone`. That line names the wrong device class on a
headset, and it changes no behaviour.

**[repo]** The wearer moves either panel and resizes it to anything the shell offers, and both apps
redraw at the new size. SDL declares the surface resizable and Horizon OS honours it. **This is most
of what an immersive build would buy**, and it costs nothing.

## 3. The headset coming off pauses the machine

**[repo]** Taking the headset off drives the ordinary Android activity lifecycle. SDL reports
`onWindowFocusChanged(): false`, then `onPause()`, `surfaceDestroyed()` and `nativePause()`.
`km_app::machine` logs `the machine's visibility changed on_screen=false`, and pauses.

**[repo]** Putting the headset back on returns the window and leaves the machine paused. The API
reports `transport: paused`, with the position held to the millisecond and the queue intact. One call
to `/api/v1/transport/play` resumes the song where it stopped.

**[inferred]** This is the machine's own on-screen rule rather than a Quest defect, and the rule
serves a phone well. A phone in a pocket should not keep playing. A headset set down for a moment is
a different situation, and nothing tells the two apart today.

## 4. An immersive build is out of reach today

The alternative to a flat panel is a window of this project's own. One large screen sits in the room,
with the real room behind it. That route runs through OpenXR, and the obvious path to OpenXR is SDL.

**[source]** The workspace builds **SDL 3.4.14**. `Cargo.lock` resolves `sdl3-sys 0.6.8+SDL-3.4.14`,
and the vendored `SDL_version.h` agrees.

**[source]** `SDL_CreateGPURenderer` and `SDL_PROP_TEXTURE_CREATE_GPU_TEXTURE_POINTER` are both in
that tree, so the bridge from the 2D renderer to the GPU API exists.

**[source]** `SDL_PROP_GPU_DEVICE_CREATE_XR_ENABLE_BOOLEAN` is absent from it, and SDL 3.4.14 carries
no OpenXR backend. Its only XR-shaped code is an OpenVR driver for SteamVR on a desktop PC.

### The version that carries it is SDL 3.6.0

**[source]** Every function in `include/SDL3/SDL_openxr.h` is annotated
`available since SDL 3.6.0`. That names the release, and the header reached `main` in commit
`9a91d723` on 2026-01-30.

**[source]** SDL 3.6.0 does not exist. The newest stable release is **3.4.16**, from 2026-09-02, and
`main` calls itself 3.5.0. An odd minor is the development line, and 3.5.0 becomes 3.6.0.

**[source]** No 3.5 preview or prerelease tag exists either. The 3.3.x line carried `preview-3.3.2`,
`prerelease-3.3.4` and `prerelease-3.3.6` ahead of the 3.4.0 release. Previews come first, and none
has come.

**[source]** Stable minors arrive about eleven and a half months apart: 3.2.0 on 2025-01-21, and
3.4.0 on 2026-01-01. Micro releases inside a line arrive monthly. **On that rhythm 3.6.0 falls near
January 2027.**

**[source]** The Rust binding is not what gates this. `sdl3-sys 0.7.0+SDL-3.4.16` reached crates.io
on 2026-09-03, one day after SDL 3.4.16.

### And the SDL route needs Vulkan

**[web]** SDL's OpenXR reaches a headset through the GPU API, on **Vulkan, D3D12 and Metal**. It
offers no OpenGL ES path. The Android manifest that SDL documents marks
`android.hardware.vulkan.level` and `android.hardware.vulkan.version` as required features.

**[repo]** §2 measured this machine drawing on `opengles2`. So the SDL route is a move to Vulkan as
well as a wait for a release. `SDL_CreateGPURenderer` is what would carry the 2D drawing across.

**[web]** The same manifest needs `<category android:name="com.oculus.intent.category.VR" />`.
Without it an application launches in 2D, which is why the APK in §1 opens as a panel.

**[repo]** The safe `sdl3` 0.18.4 wrapper exposes no `create_gpu_renderer`, so even the bridge needs
raw `sdl3::sys` calls.

**[inferred]** A spike through SDL therefore waits for 3.6.0 and then moves the renderer. The route
that works against a released SDL is the `openxr` crate, version 0.22.0, with
`XR_KHR_opengl_es_enable`. It shares SDL's EGL context and copies each frame into the swapchain
image. That means hand-written OpenXR code, and it is the only one of the two routes that keeps the
renderer the headset already ran.

**[repo]** The drawing code suits either route. Every function in
`crates/playback/km-display/src/draw.rs` is generic over SDL's `RenderTarget`, so it draws anywhere.
`crates/playback/km-display/src/offscreen.rs` already draws a `Frame` with no window, and keeps its
caches across frames.

**[repo]** The frame loop is what a headset would break.
`crates/machine/karaokemachine/src/display.rs` owns a window and paces itself on vsync. In VR,
`xrWaitFrame` and `xrEndFrame` set the pace instead. That file is the largest single change an
immersive build would need.

## 5. What a headset does to the product

These matter more than the port, and they hold for a flat panel and an immersive build alike.

**[inferred]** Only the wearer sees the words. A karaoke machine serves a room, and the room sees
nothing. Casting to a television gives the room a picture and adds delay to it.

**[repo]** The microphones are the harder problem.
[`Microphones`](../decisions/audio.md#microphones) puts mixing in hardware and applies no DSP. A
headset has no mixer, so the wearer hears the music in the headset and their own voice through the
air.

**[inferred]** Bluetooth headphones add delay a singer notices, and the 40 ms output period in §2
comes before any of that.

**[repo]** The machine pauses when the headset comes off, as §3 describes. Somebody adjusting the
strap interrupts their own song.

**[inferred]** A headset therefore suits one person practising alone, and it is a poor fit for a room
of singers. Passthrough helps with the room, because the wearer still sees it, and it answers neither
of the first two problems.

## 6. What others have built

**[web]** Three karaoke titles sell on the Meta store, and all three are full VR applications.
Songbird is a singing game with a story world. KaraMeta is a virtual room with a sound stage, a vocal
guide and key controls. SingRoom is social karaoke with other people.

**[web]** All three carry their own song catalogs, and all three process the headset microphone with
effects such as echo. None plays an owner's own MIDI, MP3+G or video files. No report of a user-file
karaoke player on the Quest was found.

**[web]** Ordinary Android applications do run as flat panels. RetroArch installs from its normal APK
and behaves much as it does on a phone, which makes it the closest match to this machine. PPSSPP took
the other route and ships a separate OpenXR build.

## 7. Recommendation

**The flat panel works, and it is a sideload rather than a carrier.** Somebody who owns a Quest 3 can
install today's APK and sing to it. That costs this project nothing: no code, no build, no release
artefact, and no row in the download table.

**Do not build an immersive version.** Both routes are expensive. Waiting for SDL 3.6.0 costs about a
year and then a move to Vulkan, and the `openxr` crate costs weeks of hand-written OpenXR. The frame
loop is the largest file in the machine either way, and the product problems in §5 would survive all
of it. The panel already moves and resizes, so what the work buys is a curved screen, no system
frame, and passthrough behind it.

**The question worth answering first is the microphone**, and it is a product question rather than a
build one. A headset with no mixer reopens a standing non-goal. Until somebody reopens it, a headset
stays a practice device for one person.

Two measurements remain cheap and would sharpen this note. Bluetooth latency decides whether anybody
can sing through headphones at all. And a longer session would say whether the headset throttles
video decode as it warms.

## Sources

- Meta's guide to making an existing Android application compatible with Horizon OS.
- Meta's native OpenXR and passthrough documentation, for floating panels and `XR_FB_passthrough`.
- The Meta store listings for Songbird, KaraMeta and SingRoom.
- RetroArch and PPSSPP, for the two routes an existing Android application can take.
- SDL's `README-xr`, read for the backends the GPU API offers and for the Android manifest a VR
  application needs.
- `include/SDL3/SDL_openxr.h` on `main`, read for the version each function is annotated with.
- SDL's release tags and their dates, read for the cadence between stable minors.
- The `sdl3-sys` and `openxr` crates on crates.io, read for their versions and publication dates.
- `Cargo.lock`, `crates/playback/km-display/`, `crates/machine/karaokemachine/src/display.rs`,
  `ports/machine/android/` and `docs/architecture/android.md` in this repository.
