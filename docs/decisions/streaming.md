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

**Being several seconds behind costs nothing that matters.** The stream is the only screen and the
only sound in the room it is watched in. So it is in time with itself, and nobody is chasing a
television. What it costs is control: pause, and the music runs on for the length of the buffer.
Segment length is the dial.

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
