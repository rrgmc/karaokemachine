# Watching it in another room

The machine normally draws on the television it is plugged into. When the singing is in a room with a
television and no machine, stream the screen there instead. The machine sends its picture and its
sound over your network, and the other television plays them in a browser or a player.

## Start the stream

**Every install has a second launcher that starts the machine streaming**, with nothing to type. Its
icon has a broadcast badge in the corner.

| Platform | Where the launcher is |
|---|---|
| Windows | A Start Menu entry |
| Linux | An action in the desktop menu |
| macOS | `KM Stream.app` |

From a shell, `karaokemachine --stream` does the same. The flag applies to that run only, and the
machine refuses it together with `--headless`.

A streaming machine opens no window, because it sends the screen to an encoder instead of a
television. On Windows it shows an icon in the notification area, and on macOS in the menu bar. The
icon names the address a phone can reach, and it follows that address as the network changes. Its
*Remote*, *Watch* and *Setup* entries open the three pages the machine serves.

## Open it on the other television

You have two ways to watch, and both start from the machine's address.

- **In a browser**, open `http://<the machine>/watch/`. The page runs under half a second behind the
  machine.
- **In a player**, open `http://<the machine>/stream/live.m3u8`. A television's own player, VLC, Kodi
  or a set-top box plays it. A player runs three or four seconds behind.

The page is faster because it takes the stream over a WebSocket where the browser can. Where the
browser cannot, the page plays the playlist. A machine that does not stream serves neither address.

Queue songs from a phone as usual. See [The remotes](remotes.md).

## What a stream costs

- **It runs behind the machine.** When you press pause, the music continues for that long before it
  stops.
- **It carries only the backing track.** The microphones go to a hardware mixer and never reach the
  machine, so no voice is in the stream.

## If the picture stops to buffer

An older television may stop to buffer. The fix depends on how you watch.

- **On the page**, open `/watch/?hls` to play the playlist instead.
- **In a player**, set `stream.segment_seconds` to `2` in `settings.json`. The stream then plays
  smoothly, a few seconds further behind.
