# `km-api/static`

What the watch page is made of. It is served from `/watch` and `/watch/`, and compiled in with
`include_str!` and `include_bytes!` rather than read from disk. The development console's built-in
copy gives the reason. A machine under a television has whatever its executable carries, and no
folder anybody can put a file in.

| File | What it is | License |
|---|---|---|
| `watch.html` | the page itself: one `<video>`, and the script that feeds it | this project's |
| `hls.light.min.js` | [hls.js](https://github.com/video-dev/hls.js) **1.6.15**, the `light` build, unmodified but for one line | Apache-2.0, verbatim in `hls-LICENSE.txt` |
| `hls-LICENSE.txt` | hls.js's license, served at `/watch/hls-LICENSE.txt`, because a vendored dependency's terms travel with it | |

## Two paths, the socket first

**The page plays `/stream/live.ws` when the browser has a media source**, and the playlist
otherwise. The socket pushes each fragment as the encoder cuts it, so the picture runs under half a
second behind. A playlist player keeps whole segments and runs two to four.

- **The socket is tried when** `MediaSource` or `ManagedMediaSource` exists, a WebSocket exists, and
  the media source accepts H.264 and AAC.
- **The playlist takes over** when the socket never delivers, or the media source refuses the
  stream's real codec. It also takes over after three falls behind without half a minute of picture.
- **A machine that goes away is waited for**, with a line saying so. Its playlist is gone as well.
- **`/watch/?hls` skips the socket**, for a television whose media source stutters.

## Why a library is here at all

**Most of the clients need none of it.** A smart television's browser, Safari, iOS and Android all
play an HLS playlist from a plain `<video src>`, through the platform's own media pipeline. Those are
the machines this stream exists for.

**Desktop Firefox and older desktop Chrome are the exception, and play no HLS at all.** They are
therefore the one place nobody can watch the playlist without help. They are also where somebody
developing this, or checking from the computer they are already sitting at, will try first.

So the page asks the browser and takes the cheaper road when it can. `canPlayType` for the HLS media
type, native where it is there, and the library only where it is not.

**The `light` build, because this page uses none of what the full one adds.** It leaves out alternate
audio tracks, subtitles and encrypted media. This stream is one video track and one audio track,
unencrypted, and the encoder draws any words on screen into the picture first.

**The page asks for H.264 and AAC itself, because `Hls.isSupported` asks for less.** It passes a
browser that decodes any one of H.264, VP9, AV1, AAC or FLAC. A Chromium built without proprietary
codecs passes on VP9 and then shows a black screen. So the page tests the stream's own codecs first.
It also stops at the codec error hls.js raises for an init segment the browser refuses. Both send the
viewer to VLC with the playlist's address.

## The one modification

The `//# sourceMappingURL=` comment on the last line goes, and nothing else is touched. The map it
names is not vendored here. A browser with its developer tools open would ask this machine for a file
that was never here. It would then report a 404 against a page that is working perfectly.
