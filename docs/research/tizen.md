# Research: the machine on a Samsung Tizen television

**Research only. Nothing implemented, nothing decided.** A platform is a product decision, so a
Tizen carrier would need an entry in `docs/decisions/distribution.md` beside
[`What the machine *is*, on Linux`](../decisions/distribution.md#what-the-machine-is-on-linux) and
[`What the machine *is*, on iOS`](../decisions/distribution.md#what-the-machine-is-on-ios) first.

Investigated 2026-09-13 and revised 2026-11-17. **Most claims here were measured on a television**, a
2025 set running Tizen 10: §2's listening port, and §3's SDL3 window, sound, frame pacing and remote
keys. The marker on each of the others says how far it can be trusted. The sibling note is [`webos.md`](webos.md), which asks the same question
of an LG television.

**Summary.** The first version of this note said a native port was impossible, because Samsung
dropped NaCl, the only route for third-party native code. That was wrong. A Tizen application package
(`.tpk`) is a .NET application, and a .NET application can load a C library. **A port in the shape of
the Android one is possible, and its gates are measured open.** An application on the television
accepts connections from the network. Sound plays through Tizen's native `audio_io`, though not through
ALSA. The remote's arrows, OK, Back and number keys arrive in SDL3 as ordinary key events. SDL3's own
Wayland driver draws with its GLES2 renderer, after two one-line patches. **It has to draw into the
host application's own window**: as a subsurface of that window, driven a frame at a time from the
host's main loop, it ran a minute at 60 fps and looked perfect. In a window of its own, the picture
sticks on an old frame. How long an install lasts is unmeasured.

**Two cheaper answers already exist.** A Google TV device on HDMI runs the whole machine on a Samsung
television. The machine's HLS stream plays on the television's own player with no port at all.

| Marker | Meaning |
|---|---|
| **[repo]** | Read from this repository. High confidence. |
| **[source]** | Read from vendor documentation or another project's source. High confidence. |
| **[web]** | Public documentation and community reports. Second-hand. |
| **[inferred]** | Reasoning. A hypothesis to test. |
| **[measured]** | Run on a television. Highest confidence, for that model year. |

## 1. Native code reaches a Samsung television through .NET

**[source]** Samsung's NaCl documentation says NaCl is supported "only until 2021-year products",
with WebAssembly as its replacement. That closes one route. It is not the only one.

**[source]** `nuvio-native-legacy` is an open-source media client in C with SDL2 and OpenGL ES. Its
release notes and the code show how it runs on Tizen:

| | |
|---|---|
| **Package** | Four `.tpk` files: Tizen 4.0–5.5, 6.0, 6.5–7, and 8–9 |
| **Host** | A .NET 8 application on Samsung's NUI framework creates the window and the EGL context |
| **Native code** | The whole C application is built as `libnuvio.so`. The host loads it and calls an entry point per vsync, and the C side draws on its own thread with the borrowed context |
| **SDL** | Used for its API only. `SDL_GL_CreateContext` and `SDL_GL_SwapWindow` are macros onto the host's EGL. EGL is opened with `dlopen` and never linked |
| **Toolchain** | A Debian buster armel Docker image, glibc 2.28, dependencies built static; the .NET 8 SDK with the Tizen workload |
| **Video** | `Tizen.Multimedia.Player` on the television's video plane, not a decoder of its own |
| **Tizen 4.0–5.5** | The television refuses native code loaded from the application folder, so the host loads the library into memory instead |
| **Tizen 8–9** | A separate GL window made the television pause and close the application when started from its menu. The drawing surface has to be a view inside the main window |

**[web]** The project reports the packages working on two Tizen 4.0 sets, two 5.0 sets, 6.0, and three
2023–2024 sets. Tizen 6.5 and 7 have few reports.

**[web]** Signing needs only a **Public** certificate, which Apps2Samsung creates and applies per
television. A test build that dropped a Partner-level privilege for Tizen 4 and 5 had no reported
result when read. Install error 118014 is what that build was made for.

**[measured]** What the 2025 set offers a native library:

| | |
|---|---|
| Platform | Tizen 10.0, Linux 5.4, .NET 8 runtime |
| CPU | ARMv7 user space (`armv7l`) with NEON on a 64-bit core |
| ABI | soft-float calling convention: the loader is `ld-linux.so.3`, so Debian's `armel` toolchain with `-march=armv7-a -mfloat-abi=softfp -mfpu=neon` |
| glibc | 2.40 |
| Libraries in `/usr/lib` | `libwayland-client`, `libwayland-egl`, `libwayland-cursor`, `libxkbcommon`, `libEGL`, `libGLESv2`, `libasound`, `libpulse`, `libcapi-media-audio-io`, `libfreetype`, `libharfbuzz`, ffmpeg's `libavcodec.so.61`, and SDL2 2.0.14 |
| Wayland shells | `wl_shell`, `zxdg_shell_v6`, an old `xdg_shell` v1, and Tizen's own. **No `xdg_wm_base`**. `wl_subcompositor` is offered |

**[measured]** An update signed with a different certificate is refused, and the television will not
remove an application over the network. A probe re-signed from scratch has to be removed on the set
by hand first.

## 2. Listening on a port: the network reaches it

**[source]** The same client calls `bind`, `listen` and `accept` on the television. It binds
`127.0.0.1` only, because the client is the television's own media player. So a native application
**can** listen, which contradicts the community reports the first version of this note relied on.

**[inferred]** Whether the network can reach it mattered more than anything else here. The machine's
control surface **is** its HTTP API: `km_api::routes::SURFACE` holds every route, and the singer's
remote is served at `/`. A machine nobody can queue a song on from a phone is a different product.

**[measured]** It can. A .NET-only `.tpk` (`net6.0-tizen8.0`, manifest API version 8, `tv` profile,
privileges `internet` and `network.get`) bound `0.0.0.0:8177` with a `TcpListener`. A phone and a PC on
the same network each fetched a page from it. No prompt appeared on the television, and no setting
was changed.

**[inferred]** The probe was C#, and the machine's API would be Rust in a native library. Both reach
the same kernel socket calls under the same application, so the result should carry. It says nothing
about the multicast socket mDNS needs. Without mDNS, the remote finds the machine by its address or
by the QR code.

## 3. SDL3 draws on the television

**[measured]** SDL 3.4.14, built static into a shared library with its Wayland driver loading the
set's own libraries, opened a 1920×1080 window and drew with the `opengles2` renderer. The window
covered the screen above the host's own. It took three things:

1. **Two one-line patches to SDL3**, both guards worth sending upstream. The compositor announces
   `wl_output.scale` as 0; SDL divides by it in `handle_wl_output_done` and the process dies with
   `SIGFPE`. Treating a scale below 1 as 1 fixes it. The compositor also reports the refresh as `60`,
   where the protocol asks for millihertz, so SDL reads 0.06 Hz. Treating a refresh below 1000 as
   hertz fixes it.
2. **A custom-role surface.** `SDL_CreateWindow` waits for a configure event from `xdg_wm_base`, which
   this compositor does not have, so it never returns. With
   `SDL_PROP_WINDOW_CREATE_WAYLAND_SURFACE_ROLE_CUSTOM_BOOLEAN`, SDL makes the surface and EGL window and
   leaves the role to the caller. The probe made it a `wl_shell` toplevel on SDL's own connection.
3. **Nothing on the UI thread that blocks.** Tizen closes an application whose UI thread stops
   answering for about twenty seconds. SDL's loop has to run on a thread of its own, or in another
   process as the probe did.

**[measured]** Sound plays through Tizen's native `audio_io`, and not through ALSA:

| Route | Result |
|---|---|
| SDL3's ALSA driver | Opens the `default` device with 1024-frame periods, then the first write fails with `EBADFD`. Silent. The platform's audio server holds the hardware |
| `libcapi-media-audio-io`, opened with `dlopen` | `audio_out_create_new(48000, stereo, S16_LE)` and `audio_out_prepare` succeed with no privilege declared. A 440 Hz tone was heard from the set's speakers. Writes were consumed in real time, 10.1 s of audio in 10.0 s. The device buffer is 9,600 bytes, 50 ms |

**[inferred]** cpal's ALSA backend opens the same device, so it should fail the same way. A Tizen
build of `km-audio` needs an output backend that pushes PCM through `audio_out_write`. That is a
small blocking-write loop, and its 50 ms buffer is in the range the machine already works with.
PulseAudio, through SDL3 or cpal, was not tried.

**[measured]** Frame pacing does not come from SDL3. With nothing pacing it, the probe presented about
465 frames a second in a separate window. Four ways were tried there, each for 16 s, timing every
present:

| Pacing | Result |
|---|---|
| SDL3's vsync, unpatched | 0.3 fps. SDL sizes its wait for the frame callback from the refresh, and 0.06 Hz makes it 16.7 s |
| SDL3's vsync, refresh patched | 26 fps, uneven. Frame callbacks rarely arrive, so most swaps fall back on SDL's 50 ms timeout |
| `eglSwapInterval(1)` on the set's EGL, SDL's vsync off | accepted and ignored: about 4,000 presents a second |
| `libtdm-client`'s vblank wait, synchronous, then present with SDL's vsync off | **58.9 fps.** Vblank timestamps step 16.6 ms, and per five seconds about 20 of 295 frames are held for two vblanks, with about 10 short ones catching up |

**[measured]** These are present timings, not what reached the screen, and the two differ.
Watched on the set, the separate window does not stay live:

| Paced at | What the screen showed |
|---|---|
| nothing, about 465 fps | live for a minute and more, twice |
| `tdm` vblank, 60 fps | stuck on an old frame within 30 s, twice, while presents carried on at 60 fps |
| a plain 60 Hz timer | stuck by about frame 30, half a second in |
| a plain 120 Hz timer | flickering between a current frame and an older one, both drawn at once |

**[measured]** After the picture stuck in a `tdm` run, the vblank timestamps stepped at exactly
16.666 ms with no misses, as `tdm`'s fake vblank does when it cannot use the output.

**[inferred]** The window is the problem, not the pacing. The compositor shows a stale buffer of
the `wl_shell` toplevel's swap chain unless new ones arrive faster than it scans out. Nuvio's notes
record the same shape on Tizen 8–9: a separate GL window made the television pause the application,
and drawing into a view inside the main window fixed it.

**[measured]** **SDL3 drawn into the host's own window is smooth.** The probe ran inside the host
process, on its UI thread:

1. **SDL shares the host's connection.** `SDL_PROP_GLOBAL_VIDEO_WAYLAND_WL_DISPLAY_POINTER` is set to
   DALi's `wl_display`, which `ecore_wl2_window_display_get` and `ecore_wl2_display_get` give from the
   NUI window's native handle.
2. **Its surface is a subsurface of the host's window.** SDL creates a custom-role surface and its EGL
   window. The probe binds `wl_subcompositor` through its own registry on a private event queue, so
   nothing of DALi's is dispatched, and makes SDL's surface a desynchronised subsurface placed above
   DALi's window surface.
3. **The host's main loop drives each frame.** An Ecore animator calls back once a frame. The probe
   draws and presents one frame per call with SDL's vsync off, so the UI thread never blocks.

It ran 60 s at 1920×1080 and looked perfect on the set. Every five seconds it presented 301 frames,
297 to 299 of them 12–20 ms apart, with one or two slow ones. The separate window had about twenty.
The animator reported its timer source, not a vblank-driven one.

**[measured]** Tearing the probe down hung the host. After the last frame, the probe destroyed the
renderer and the window on the UI thread and left SDL initialised, because `SDL_Quit` would terminate
the EGL display DALi shares. The layer went away, and the host stopped answering. The machine does not
tear its display down while it runs, so a port can avoid this, but a clean exit path is still
unmeasured.

**[inferred]** SDL dispatches the default Wayland queue, which DALi's main loop also reads. So SDL
must run on the host's UI thread, as the probe did. A Tizen `km-display` would draw from a main-loop
callback rather than from a loop of its own. That is the shape Nuvio's host takes on Tizen 6 and
later, where the .NET host calls into the C side once per vsync.

**[measured]** The remote's keys reach SDL3 as ordinary keyboard events, with no key grab declared.
The window gained keyboard focus from the `wl_shell` toplevel, and a seat keyboard named "Virtual core
keyboard" appeared. Every key pressed during a three-minute run arrived as a down and an up:

| Remote key | SDL scancode | SDL keycode |
|---|---|---|
| Up, Down, Left, Right | `Up`, `Down`, `Left`, `Right` | the same |
| OK | `Return` | `Return` |
| Back | `Escape` | `AC Back` |
| 1 to 4 | `1` to `4` | `1` to `4` |
| the 123 button, and one other | 0 | 0 |

**[measured]** The 123 button opens the set's on-screen number pad, and the number keys that followed
were typed on it. A first run seemed to show Back freezing the picture; a second showed the picture
stuck before any key was pressed. Back does nothing to the window. The freeze is the window's, above.

**[inferred]** The number keys are the song-number entry the machine's singer screen is built
around, and they arrive as they would from a keyboard. Two keys arrived with no SDL mapping, so their
XKB keysyms are ones SDL does not translate. The machine would need the raw keysym to use them, which
SDL3 does not pass on.

**[measured]** What this run does not settle:

- **4K.** The output was 1920×1080, the size the compositor gave. Whether a native application may
  draw at the panel's 3840×2160 is open.

**[repo]** The layering says which crates travel.

| | |
|---|---|
| `km-song`, `km-suitability`, `km-fixes`, `km-songcode`, `km-queue`, `km-songbook`, `km-cdg` | pure Rust. Cross-compile to armv7 Linux unchanged |
| `km-catalog` | SQLite bundled. Builds; the database lives in the application's data folder |
| `km-api` | axum over tokio. Builds; §2 measured the network reaching it |
| `km-display` | SDL3 with its own Wayland driver, measured above, with the two patches. It draws into a subsurface of the host's window, a frame per main-loop callback on the host's UI thread, not from a loop of its own |
| `km-audio` | `rustysynth` travels. Output needs a backend over Tizen's `audio_io`, measured above; ALSA is refused |
| `km-video` | ffmpeg 7.1 is already on the set as `libavcodec.so.61`. Software decoding of 1080p on its processor is unmeasured. The alternative is the television's player on its video plane, as the client in §1 does |

**[inferred]** The Android port needed changes in five files. This needs a .NET host application, the
custom-role window, the two SDL patches, `tdm` pacing and an `audio_io` output backend, on top.

## 4. Songs and files

**[source]** A web application reads USB storage through the FileSystem API, and writing needs the
`filesystem.write` privilege. **[inferred]** A native package has the application's own data folder
and, with storage privileges, the media folders. Which of those a USB stick appears under is
unmeasured. A library in `package_dirs` is a folder, so no removable-media handling is needed either
way.

## 5. How long an install lasts

**[web]** Sideloading needs Developer Mode, and community reports say the grant expires after seven
days and breaks on a firmware update. Certificates last about a year and are reissued after a
firmware update.

**[inferred]** Whether the expiry stops an **installed** `.tpk` from starting, or only ends the
connection used to install one, is unresolved. People run the client in §1 as their everyday player,
which suggests the second. It is the third measurement: install, leave the television alone for ten
days, and start the application.

## 6. The television as a screen for a machine elsewhere

**[repo]** This was a gap when the note was first written, and it has since been decided and built.
[`A television somewhere else is a stream, and the playlist is the interface`](../decisions/streaming.md#a-television-somewhere-else-is-a-stream-and-the-playlist-is-the-interface)
makes the machine serve one continuous HLS stream at `/stream/live.m3u8`, picture, words and sound
for every song kind. A Samsung television's own player plays it, and the stream is the only sound in
that room, so nothing has to be kept in time across two boxes.

## 7. Against the LG answer

| | Samsung, Tizen | LG, webOS |
|---|---|---|
| Native ARM code from a third party | a `.tpk` with a .NET host, Public certificate | unofficial, and installs on a stock television |
| SDL | SDL3's Wayland driver draws into the host's window, measured on Tizen 10, with two patches | SDL2 has a community fork; SDL3 has nothing |
| The machine's HTTP API | reachable from the network, measured on Tizen 10 | ordinary sockets |
| Reading songs off a stick | unmeasured for a native package | a directory walk, subject to the jailer |
| Sideload lifetime | seven days reported; effect on an installed package unknown | 1000 hours |
| Sound | Tizen's `audio_io`, measured; ALSA refused | unmeasured |
| Frame pacing | the host's main loop, 60 fps and smooth on screen, measured | unmeasured |
| The remote's keys | arrows, OK, Back and numbers reach SDL3, measured | unmeasured |
| Verdict | a port; the install's lifetime is unmeasured | a port, gated on the display |

## 8. What answers the question today

**[repo]** `.cargo/config.toml` names the Google TV Streamer and Chromecast with Google TV, and armv7
is built because both run a 32-bit Android on 64-bit-capable chips. `docs/architecture/android.md`
records all three song kinds playing on one of them at 59.5 fps, with the owner's page reachable over
the network. Either device on an HDMI input puts this machine on a Samsung television for no work.
The stream in §6 does it with no second device, when a machine already runs elsewhere.

## 9. Recommendation

Build nothing yet. If a native Samsung machine is wanted, these measurements decide it, in order:

1. ~~A `.tpk` that listens on `0.0.0.0` and answers a phone.~~ **Passed** on 2026-11-17; see §2.
2. ~~Sound.~~ **Passed** on 2026-11-17 through `audio_io`, not ALSA; see §3.
3. ~~The remote's keys.~~ **Passed** on 2026-11-17; see §3.
4. ~~SDL3 drawing into the host application's own window at a steady 60 fps, watched on the set.~~
   **Passed** on 2026-11-17, as a subsurface driven from the host's main loop; see §3. A window of
   its own does not stay on screen.
5. An installed package starting after Developer Mode has expired: leave the set untouched for ten days
   after the last install, then start it.

A platform decision belongs in `docs/decisions/` after them, not before.

## Sources

- Samsung's smart television developer documentation: the NaCl overview and its deprecation notice,
  the Tizen .NET TV framework, the Tizen Sockets Extension, and file handling.
- `iqui27/nuvio-native-legacy` on GitHub: its README and release notes, `src/tpk.c`,
  `src/dts/dts_hls_tpk.c`, `tizen-tpk/README.md`, and its notes on the `.tpk` build.
- `Apps2Samsung/Apps2Samsung` on GitHub, and its issue #844 on installing from the television.
- The Samsung developer forum thread "Native app in C++?".
- Community reports on Developer Mode sideloading and how long a grant lasts.
- SDL 3.4.14's `src/video/wayland/SDL_waylandvideo.c`.
- `docs/architecture/android.md`, `docs/architecture/display.md`, `docs/decisions/foundations.md`,
  `docs/decisions/streaming.md` and `crates/machine/km-api/src/routes.rs` in this repository.
