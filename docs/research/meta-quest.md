# Research: the machine on a Meta Quest 3

**Measurements, and the decision they fed.** A platform is a product decision, and the one this note
produced is
[`What the machine *is*, on a headset`](../decisions/distribution.md#what-the-machine-is-on-a-headset).
That entry is authoritative wherever the two disagree.

Investigated 2026-09-19. **The headset claims here were measured on a Meta Quest 3**, and the marker
on each says how far it can be trusted.

**Summary.** The existing Android APK installs on a Quest 3 and runs as a flat Horizon OS panel. It
needs no code change, no manifest change and no new build. Every song kind plays. The words highlight
in time, the controllers drive the keypad, and a phone finds the machine over mDNS unaided.

One behaviour differs from a phone, and it follows this machine's own rule rather than the headset's.
Horizon OS stops the activity when the headset leaves the wearer's face. The machine pauses, holds
its position, and waits for somebody to press play.

**An immersive build is cheap, and Meta Spatial SDK is what makes it cheap.** A Kotlin activity of
about fifty lines hosts the existing SDL activity as a panel in a scene of its own. The room shows
behind it. It runs at ninety frames a second, the lyrics are sharp, and no Rust changes. The price is
two APKs rather than one. Reaching OpenXR directly stays expensive, because SDL carries it from
3.6.0, which falls near January 2027, and that route also moves the renderer to Vulkan.

The product problems are larger than the port, and the cheap route answers none of them. Only the
wearer sees the words, and the room sees nothing. A headset has no hardware mixer for the
microphones.

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

## 3. The icon comes from the APK and the name comes from the store

**[repo]** The library tile draws `android:icon`, which the machine already carries. A sideload needs
nothing added to get its own icon, and being immersive rather than flat changes none of it.

**[repo]** The name is the half a sideload cannot supply. The shell's control bar reads
`app name unavailable` under the running application, and the log says why:

```
OVRLibrary: null cursor received for query content://com.oculus.ocms.library/apps/<package>
LibraryModule: Received null app from OCMS for package name <package>
AppManagerInternal: Entitlement not found, channels=[Store, Q4B, PcStore]
```

**[repo]** `android:label` and `android:icon` named on the immersive activity itself do not change
it. The shell asks its own library database rather than the package manager, and only a store entry
puts a row there.

**[web]** The rectangular cover art store applications show comes from the same place, keyed by
application id. Meta's asset guidelines describe the set as listing material. It holds a 512x512
icon, a 2560x1440 landscape cover, a 1440x1440 square, a 1008x1440 portrait and a 3000x900 hero.
Nothing in Meta's manifest guide puts any of them in an APK.

**[web]** MetaMetadata scrapes the store, SideQuest and OculusDB daily so that launchers such as
Lightning Launcher have banners and icons to draw. It carries nothing for an application in none of
the three, and those launchers let a wearer pick a cover by hand.

## 4. The headset coming off pauses the machine

**[repo]** Taking the headset off drives the ordinary Android activity lifecycle. SDL reports
`onWindowFocusChanged(): false`, then `onPause()`, `surfaceDestroyed()` and `nativePause()`.
`km_app::machine` logs `the machine's visibility changed on_screen=false`, and pauses.

**[repo]** Putting the headset back on returns the window and leaves the machine paused. The API
reports `transport: paused`, with the position held to the millisecond and the queue intact. One call
to `/api/v1/transport/play` resumes the song where it stopped.

**[inferred]** This is the machine's own on-screen rule rather than a Quest defect, and the rule
serves a phone well. A phone in a pocket should not keep playing. A headset set down for a moment is
a different situation, and nothing tells the two apart.

**[repo]** Resuming by itself is settled rather than open.
[`Leaving the screen stops the music`](../decisions/interface.md#leaving-the-screen-stops-the-music)
rejects auto-resume. A song that restarts on return surprises a room the way one that never stopped
does. A headset would need that decision changed, and one Android build serves phones, televisions
and headsets alike.

## 5. Meta Spatial SDK draws the machine in a panel

The alternative to a flat panel is a window of this project's own. One large screen sits in the room,
with the real room behind it. Two routes reach it. Meta Spatial SDK hosts the existing activity, and
OpenXR asks the application to drive the headset itself.

### Spatial SDK costs no Rust at all

Meta Spatial SDK is a Kotlin framework for Horizon OS. It owns the immersive scene and places
ordinary Android activities in it as panels, so an application reaches a headset without touching
OpenXR.

**[repo]** A Kotlin activity of about fifty lines put the machine in a panel. It extends
`AppSystemActivity`, registers one panel naming `MainActivity` as its `activityClass`, and places it
two metres out at eye height. The manifest gives that activity `allowEmbedded` and
`resizeableActivity`, drops its `singleInstance` launch mode, and adds the
`com.oculus.intent.category.VR` category to the new one.

**[repo]** Measured on a Quest 3, with a MIDI song playing:

| Check | Result |
|---|---|
| The machine draws in the panel | Yes, at 2880x1620, the 1600x900 dp asked for |
| Frames | 91 of 90 a second, no stale frames |
| Application time a frame | 0.97 ms to 3.34 ms of an 11.1 ms budget |
| Dropped frames | One, at 12.5 ms |
| Lyrics on a compositor layer | Sharp |
| Controller input reaching the keypad | Yes |
| Temperature | 41 C at the start, 48 C after a session |

**[repo]** Passthrough takes three things, and any two of them give a black void.
`scene.enablePassthrough(true)` and `scene.enableHolePunching(true)` ask for it, and
`<uses-feature android:name="com.oculus.feature.PASSTHROUGH" />` is what lets Horizon OS grant it.
An application missing the feature is refused silently, with no error and no warning.

**[repo]** `PT is: ON` in the system log says only that the headset offers passthrough. The count
beside it answers whether the scene submits a layer: `numLayers: 0` is the void, `numLayers: 1` is
the room.

**[repo]** The renderer never moved. The log still reads `renderer=opengles2 vsync=true`, and
Spatial SDK runs inside the machine's own process beside `SDL_main`.

**[repo]** The headset warms to 48 C and its fan becomes audible, while still holding 90 frames a
second. Passthrough is what costs this, because it runs the cameras and the depth pipeline for as
long as the scene asks for it. A flat panel leaves all of that off.

**[repo]** `adb shell am broadcast -a com.oculus.vrpowermanager.prox_close` stops the headset
sleeping when it leaves the wearer's face, which a measurement over adb needs.
`com.oculus.vrpowermanager.automation_disable` puts it back, and a headset left on the desk with
passthrough running is what makes putting it back matter. Neither survives a reboot.

**[repo]** `scene.setReferenceSpace(ReferenceSpace.LOCAL_FLOOR)` is what makes the headset's Reset
View bring the panel round. `scene.setViewOrigin` pins the origin to the tracking space instead. A
screen the wearer cannot bring back in front of them is a screen in the wrong place for good.

**[repo]** The cost is two APKs rather than one. Spatial SDK needs `minSdk` 34 against this project's
26, Kotlin against one Java activity, and three `com.meta.spatial` dependencies. Meta's own porting
guide keeps a `mobile` and a `quest` build variant, each with its own manifest. The Quest manifest
names an immersive launcher, and merging the two leaves a phone with two launcher entries.

### OpenXR is the other route, and it is the expensive one

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

## 6. What a headset does to the product

These matter more than the port, and they hold for a flat panel and an immersive build alike.

**[inferred]** Only the wearer sees the words. A karaoke machine serves a room, and the room sees
nothing. Casting to a television gives the room a picture and adds delay to it.

**[repo]** The microphones are the harder problem.
[`Microphones`](../decisions/audio.md#microphones) puts mixing in hardware and applies no DSP. A
headset has no mixer, so the wearer hears the music in the headset and their own voice through the
air.

**[inferred]** Bluetooth headphones add delay a singer notices, and the 40 ms output period in §2
comes before any of that.

**[repo]** The machine pauses when the headset comes off, as §4 describes. Somebody adjusting the
strap interrupts their own song.

**[inferred]** A headset therefore suits one person practising alone, and it is a poor fit for a room
of singers. Passthrough helps with the room, because the wearer still sees it, and it answers neither
of the first two problems.

## 7. What others have built

**[web]** Three karaoke titles sell on the Meta store, and all three are full VR applications.
Songbird is a singing game with a story world. KaraMeta is a virtual room with a sound stage, a vocal
guide and key controls. SingRoom is social karaoke with other people.

**[web]** All three carry their own song catalogs, and all three process the headset microphone with
effects such as echo. None plays an owner's own MIDI, MP3+G or video files. No report of a user-file
karaoke player on the Quest was found.

**[web]** Ordinary Android applications do run as flat panels. RetroArch installs from its normal APK
and behaves much as it does on a phone, which makes it the closest match to this machine. PPSSPP took
the other route and ships a separate OpenXR build.

## 8. Recommendation

**The flat panel works, and it needs nothing.** Somebody who owns a Quest 3 installs today's ordinary
APK and sings to it. No code, no build and no release artefact sit behind that.

**An immersive version is cheap to build and still answers nothing.** Spatial SDK costs a Kotlin
activity, a second build variant and a `minSdk` of 34. It buys a fixed screen with the room behind
it. The product problems in §6 survive all of it, and the flat panel already moves and resizes. So
the case rests on whether a fixed screen in passthrough beats a system panel the wearer places. That
is a matter of taste rather than of capability.

**The OpenXR route stays closed.** Waiting for SDL 3.6.0 costs about a year and then a move to
Vulkan, and the `openxr` crate costs weeks of hand-written OpenXR. Neither buys anything Spatial SDK
does not.

**The question worth answering first is the microphone**, and it is a product question rather than a
build one. A headset with no mixer reopens a standing non-goal. Until somebody reopens it, a headset
stays a practice device for one person.

Two measurements remain cheap and would sharpen this note. Bluetooth latency decides whether anybody
can sing through headphones at all. And a longer session would say whether the headset throttles
video decode as it warms.

## Sources

- Meta's guide to making an existing Android application compatible with Horizon OS.
- Meta's Spatial SDK documentation: the panel registration and media playback pages. Also the guide
  to adding Spatial SDK to an existing 2D application, and the `HybridSample` project.
- The Spatial SDK archives on Maven Central, read with `javap` for the signatures the documentation
  leaves out.
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
