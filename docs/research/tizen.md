# Research: the machine on a Samsung Tizen television

**Research only. Nothing implemented, nothing decided.** A platform is a product decision, so a
Tizen carrier would need an entry in `docs/decisions/distribution.md` beside
[`What the machine *is*, on Linux`](../decisions/distribution.md#what-the-machine-is-on-linux) and
[`What the machine *is*, on iOS`](../decisions/distribution.md#what-the-machine-is-on-ios) first.

Investigated 2026-09-13. **No claim here was verified on a television**, and the marker on each says
how far it can be trusted. The sibling note is [`webos.md`](webos.md), which asks the same question
of an LG television and gets a different answer.

**Summary.** A Samsung port in the shape the four existing ports have is not available. Tizen
television applications are web applications, and native C and C++ was reachable only through NaCl.
Samsung supports NaCl through 2021 products and replaced it with WebAssembly from 2020 products.
SDL3, cpal and ffmpeg therefore have nowhere to run. The display, the audio path and the video path
would each need a browser backend rather than a port.

Two further findings settle what is left. **An application on a Samsung television cannot listen on
a TCP port**, so the machine could not serve its API to a phone. It would lose its whole control
surface.
And a sideloaded application expires every seven days. The engine's pure-Rust core would compile to
WebAssembly, and the file access is better than an LG television's. So the obstacles are the display,
the API and the carrier rather than the songs.

**A Google TV device on HDMI already puts this machine on a Samsung television, and the Android port
already builds for it.**

| Marker | Meaning |
|---|---|
| **[repo]** | Read from this repository. High confidence. |
| **[source]** | Read from vendor documentation. High confidence. |
| **[web]** | Public documentation and community reports. Second-hand. |
| **[inferred]** | Reasoning. A hypothesis to test. |

## 1. What a Tizen television permits

**[source]** Samsung's own NaCl documentation states the timeline:

> "Due to NaCl deprecation by the Chromium project, Tizen TV will continue its support for NaCl only
> until 2021-year products, while Tizen TV will start focusing on high-performance, cross-browser
> WebAssembly from 2020-year products."

So the one route that ever ran an ARM binary from a third party is closed on every television sold
since. WebAssembly inside a web application is the replacement.

**[web]** Tizen .NET for televisions exists and reaches Tizen 4.0 products, so it covers neither
current hardware nor most of the installed base. Tizen 8.0 is what current televisions run, and
JavaScript against the Tizen Web API is the practical standard for new work.

**[inferred]** The `ports/machine/android` pattern therefore has no Tizen equivalent. Android ships
`libkm_app.so` and lets SDL call `SDL_main`; iOS links the same library into a Swift application.
Neither shape exists where the application is a web page.

## 2. The machine cannot be a server there

**[web]** A Samsung television application cannot listen for incoming TCP connections. The Tizen
Sockets Extension gives WebAssembly low-level socket access and reaches Tizen 5.5 products, and
native applications are documented as unusable as servers. A web application has WebSocket and
`XMLHttpRequest`, both of which connect outward.

**[repo]** That is fatal to the product rather than inconvenient. The machine's control surface **is**
an HTTP API, and `km_api::routes::SURFACE` holds 57 routes. The machine serves the singer's remote at
`/` to any browser on the network, and every search, queue and setting happens through it.
`docs/decisions/foundations.md` puts the rule plainly, that the display shows the singer screen and
song-number entry only. A karaoke machine nobody can queue a song on from a phone is a different
product.

**[inferred]** The reverse direction survives. A television application can hold a WebSocket open to
a machine running somewhere else, which is what §6 is about.

## 3. What would port, and what would not

**[repo]** The pure-Rust core is the part that travels, and the layering says which crates those are.

| | |
|---|---|
| `km-song`, `km-suitability`, `km-fixes`, `km-songcode`, `km-queue`, `km-songbook` | pure Rust with no C and no OS. Compile to WebAssembly unchanged |
| `km-cdg` | pure Rust over `symphonia`, with no feature to turn on. Travels |
| `km-catalog` | `rusqlite` with SQLite bundled, so the C compiles, and a browser has no file to open. Would need its storage rewritten |
| `km-audio` | `rustysynth` is pure Rust and travels; cpal does not. Output would be Web Audio, and the sequencer would run in an audio worklet |
| `km-video` | ffmpeg has nowhere to run. A video song would play through an HTML video element, which changes what a song *is* to the rest of the machine |
| `km-display` | SDL3 throughout, and `docs/architecture/display.md` states there is no renderer seam. A canvas or WebGL renderer is a rewrite of the largest thing in the project |
| `km-api` | cannot exist, per §2 |

**[repo]** Three files inside `km-display` are already pure logic with no SDL in them. They hold
which lyric lines are visible and how far the wipe has crossed, the number formatting, and the
keypad's geometry. Those would travel. The 4,810-line drawing file would not.

**[inferred]** The honest description is a second implementation of the machine sharing a song
parser, not a port. The Android port needed changes in five files. This needs new backends for the
display, the audio and the video, and a replacement for the API.

## 4. Songs and files, which are the easy part

**[source]** Samsung's file handling documentation says an application reads external USB storage
through the FileSystem API. Writing needs the `filesystem.write` privilege declared in `config.xml`.

**[inferred]** This is better than an LG television gives a web application, where files can be read
only from inside the application package. On the evidence, a Tizen application could find a karaoke
library on a stick and read it. Songs are therefore not what stops a Samsung port; §2 and the display
are.

## 5. The carrier expires every seven days

**[web]** Sideloading onto a retail Samsung television runs through developer mode, and the
developer-mode grant expires every seven days and breaks on a firmware update. Signing certificates
are separately valid for about a year and have to be reissued after a firmware update.

**[inferred]** Seven days is a different order of problem from an LG television's 1000 hours. It
rules out an appliance somebody switches on in a room without thinking about it, whatever happens to
the technical obstacles above.

## 6. The television as a screen for a machine elsewhere

**[repo]** A different product is available and the API already has most of it. A client fetches
`GET /api/v1/songs/{number}/lyrics`, which returns lines carrying `start_ms`, `end_ms` **and their
syllables**. It follows `GET /api/v1/events` over a WebSocket for `state` at 4 Hz, `song_started`,
`song_ended` and `lyric_line`. Per-syllable position is deliberately never streamed: a client
interpolates locally against its own clock, and `km-api`'s events module exists to enforce that. A
television application can hold that WebSocket open, which §2 permits.

**[repo]** What stops it is that **nothing serves media bytes**. `SURFACE` has no audio or video
stream route; the `/audio/*` routes manage devices and SoundFont banks. `km-video` decodes to YUV
planes and hands them straight to a texture in-process. `km-cdg` renders its tile plane the same
way. So a video song and an MP3+G song have no representation a remote screen could draw. Two of the
three song kinds would be blank.

**[inferred]** Sound is the second problem. The hardware mixes microphone audio into the amplifier
the machine plays into. A television that draws words while the audio comes out of a different box
has to stay in time with it across the network. A karaoke screen is unforgiving about that.

## 7. Against the LG answer

| | Samsung, Tizen | LG, webOS |
|---|---|---|
| Native ARM binary from a third party | closed with NaCl | unofficial, and installs on a stock television |
| SDL | unavailable at any version | SDL2 has a community fork; SDL3 has nothing |
| The machine's HTTP API | cannot listen | ordinary sockets |
| Reading songs off a stick | FileSystem API, with a privilege | a directory walk, subject to the jailer |
| Sideload lifetime | seven days | 1000 hours |
| Verdict | a second implementation, minus the API | a port, gated on the display |

## 8. What answers the question today

**[repo]** `.cargo/config.toml` names the Google TV Streamer and Chromecast with Google TV, and
armv7 is built because both run a 32-bit Android on 64-bit-capable chips.
`docs/architecture/android.md` records all three song kinds playing on one of them at 59.5 fps, with
the owner's page reachable over the network. Either device on an HDMI input puts this machine on a
Samsung television, with the remote, the API and video, for no work at all.

## 9. Recommendation

Build nothing, and keep the finding so the question does not come back. The three obstacles are
independent, and two of them are decisions Samsung has already taken: no native code, and no
listening socket. Neither is waiting on effort here.

Somebody may want a Samsung television as a **screen** rather than as the machine. Then the work is
in this repository rather than on the television. The API would have to serve what a remote screen
needs for a video song and an MP3+G song. The clock across two boxes would have to be good enough to
keep words on the beat. That is a product decision about what the API is for, and it
belongs in `docs/decisions/` before any of it is built.

## Sources

- Samsung's smart television developer documentation. That covers the NaCl overview and its
  deprecation notice and the Tizen .NET TV framework. It also covers the Tizen Sockets Extension and
  the data and file handling questions.
- Tizen's own web application documentation, for what a web application's networking can do.
- Community reports on developer-mode sideloading and how long a grant lasts.
- `docs/architecture/android.md`, `docs/architecture/display.md`, `docs/decisions/foundations.md`
  and `crates/machine/km-api/src/routes.rs` in this repository.
