# `km-api/static`

What the watch page is made of. Served from `/watch` and `/watch/`, compiled in with `include_str!`
and `include_bytes!` rather than read from disk, for the reason the development console's built-in
copy gives: a machine under a television has whatever its executable carries and no folder anybody
can put a file in.

| File | What it is | License |
|---|---|---|
| `watch.html` | the page itself: one `<video>`, and the script that feeds it | this project's |
| `hls.light.min.js` | [hls.js](https://github.com/video-dev/hls.js) **1.6.15**, the `light` build, unmodified but for one line | Apache-2.0, verbatim in `hls-LICENSE.txt` |
| `hls-LICENSE.txt` | hls.js's license, served at `/watch/hls-LICENSE.txt`, because a vendored dependency's terms travel with it | |

## Why a library is here at all

**Most of the clients need none of it.** A smart television's browser, Safari, iOS and Android all
play an HLS playlist from a plain `<video src>` through the platform's own media pipeline, and those
are the machines this stream exists for.

**Desktop Chrome and Firefox are the exception and play no HLS at all**, which makes them the one
place the stream cannot be watched without help — and they are where somebody developing this, or
checking from the computer they are already sitting at, will try first.

So the page asks the browser and takes the cheaper road when it can: `canPlayType` for the HLS media
type, native where it is there, and the library only where it is not.

**The `light` build, because none of what the full one adds is used here.** It leaves out alternate
audio tracks, subtitles and encrypted media; this stream is one video track and one audio track,
unencrypted, and any words on screen are drawn into the picture before it is encoded.

## The one modification

The `//# sourceMappingURL=` comment on the last line is removed, and nothing else is touched. The map
it names is not vendored, so a browser with its developer tools open would ask this machine for a
file that was never here and report a 404 against a page that is working perfectly.
