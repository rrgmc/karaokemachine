# Samsung Tizen television

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

The machine as an application on a Samsung television. The requirements row is
[`What the machine *is*, on a Samsung television`](../decisions/distribution.md#what-the-machine-is-on-a-samsung-television),
and the measurements behind every choice here are in [`docs/research/tizen.md`](../research/tizen.md).
[`android.md`](android.md) is the sibling: the same split of a thin native shell around a `cdylib`,
with C# where Kotlin was.

## Two pieces in one `.tpk`

| Piece | What it is | Where |
|---|---|---|
| The host | a .NET NUI application, `KaraokeMachine.dll`. It creates the window, then hands it and two folders to the machine | `ports/machine/tizen/` |
| The machine | `libkm_app.so`, the whole program as a `cdylib`, exporting `km_tizen_run` | `crates/machine/karaokemachine`, behind the `tizen` feature |

Inside the package:

| Path | Holds |
|---|---|
| `bin/` | the host's assemblies |
| `lib/libkm_app.so` | the machine, stripped, about 23 MB |
| `res/assets/` | the bank, the lyric font, the wallpapers and the development console, read in place |
| `shared/res/karaokemachine.png` | the icon the app list shows |
| `tizen-manifest.xml` | package `KmMachine1`, app id `KmMachine1.KaraokeMachine`, the TV profile, and three privileges: `internet`, `network.get` and `tv.inputdevice` |

**Everything is behind a `tizen` cargo feature, not `target_os`**, because Tizen reports itself as
ordinary Linux. The one exception is how SDL links. A feature cannot take away the workspace's
`build-from-source-static`, so the triple `armv7-unknown-linux-gnueabi`, which no other carrier uses,
selects the patched SDL3 prebuilt in the build image.

## The build

`tools/port/machine/tizen/build.sh` builds the `km-tizen` Docker image, then the library inside it.

- **The image** is Debian bookworm's armel cross-compiler, raised to ARMv7 with NEON. Bookworm's
  glibc, 2.36, is older than the set's 2.40, so nothing links against a symbol the set lacks.
- **The triple is soft-float by default**, so the build turns hardware floating point back on with
  `-C target-feature=-soft-float,+neon,+vfp4`. Left as it ships, every sample the synthesizer makes
  would go through a software float library.
- **SDL3 and SDL3_ttf are built static into `/opt/sdl3`**, from the sources the `sdl3-sys` crates
  would build, with `sdl3-tizen.patch`. That patch is two one-line guards in the Wayland driver: the
  compositor announces an output scale of 0, which SDL divides by, and a refresh rate in hertz where
  the protocol says millihertz. The patch stays local, since SDL takes no AI-written contributions.
- **ffmpeg 7.1 is in the image for linking only.** On the set its own `libavcodec.so.61` and
  siblings answer, so nothing of ffmpeg's ships.
- **EFL and `libwayland-client` are opened with `dlopen`**, as is `audio_io`. The set has them and
  the image does not.
- **The script ends by reading the library back** with `readelf`. It checks the softfp loader, the
  two exports, every needed library against what the set is known to carry, and the newest glibc
  symbol needed.

**Two builds of the same library.** The plain build carries the assets inside the `.so`, through the
`tizen-embed-assets` feature, for the probe app below. `--app` leaves them out, because the `.tpk`
carries them in `res/assets`. `package.sh` makes the `--app` build, then builds the host with the
.NET SDK and Samsung's Tizen workload into a `.tpk`.

## Starting: the host draws first

`ports/machine/tizen/Main.cs`, in order:

1. `OnCreate` sets a black background and adds a label, "Karaoke Machine".
2. `AddFramePresentedCallback` reports when that frame has reached the screen. A second later, or
   five seconds after creation if no report comes, the machine starts.
3. `NativeLibrary.Load` opens `lib/libkm_app.so`.
4. The window handle comes from NUI's internal `Window.GetNativeWindowHandler()`, called through
   reflection. It returns the `Ecore_Wl2_Window *`, and it is the same in TizenFX API 8 to 12.
5. `km_tizen_run(window, width, height, data folder, res/assets, log)` is called on the UI thread,
   and does not return until the machine ends.

**The label and the wait are not decoration.** The set keeps its launch animation up until the
application's window is on screen. Called straight from `OnCreate`, with an empty window, the
machine took the thread before anything was drawn. The set then showed its spinner over the home
screen for ever, while the machine ran unseen behind it with its API answering.

If the machine returns an error, or cannot be loaded, the host replaces the label with the reason,
and Back closes the app. Nobody reads a television's log.

## Whose loop runs: the nested loop

**The machine's frame loop runs on the host's UI thread and lets Ecore do its work once per
frame.** SDL and DALi read one Wayland queue, so SDL has to live on that thread. Without the Ecore
main loop turning, Tizen closes the app after about twenty seconds, and no key, lifecycle or system
event arrives. `crates/machine/karaokemachine/src/tizen/window.rs` is the whole of it:

- **SDL shares the host's `wl_display`.** A window of its own, on its own connection, sticks on an
  old frame within seconds.
- **SDL's surface is a desynchronised subsurface above the host's window**, with a custom role,
  because the compositor offers no `xdg_wm_base`. `wl_subcompositor` is bound through a registry of
  its own on a private event queue, so nothing of DALi's is dispatched.
- **It starts transparent and is declared opaque after its first frame.** An opaque region on the
  first frame takes keyboard focus from the application for good. Left transparent, a third of the
  frames run late once sound plays.
- **While a system overlay has focus, the opaque region is cleared**, and set again a frame after
  focus returns. Otherwise focus never comes back.
- **Each frame waits in Ecore until an animator ticks**, so the UI thread keeps turning. SDL's
  vsync is off.
- **The window is never destroyed and SDL never quit.** Either kills the host. The ending destroys
  the subsurface and commits a null buffer, then ends the app with `ui_app_exit`, or with
  `ecore_main_loop_quit` where that is missing.

## Hidden

**Home, another input or an app over the machine hides it, and it then stops drawing and leaves the
screen.** The rule is
[`Leaving the screen stops the music`](../decisions/interface.md#leaving-the-screen-stops-the-music):
a playing song pauses, and no demo starts. SDL's Wayland driver raises no background event, so two
witnesses report it instead, and either is enough:

| Witness | Route |
|---|---|
| NUI's `OnPause` and `OnResume` in `Main.cs` | the export `km_tizen_set_visible`. A pause before the library is loaded is passed on when it loads |
| Ecore's `ECORE_WL2_EVENT_WINDOW_VISIBILITY_CHANGE` | `fully_obscured`, read in `tizen/window.rs` |

Both run on the UI thread, inside the Ecore iterate the frame loop turns, and only store a flag.
`Frames::wait` takes it. While hidden, the display loop skips drawing and presenting and only waits
in Ecore. A present into a surface the compositor does not show could wait in EGL for a buffer, and
Tizen closes an app whose UI thread stops. The opaque region is cleared on hiding and set a frame
after the app is shown again, as after an overlay. Focus and visibility changes are logged at info.

**Measured on the set through the probe app**, so with Ecore's witness alone: Home and a change of
input hide the app and pause the song, and it draws again on return; the set's menu is only a focus
change, and the song plays on under it. The `OnPause` route waits for an installed `.tpk`:
[`docs/research/tizen.md`](../research/tizen.md) §9, item 7.

## Keys

**SDL's own `wl_keyboard` receives the remote's keys**, because it shares the host's connection.
Back arrives as `AC Back`, which the machine already treats as back. Pushing Ecore's keys into SDL
as well delivered every key twice, so nothing is pushed.

**Back is held back from the host.** An Ecore key handler, prepended ahead of DALi's, swallows
`XF86Back`. Otherwise NUI would end the application on every press. The machine decides what Back
means, and on the first screen it ends the app.

**The media keys are grabbed.** The set keeps Play/Pause, Stop, Next and the rest for itself unless
a window grabs them, and shows a "not available" notice instead. `ecore_wl2_window_keygrab_set`
takes nine of them in TOPMOST mode, which works under the free certificate a sideloaded package
carries. On the set all nine were taken, and Play/Pause paused a song. Every key name Ecore sees is
logged at debug.

## Sound

**`audio_io`, not ALSA.** The platform's audio server holds the hardware, so ALSA opens its device
and fails the first write. `km-audio`'s Tizen output, `audio/audioio.rs`, opens
`libcapi-media-audio-io.so.0` with `dlopen`. A thread renders 1,024-frame blocks, converts them to
S16 and hands them to the blocking `audio_out_write`. That blocking write is the clock the words
are drawn from. The device buffer is 50 ms, and the words needed no latency correction.

**The stream opens with the machine and is held for the run, and its silence is dithered.**
Through a receiver, every song lost its first two or three seconds while the way to the speakers
woke up. So on Tizen the engine ignores `audio.idle_release_secs`, and the S16 conversion adds one
step of triangular dither. See
[`Holding the audio device`](../decisions/audio.md#holding-the-audio-device).

**A song asks the set for the sound, as on Android.** `tizen/soundfocus.rs` is `audiofocus`'s three
platform calls over `libcapi-media-sound-manager.so.0`, opened with `dlopen`: one media stream
information, made on the first song and kept, `sound_manager_acquire_focus` while a song plays and
`_release_focus` when it stops. The focus callback stores one integer, which the machine's tick reads,
and the shared policy pauses or resumes; see
[`The machine asks Android for the sound, and gives it back`](../decisions/audio.md#the-machine-asks-android-for-the-sound-and-gives-it-back).
The stream information is not attached to the `audio_io` handle, so the sound server cannot refuse
the output over focus. Every change the set reports is logged at info. **Measured with the voice
assistant**: its spoken answer pauses the song and its end resumes it, but its listening silences
the machine without a focus change, so the song plays on unheard there:
[`docs/research/tizen.md`](../research/tizen.md) §9, item 8.

## Video

The set's ffmpeg 7.1 decodes, through the same `km-video` the desktop uses. Frames go to SDL as YUV
textures, so the GPU converts the colour. 1080p H.264 plays at its own rate. The set's build carries
no Vorbis or Opus decoder.

## Folders and the network

- **The data folder** is the app's `DirectoryInfo.Data`, handed down by the host. Settings, the
  catalog and installed packages live there, as everywhere else.
- **The assets are read in place from `res/assets`**, which is read-only, and nothing is copied out.
  `Paths::asset_dir` points there.
- **The API binds where settings say**: `0.0.0.0:8177` by default. Phones reach it, and mDNS
  advertising works on the set.

## The probe app, for development

`KM Probe` is a small NUI app installed on the development set, and it is not in this repository. It
listens on port 8177. `POST /load` takes a library, writes it into the probe's data folder, loads it
and calls `km_probe_run` on its UI thread. `GET /log` reads what was logged.
`tools/port/machine/tizen/load.sh` drives it. That is how M2 to M4 ran the machine on the set with
nothing installed, rebuilding and reloading in about a minute.

`km_probe_run` takes its folders in a `key=value` string (`tizen/args.rs`). Its API defaults to
8277, because the probe holds 8177. It writes the assets carried in its `.so` into the data folder
on first start (`tizen/assets.rs`).

## Which televisions

Measured on one 2025 set, upgraded to Tizen 10. The host targets `net6.0-tizen8.0`, so a 2024 set
on Tizen 8 is plausible but untried, and a 2023 set on Tizen 7 may refuse it.

**A firmware update can move what this rests on**: the focus behaviour, the compositor, and the
ffmpeg sonames the video path loads. The probe app is how to re-measure them without installing
anything. What older sets would need is in
[`docs/research/tizen.md`](../research/tizen.md), "Which televisions: the Tizen versions".
