# Research: the machine on a Samsung Tizen television

**Research only. Nothing implemented, nothing decided.** A platform is a product decision, so a
Tizen carrier would need an entry in `docs/decisions/distribution.md` beside
[`What the machine *is*, on Linux`](../decisions/distribution.md#what-the-machine-is-on-linux) and
[`What the machine *is*, on iOS`](../decisions/distribution.md#what-the-machine-is-on-ios) first.

Investigated 2026-09-13 and revised 2026-11-17. **Four claims were measured on a television**, a 2025
set running Tizen 10: §2's listening port, and §3's SDL3 window, sound and frame pacing. The marker on
each of the others says how far it can be trusted. The sibling note is [`webos.md`](webos.md), which asks the same question
of an LG television.

**Summary.** The first version of this note said a native port was impossible, because Samsung
dropped NaCl, the only route for third-party native code. That was wrong. A Tizen application package
(`.tpk`) is a .NET application, and a .NET application can load a C library. **A port in the shape of
the Android one is possible, and most of its gates are measured open.** An application on the television
accepts connections from the network. SDL3's own Wayland driver draws a full-screen frame with its
GLES2 renderer, after two one-line patches. Sound plays through Tizen's native `audio_io`, though not
through ALSA. Frames are paced to the display by Tizen's `tdm` vblank, not by SDL3's vsync, at 59 fps
with about one frame in fourteen held for two vblanks. What is left unmeasured is the remote's keys.

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

**[measured]** Frame pacing comes from Tizen's display manager, not from SDL3. With nothing pacing it,
the probe presented about 465 frames a second. Four ways were tried, each for 16 s, timing every
present:

| Pacing | Result |
|---|---|
| SDL3's vsync, unpatched | 0.3 fps. SDL sizes its wait for the frame callback from the refresh, and 0.06 Hz makes it 16.7 s |
| SDL3's vsync, refresh patched | 26 fps, uneven. Frame callbacks rarely arrive, so most swaps fall back on SDL's 50 ms timeout |
| `eglSwapInterval(1)` on the set's EGL, SDL's vsync off | accepted and ignored: about 4,000 presents a second |
| `libtdm-client`'s vblank wait, synchronous, then present with SDL's vsync off | **58.9 fps.** Vblank timestamps step 16.6 ms, and per five seconds about 20 of 295 frames are held for two vblanks, with about 10 short ones catching up |

**[inferred]** `tdm` is what the platform's own toolkits pace with, and the client library is already
loaded into every application. A Tizen `km-display` would wait on it before each present and turn
SDL's vsync off. Why one frame in fourteen misses its vblank is open. The probe drew almost nothing,
so it is not render time; the present after the wait blocking on a buffer the compositor has not
released is the likely cause. A moving wipe would show it as an occasional small stutter.

**[measured]** What this run does not settle:

- **The remote's keys.** No key event was looked for.
- **4K.** The output was 1920×1080, the size the compositor gave. Whether a native application may
  draw at the panel's 3840×2160 is open.

**[repo]** The layering says which crates travel.

| | |
|---|---|
| `km-song`, `km-suitability`, `km-fixes`, `km-songcode`, `km-queue`, `km-songbook`, `km-cdg` | pure Rust. Cross-compile to armv7 Linux unchanged |
| `km-catalog` | SQLite bundled. Builds; the database lives in the application's data folder |
| `km-api` | axum over tokio. Builds; §2 measured the network reaching it |
| `km-display` | SDL3 with its own Wayland driver, measured above. Needs a custom-role window path, the two patches, and `tdm` pacing |
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
| SDL | SDL3's Wayland driver draws, measured on Tizen 10, with a patch and a custom-role window | SDL2 has a community fork; SDL3 has nothing |
| The machine's HTTP API | reachable from the network, measured on Tizen 10 | ordinary sockets |
| Reading songs off a stick | unmeasured for a native package | a directory walk, subject to the jailer |
| Sideload lifetime | seven days reported; effect on an installed package unknown | 1000 hours |
| Sound | Tizen's `audio_io`, measured; ALSA refused | unmeasured |
| Frame pacing | Tizen's `tdm` vblank, 59 fps with about 7% held a vblank, measured | unmeasured |
| Verdict | a port; the remote's keys and the install's lifetime are unmeasured | a port, gated on the display |

## 8. What answers the question today

**[repo]** `.cargo/config.toml` names the Google TV Streamer and Chromecast with Google TV, and armv7
is built because both run a 32-bit Android on 64-bit-capable chips. `docs/architecture/android.md`
records all three song kinds playing on one of them at 59.5 fps, with the owner's page reachable over
the network. Either device on an HDMI input puts this machine on a Samsung television for no work.
The stream in §6 does it with no second device, when a machine already runs elsewhere.

## 9. Recommendation

Build nothing yet. If a native Samsung machine is wanted, these measurements decide it, in order:

1. ~~A `.tpk` that listens on `0.0.0.0` and answers a phone.~~ **Passed** on 2026-11-17; see §2.
2. ~~SDL3 drawing a frame.~~ **Passed** on 2026-11-17, with the patch and the custom-role window; see §3.
3. ~~Sound.~~ **Passed** on 2026-11-17 through `audio_io`, not ALSA; see §3.
4. ~~Presentation paced to the display's 60 Hz.~~ **Passed** on 2026-11-17 through `tdm`, with about
   7% of frames held a vblank; see §3. The remote's keys reaching SDL is still open.
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
