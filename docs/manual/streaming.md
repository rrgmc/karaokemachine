# Watching it in another room

**`--stream` sends the screen to an encoder instead of a television.** The machine opens no window. It
serves the picture and the sound as one continuous HLS stream. The flag applies to one run only, and
the machine refuses it together with `--headless`.

**Any player that follows a URL plays `http://<the machine>/stream/live.m3u8`**: a television's own
player, VLC, Kodi or a set-top box. `http://<the machine>/watch/` shows the same stream on a page. A
machine that does not stream serves neither.

**The page runs under half a second behind, and a player on the playlist three or four.** The page
takes the stream over a WebSocket where the browser can. Where it cannot, it plays the playlist.

**A launcher starts the stream with nothing to type.** It is a Start Menu entry on Windows, an action
in the Linux desktop menu, and `KM Stream.app` on macOS. Its icon has a broadcast badge in the corner.

**A streaming run shows an icon in the notification area on Windows and in the menu bar on macOS.**
The icon names the address a phone can reach. Its *Remote*, *Watch* and *Setup* entries open the three
pages the machine serves. It follows the address as the network changes.

**The stream costs two things.** It runs behind the machine, so a pause lets the music continue that
long. It also carries only the backing track, because the microphones go to a hardware mixer and
never reach the machine.

**An older television may stop to buffer.** On the page, open `/watch/?hls` to play the playlist
instead. On the playlist, set `stream.segment_seconds` to `2` in `settings.json`, and it plays
smoothly a few seconds further behind.
