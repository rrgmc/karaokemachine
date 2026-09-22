# Streaming

> Product decisions, each with the reasoning that produced it -- what the product must do, and why
> it is that way. Part of [`docs/decisions/`](README.md); how the thing is built is in
> [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

## A television somewhere else is a stream, and the playlist is the interface

**The machine can draw its screen for an encoder instead of for a television, and serve the result as
one continuous HLS stream.** Picture, words and sound, for every kind of song, at one address.

`docs/research/tizen.md` found the gap and said where the answer belonged. It said:

> *"the API would have to serve what a remote screen needs for a video song and an MP3+G song… That
> is a product decision about what the API is for, and it belongs in `docs/decisions/` before any of
> it is built."*

This is that decision.

**The playlist is the product; the page above it is a convenience.** Anything able to follow a URL
plays `/stream/live.m3u8`. That includes a television's own media pipeline, VLC, Kodi and a player
on a set-top box. None of them needs a page, a browser or any knowledge that this project exists.
That makes "as many clients as possible" achievable instead of a pile of per-device work. It is also
why the page at `/watch/` must never become the only way in.

**The playlist is HLS, because the clients are televisions.** A set's own player and its browser both
play a playlist through the television's pipeline. Files make this ordinary request-and-response, and
nothing holds a playlist request open. The page has a faster path beside it, which
[`The page takes the stream over a WebSocket, and the playlist stays the interface`](#the-page-takes-the-stream-over-a-websocket-and-the-playlist-stays-the-interface)
records.

**Being behind costs nothing for singing, and it costs control.** The stream is the only screen and
the only sound in the room it is watched in. So it is in time with itself, and nobody is chasing a
television. What the delay costs is control: pause, and the music runs on for the length of the
buffer. Segment length is the dial, and
[`The stream runs as close to live as plain HLS allows`](#the-stream-runs-as-close-to-live-as-plain-hls-allows)
sets it.

**Every song kind is composited, and a video song is re-encoded with the rest.** Serving a video
song's packaged bytes untouched would be better quality, and it was refused for two reasons. It
forces a discontinuity mid-playlist, which is what simpler players handle worst. Or it moves
source-switching into the page and ends the playlist being the interface. On one song kind out of three,
four things would vanish: the progress bar, the queue count, the next-song banner and the connect
panel. That is worse than the generation of encoding it saves.

**It carries the backing track and never a singer.** Mic audio is mixed in hardware, downstream of
anything this process can see. So what a stream can publish is what a package holds.

**Public, on the same terms as reading the queue.** It sits outside `/api/v1/admin/`, so no token is
asked for. That is the judgment [`Network reach`](api-and-network.md#network-reach) already makes
about a home LAN indoors. Watching is the
[`view level`](api-and-network.md#four-access-levels-and-the-method-and-path-decide-them), which
every phone in the room holds whatever the room level is.

**A machine that is not streaming has neither path**, rather than a page explaining itself. That is
[`Power is a capability of the host`](api-and-network.md#power-is-a-capability-of-the-host-not-a-method-on-the-machine)
applied again: a route that is not mounted spells absence.

## The stream runs as close to live as plain HLS allows

**Segments are one second long by default, because a shorter delay is better.** A client plays two
or three segments behind the newest one. So a television runs about three seconds behind the machine.
Pause, skip and a newly queued song reach its screen that much sooner.

**One second is the floor.** A playlist states segment length in whole seconds. Anything shorter is
Low-Latency HLS, which holds a playlist request open until the next part exists. The entry above
refuses exactly that request.

**The players this serves play one-second segments.** hls.js, Safari, iOS, Android, VLC and Kodi all
do. The risk is an older television's own player, which may stop to buffer on a busy network.
`stream.segment_seconds` set to `2` is the answer for that set, and that is why it stays a setting.

**The playlist names twelve segments**, so it holds twelve seconds of stream. A television that falls
behind still finds the segment it wants next, rather than one the muxer has deleted.

**A keyframe starts every segment, and the machine forces it.** An encoder's keyframe interval is only
a ceiling. `libx264` adds a keyframe where the picture changes sharply, and counts the next interval
from there. Segments then run long and the playlist's target duration rounds up to two seconds. A
player sets its distance from the live edge from that number.

**A settings file that already names a segment length keeps it.** A changed default is not a new
settings version, as
[`What the machine does when nobody is singing`](interface.md#what-the-machine-does-when-nobody-is-singing)
argues for the demo delay.

## The page takes the stream over a WebSocket, and the playlist stays the interface

**The watch page plays the stream under half a second behind the machine.** A playlist puts a player
two to four seconds behind, because a player keeps whole segments. The page instead appends each
frame to its own `<video>` as the encoder cuts it. In Chrome on the machine's own network, the page
ran a median of 0.37 s behind, and the playlist about 2.3 s further back.

**The playlist stays the interface.** VLC, Kodi, a set-top box and a television's own player open
`/stream/live.m3u8` as before. The socket at `/stream/live.ws` serves the page, and only a page can
use it. So every streaming machine serves both, and the page is still never the only way in.

**Both come from one encode.** A second muxer writes each encoded packet as a fragment of its own.
Encoding is the expensive half, so a viewer on the socket costs a copy of each packet.

**A socket per viewer is the shape the remotes already have.** The machine holds a WebSocket open to
every remote. The encoder publishes into a shared ring and never waits for a viewer. So a slow
viewer cannot hold up a frame, and nothing reaches the audio.

**A viewer starts on a keyframe.** A decoder shows nothing until its first keyframe. So the machine
skips what comes before one, and a new viewer waits up to a second.

**A viewer that falls behind is sent away to start again.** The ring holds about three seconds of
fragments. A viewer that misses some has a gap, and a `<video>` stops at a gap and waits there. The
machine closes that socket with code 1013, and the page reconnects at a keyframe.

**The page ends in a picture or in the playlist, never in a dead end.**
- A browser that cannot use the socket plays the playlist instead.
- A device that falls behind three times without half a minute of picture plays the playlist too.
- A machine that restarts is waited for, because its playlist is gone as well.

**`/watch/?hls` skips the socket.** A television's media source may stutter where its own player
does not. The query gives that set the playlist on the same page.

**Low-Latency HLS stays refused.** It would bring the native players closer, and it holds a playlist
request open for every part. The socket brings the page closer, and the native players keep plain
HLS.
