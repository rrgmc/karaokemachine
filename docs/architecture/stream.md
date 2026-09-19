# The stream

> Part of the [architecture notes](../ARCHITECTURE.md). The product decisions are
> [`docs/decisions/streaming.md`](../decisions/streaming.md); this file says how the thing is built.

A drawn screen and the sound beside it are encoded into one HLS stream, and a television somewhere
else plays it. `km-stream` is the pipeline, and `km_api::watch` serves what it writes. Neither knows
what a song is.

## `km-stream` — the pipeline

**It knows nothing about karaoke.** `km-video` makes the same division in the other direction. What
arrives is a drawn screen as packed BGRA and stereo samples as `f32`. What leaves is a rolling
playlist and its segments in a directory. `examples/synthetic.rs` drives the whole of it from a
generated test pattern. So every part can be exercised with no machine, no catalog and no screen.

**Two crates name ffmpeg now, split by direction.** `km-video` decodes a song's own picture into
planes bound for a texture; this encodes the screen the machine drew. Neither takes the other, and
each carries its own `ffmpeg` feature, so a build wanting one does not compile the other.

**`pixels` is outside that feature and is tested without ffmpeg.** Converting a screen into the
planes an encoder wants needs no encoder. A colour matrix is exactly the thing worth catching on a
machine that cannot run the rest. Every way of getting it wrong produces a picture that is plausible
rather than broken.

### Why the conversion is written out

`tools/setup/ffmpeg-pin.sh` configures `--disable-swscale`, one of four sublibraries switched off
because they reach for ffmpeg's optional external dependencies. So there is no library call that
turns a packed picture into planes.

**It costs nothing to keep it that way, because the screen is drawn at the size it is encoded at.**
Scaling is what swscale would really have been wanted for, and there is none to do.

The matrix is Rec. 709 at limited range, and **the encoder is told the same thing**. A picture
converted one way and tagged the other comes out washed or crushed. That reads as a display problem
rather than a conversion one.

## What a client gets

```
/stream/live.m3u8    the stream. This is the interface.
/stream/seg-*.m4s    the segments it names, and init.mp4 before them
/watch/              a page, for the sets where opening a URL is what is easiest
```

**The playlist is the product and the page is a convenience**, which is
[`A television somewhere else is a stream, and the playlist is the interface`](../decisions/streaming.md#a-television-somewhere-else-is-a-stream-and-the-playlist-is-the-interface).
Anything that follows a URL plays the stream without knowing a page or a browser exists. That
includes a television's own media pipeline, VLC, Kodi and a player on a set-top box. The page is
where `hls.js` fills the one gap. HLS is native on smart televisions, Safari, iOS and Android, and
absent from desktop Chrome and Firefox.

Files also make this an ordinary request and response, rather than a body held open for the length
of a song. `handlers.rs` already declines to hold one open on a machine whose other job is playing
audio.

## Four things that are silent when they are wrong

Each of these produces a stream that looks right from the machine's side and plays nothing.

- **A muxer flagged `AVFMT_NOFILE` opens its own files**, and `ffmpeg_next::format::output_as` opens
  one anyway. It does not test the flag, where `output_to_stream` beside it does and refuses
  outright. So the HLS muxer gets a playlist this process is holding open. On Windows its rename onto
  that name fails with nothing reported. `release_playlist_handle` undoes it, and is the workspace's
  thirty-ninth `unsafe` exception.
- **A path with both kinds of separator** comes from joining a playlist name onto a directory
  somebody typed with forward slashes. The muxer keeps only what precedes the last separator it
  recognises. So the initialisation segment every client fetches first lands one directory above the
  playlist naming it.
- **`hls_fmp4_init_filename` is a bare name and `hls_segment_filename` is a path**, and they are not
  interchangeable. The muxer joins the first to the playlist's directory and writes the second as
  given. So it refuses an absolute path in the first as a doubled path. A bare name in the second
  lands in the working directory.
- **An encoder need not fill in a packet's duration** and libopenh264 does not. That leaves the muxer
  inferring segment boundaries from the gap to the next packet, and cutting a frame late. One tick,
  the encoder's time base being one tick per frame.

## What it costs, and the one thing that decided it

Measured on the idle screen, **debug build**, by comparing the playlist's media sequence against the
wall clock. Each segment is two seconds of stream, so real time is where the two agree.

| | Rate against real time |
|---|---|
| 1920x1080, wallpaper uploaded every frame | 0.33x |
| 1920x1080, uploaded only when it changes | 0.50x |
| 1280x720, uploaded only when it changes | 0.75x |
| 1280x720, **and `km-stream` at `opt-level = 2`** | **1.00x** |
| 1920x1080, the same | **1.00x** |

**The conversion being unoptimized was the whole of it**, and it is invisible from the outside. Every
other piece of pixel work in this path is C, compiled optimized whatever profile the workspace uses:
SDL's software renderer, and libopenh264 inside ffmpeg. So `km-stream` joins `image`, `rustysynth`
and `argon2` in `[profile.dev.package]`, for the reason all three are there.

**A stream that runs slow is not a slow stream.** It is one whose sound will arrive faster than its
pictures.

**Uploading a wallpaper is not free and must not be per-frame.** `Offscreen::set_picture` converts
the whole image and builds a texture from it. At 1080p that is megabytes of work. It is right for a
picture that has changed, and ruinous thirty times a second for one that stands for minutes. Each
source numbers its pictures and the loop uploads only when the number moves.

## The sound, and why it needs no clock

**A streaming run opens no device**, so `Engine::streaming` starts no audio thread either. A sound
card pulls blocks on its own schedule, and a thread exists to serve it. Here the stream loop pulls
them, and whoever is rendering drains the jobs the engine's handle sends. Nothing else about the
engine changes. Commands travel as the same `Job`s down the same channel.

**`km_audio::Renderer` is `OutputStream`'s counterpart**, and the two share `fill_and_publish`. So a
stream feeding a sound card and one feeding an encoder cannot come to disagree about what a block
means. That one place writes everything a reader outside sees. That is the position the screen draws
from, the transport the queue watches, and the count of songs that ended.

**Nothing in the rendered path is real-time**, which is the other half of the difference. There is no
callback deadline to miss, so commands arrive on an ordinary channel. A retired song is dropped where
it is found, rather than handed to another thread to free.

**One frame's worth of sound per frame is the whole of the synchronisation.** The encoder numbers a
picture by the count of frames and a packet by the count of samples. So rendering exactly
`sample_rate / fps` samples for every frame drawn puts the words on the beat by arithmetic. A rate the
frame rate does not divide would leave a remainder every frame, and the two would creep apart. 48 kHz
over 30, 25 or 60 divides exactly.

**A bank change waits for the deck to empty**, whatever `close` asked for. On the device path, a swap
that asks to be heard at once drops the stream and takes the playing song with it. The machine then
sends the song again. Here that would be a stream that stopped mid-song in front of people, with no
way to see why. The wait is at most one song.

## The icon in the bar, and which thread runs what

**A streaming machine is otherwise invisible.** It draws on no television and opens no window. So on
a desktop nothing says it is there and nothing says where to watch it. Nothing but Task Manager stops
it. The `tray` feature gives it the icon that `km-package-builder` and `km-remote` already give a
server that gets out of the way.

The tooltip holds the address, and the menu holds *Remote*, *Watch*, *Setup* and *Quit*.

**The two platforms take different pictures, and the bar each hangs them in is why.** Windows reads
the second icon resource out of the executable. `Spec::icon_ordinal` carries it and `build.rs`
attaches it beside the first. It is the badged mark, standing among other full-color icons. At the
16 pixels the notification area asks for, the badge is an amber corner rather than a glyph.

macOS decodes `karaokemachine-bar.png`: the letters with the tile taken off and no badge on them. A menu
bar draws template images that the system colors itself.

Neither wears the plain machine's mark, because a machine drawing a television opens no tray at all.
See
[`A badge says how the machine was started`](../decisions/interface.md#a-badge-says-how-the-machine-was-started)
and
[`The icon in a macOS menu bar is a silhouette, and the run behind it takes no Dock tile`](../decisions/interface.md#the-icon-in-a-macos-menu-bar-is-a-silhouette-and-the-run-behind-it-takes-no-dock-tile).

**The macOS launcher `open`s the machine's bundle rather than exec'ing its binary**, and the icon is
why. A process that execs from one bundle into another carries two identities. One is the identity
LaunchServices launched, and the other is the one its image belongs to. `NSStatusBar` returns a
status item created under that disagreement and never draws it, with no error anywhere.

Opening the bundle also settles what `exec` was there for. Launched this way, the launcher is not in
the machine's own path at all. So `current_exe()` names the real binary in the real bundle, and the asset lookup lands where
it should.

**And on macOS the run drops its Dock tile, in the same breath as taking the icon.** `tray.rs` asks
the event loop's window target for `ActivationPolicy::Accessory` at the one moment it is safe to. That
moment is when `build` has returned a `Tray`. So a machine whose icon failed keeps a tile, and a
person can still find it. A manifest key could not express that, and nothing would read it anyway:
the streaming launcher hands over to the machine's own bundle.

**Three entries where the tools have one, because this run serves three pages**: the singer's remote
at the root, the stream's page, and the owner's. The three are one value in `tray.rs`. It builds them
together from the base the API reports and the paths `km-api` names. So the menu and the click cannot
come to disagree. Each is spelled the way the router answers rather than the way it redirects:
`/watch/` with the trailing slash, `/admin` without one.

**The base is `connect::reachable_url`, and the loop asks for it again.** It is the address a phone
can route to, rather than the one this box reaches itself by. See
[`The machine's icon names the address a phone can reach`](../decisions/interface.md#the-machines-icon-names-the-address-a-phone-can-reach).
That address moves under a run that lasts an evening.

There are two cadences, each proportional to what it waits for. `LOOK` compares two flags five times a second. `ASK_THE_ADDRESS` takes the read
lock behind `ApiState::connect_info` every two seconds, chasing a fact `CONNECT_REFRESH` moves every
thirty. `km_tray::Tray::set_url` writes the new one into the menu's label and the tooltip together.
That is why `Tray` keeps that `MenuItem` and the product's name.

**The event loop belongs to the main thread and the stream does not.** An icon can be created only on
a thread running the platform's loop. Drawing and encoding need no particular thread, only to be left
alone. So the stream goes to a thread of its own. It does whether or not the feature is on, which
keeps one shape rather than two.

**`run_return` and never `run`.** `EventLoop::run` diverges, ending the process where it stands. And
everything the machine does on the way out happens after the loop returns. The API is asked to stop,
and the watchdog gets its budget. `persist` writes down what somebody changed from a phone. Quitting
from an icon would otherwise throw all three away: silently, and only for the people who used the
icon.

**Windows and macOS stage it; the two Linux carriers never mention it.** `tray-icon`'s Linux backend
links `libayatana-appindicator` at load time. `The package builder's window` already refuses that
dependency for `wry`. So the two staging scripts name the feature, rather than anywhere a workspace
build reads. It rides with `video` because the run that wants an icon is the streaming one.

## The encoder is named, not chosen

`Config::encoder` is an ffmpeg encoder name. To try a hardware one, set its name and measure, with no
change to any code. A name the build does not have is reported when the stream is opened. It is not
left until the first segment fails to appear.

**This is not the automatic choice `Which H.264 encoder` refuses.** That decision is about a
packaging run picking an encoder for itself. There, a missing graphics card becomes a failure in the
middle of a batch. Naming one deliberately, on a machine somebody is configuring, is a different act.

### `auto`, and the one thing a name cannot know

> Which encoder travels with the machine, and on what terms, is
> [`A streamed screen carries its own H.264 encoder`](../decisions/distribution.md#a-streamed-screen-carries-its-own-h264-encoder).

**ffmpeg implements no H.264 encoder of its own, and whoever configured it decides which external one
it carries.** An ffmpeg built here carries `libopenh264`, the only H.264 encoder an LGPL configure
line may have. A distribution builds `--enable-gpl` and carries `libx264` instead. Debian's carries
no openh264 at all. So the same written name cannot be right on both. A machine installed from the
`.deb` would start and say it had no encoder.

So the default is `auto`, which takes the first of `SOFTWARE_ENCODERS` the build has. **Only `auto`
searches.** A name somebody wrote down resolves to itself or to nothing. A person who set
`h264_nvenc` and silently got software H.264 has been told their graphics card is working when it is
not.

The two are the same codec by two implementations, either simply present or absent. That separates
this from the choice `Which H.264 encoder` refuses. Nothing here depends on hardware, so nothing here
can fail in the middle of a batch for want of it.

## Drawing the screen

`km_display::Offscreen` keeps a surface, a canvas, a text cache and the picture behind the frame
across frames. It reads the drawn pixels straight out of the canvas's own surface without copying
them. `render_to_image` is a thin wrapper over it for the callers that want one picture.

**The text cache is why the type exists.** Rendered text is the largest single cost in drawing a
screen. It took the windowed display from 88% of a core to 10.7%. A cache built and dropped per frame
never serves a hit.

It needs no video subsystem: `sdl3::ttf::init()` and `Fonts::discover` are the whole of the setup.
That is why it runs where CI does, and over SSH on a box with no monitor.

**So a streaming run is also the cheapest proof that the screen draws at all on a platform.** It
needs no display server, no X and no Wayland. Start the machine with `--stream`, fetch
`/stream/live.m3u8` and decode one frame. That exercises the fonts, the CJK fallback, the layout, the
wallpaper and every branch of `draw()`. What that does *not* say is which backend SDL picks for a
**window**. That is a different question, and it wants a real session to answer.
