# Research: the machine on a Samsung Tizen television

**Research only. Nothing implemented, nothing decided.** A platform is a product decision, so a
Tizen carrier would need an entry in `docs/decisions/distribution.md` beside
[`What the machine *is*, on Linux`](../decisions/distribution.md#what-the-machine-is-on-linux) and
[`What the machine *is*, on iOS`](../decisions/distribution.md#what-the-machine-is-on-ios) first.

Investigated 2026-09-13 and revised 2026-11-17. **One claim was measured on a television**: §2's
listening port, on a 2025 set running Tizen 9. The marker on each of the others says how far it can be
trusted. The sibling note is [`webos.md`](webos.md), which
asks the same question of an LG television.

**Summary.** The first version of this note said a native port was impossible, because Samsung
dropped NaCl, the only route for third-party native code. That was wrong. A Tizen application package
(`.tpk`) is a .NET application, and a .NET application can load a C library and lend it an OpenGL ES
context. An open-source client in C and SDL2 ships this way for Tizen 4.0 to 9, and is installed with
an ordinary Public certificate. **A port in the shape of the Android one is therefore possible.** An
application on the television accepts connections from the network, measured. The port is gated on two
things not yet measured: drawing through SDL3, and playing PCM.

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

## 2. Listening on a port: the network reaches it

**[source]** The same client calls `bind`, `listen` and `accept` on the television. It binds
`127.0.0.1` only, because the client is the television's own media player. So a native application
**can** listen, which contradicts the community reports the first version of this note relied on.

**[inferred]** Whether the network can reach it mattered more than anything else here. The machine's
control surface **is** its HTTP API: `km_api::routes::SURFACE` holds every route, and the singer's
remote is served at `/`. A machine nobody can queue a song on from a phone is a different product.

**[measured]** It can. On 2026-11-17 a probe was installed with Apps2Samsung on a 2025 television
running Tizen 9. The probe was a .NET-only `.tpk` (`net6.0-tizen8.0`, manifest API version 8, `tv`
profile, privileges `internet` and `network.get`) whose `TcpListener` bound `0.0.0.0:8177`. The bind
succeeded. A phone and a PC on the same network each fetched a page from it, and the probe showed each
request with the sender's address. No prompt appeared on the television, and no setting was changed.

**[inferred]** The probe was C#, and the machine's API would be Rust in a native library. Both reach
the same kernel socket calls under the same application, so the result should carry. It says nothing
about the multicast socket mDNS needs. Without mDNS, the remote finds the machine by its address or
by the QR code.

## 3. What would port, and what would not

**[repo]** The layering says which crates travel.

| | |
|---|---|
| `km-song`, `km-suitability`, `km-fixes`, `km-songcode`, `km-queue`, `km-songbook`, `km-cdg` | pure Rust. Cross-compile to armv7 Linux unchanged |
| `km-catalog` | SQLite bundled. Builds; the database lives in the application's data folder |
| `km-api` | axum over tokio. Builds; §2 measured the network reaching a port on the television |
| `km-audio` | `rustysynth` travels. cpal has no Tizen backend, so output would go through Tizen's native `audio_io` or a .NET `AudioPlayback` the host feeds. Neither has been seen working from a third-party package |
| `km-video` | ffmpeg cross-compiles. Software decoding of 1080p on a television's processor is unmeasured. The alternative is the television's player on its video plane, as the client in §1 does, which changes how a video song is composited |
| `km-display` | SDL3 throughout, with no renderer seam. SDL3 has no Tizen driver, so it has to draw into a context it did not create |

**[inferred]** `km-display` is the gate, as on webOS. SDL3's renderer owns its window and context.
The client in §1 avoids the question by using SDL only as an API over GLES. Whether SDL3 can be given
an external EGL context, or needs a small video driver of its own, is the second measurement. The
Android port needed changes in five files; this needs a host application, an audio backend and that
driver on top.

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
| SDL | none; SDL2's API over the host's EGL has been shown | SDL2 has a community fork; SDL3 has nothing |
| The machine's HTTP API | reachable from the network, measured on Tizen 9 | ordinary sockets |
| Reading songs off a stick | unmeasured for a native package | a directory walk, subject to the jailer |
| Sideload lifetime | seven days reported; effect on an installed package unknown | 1000 hours |
| Verdict | a port, gated on the display and the audio | a port, gated on the display |

## 8. What answers the question today

**[repo]** `.cargo/config.toml` names the Google TV Streamer and Chromecast with Google TV, and armv7
is built because both run a 32-bit Android on 64-bit-capable chips. `docs/architecture/android.md`
records all three song kinds playing on one of them at 59.5 fps, with the owner's page reachable over
the network. Either device on an HDMI input puts this machine on a Samsung television for no work.
The stream in §6 does it with no second device, when a machine already runs elsewhere.

## 9. Recommendation

Build nothing yet. If a native Samsung machine is wanted, three measurements decide it, in order,
each a day or less:

1. ~~A `.tpk` that listens on `0.0.0.0` and answers a phone.~~ **Passed** on 2026-11-17; see §2.
2. SDL3 drawing one frame into the host's EGL context.
3. An installed package starting after Developer Mode has expired. The probe from §2 is that package,
   installed on 2026-11-17. Starting it after 2026-11-27 answers this.

A platform decision belongs in `docs/decisions/` after them, not before.

## Sources

- Samsung's smart television developer documentation: the NaCl overview and its deprecation notice,
  the Tizen .NET TV framework, the Tizen Sockets Extension, and file handling.
- `iqui27/nuvio-native-legacy` on GitHub: its README and release notes, `src/tpk.c`,
  `src/dts/dts_hls_tpk.c`, `tizen-tpk/README.md`, and its notes on the `.tpk` build.
- `Apps2Samsung/Apps2Samsung` on GitHub, and its issue #844 on installing from the television.
- The Samsung developer forum thread "Native app in C++?".
- Community reports on Developer Mode sideloading and how long a grant lasts.
- `docs/architecture/android.md`, `docs/architecture/display.md`, `docs/decisions/foundations.md`,
  `docs/decisions/streaming.md` and `crates/machine/km-api/src/routes.rs` in this repository.
