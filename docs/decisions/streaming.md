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

**HLS rather than anything lower-latency, because the clients are televisions.** A set's browser
plays a playlist through the television's own pipeline. A stream fed from JavaScript reaches only the
newer models. Files also make this ordinary request-and-response. The alternative is a body held open
for the length of a song, on a machine whose other job is playing audio.

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

**Public, on the same terms as the queue.** It sits outside `/api/v1/admin/`, so no token is asked
for. That is the judgment [`Network reach`](api-and-network.md#network-reach) already makes about a
home LAN indoors. Anybody in the room can queue a song, so anybody in the room can watch the screen
they are queueing it onto.

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
