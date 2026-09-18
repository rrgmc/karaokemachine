# Changelog

What a release of the machine, its remotes and its tools gives the person using them. The format
is [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the numbering is
[semantic versioning](https://semver.org/spec/v2.0.0.html) — one number covers every program here,
so a release moves all of them together.

Entries are written for somebody who has the machine. Why any of it is the way it is lives in
[`docs/decisions/`](docs/decisions/), and how it is built in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## [Unreleased]

## [1.17.0] - 2026-09-17

### Added

- **A karaoke machine that behaves like a commercial home unit.** Pick a song by number, it plays,
  the words highlight in time. It runs full-screen on a television, and phones on the same network
  are the remotes. Native on Windows, macOS and Linux, on Android, and on an iPhone and an iPad; on
  Debian it draws straight to the screen from a bare TTY, with no desktop installed.

- **Four kinds of song file, and nothing else.** MIDI and KAR files in all three karaoke conventions
  that exist in the wild; video songs in an MP4; MP3+G pairs, an MP3 with a `.cdg` of the same stem;
  and UltraStar songs, the `.txt` a singing game times its words in beside the MP3 it names. An MP3
  on its own is not a song, because it has no words in it.

- **Songs arrive in packages.** A `.kmpkg` carries its own queue numbers, titles, artists and
  analysis. Drop one in the packages folder or onto the machine's window and it goes in without a
  restart.

- **Every song file is rated for suitability out of ten**, with a breakdown, and carries the language
  it is in and the first line or two of its words — so a catalog can be asked what Portuguese it
  holds, and a list shows something a person recognizes a song by. The melody channel is found at
  packaging time when it can be found confidently, and abstains with a stated reason when it cannot.

- **Words highlight syllable by syllable on the sequencer's own clock.** Transpose and tempo are per
  song, the guide melody can be muted, and a lyric timing offset in milliseconds is adjustable
  mid-song — a television adds picture lag and a mixer takes the sound out early. The offset moves
  the highlight only, never the audio, because the microphones are in that audio.

- **A queue with singer names**, shown over whatever is playing, and wallpapers cycling behind it
  with a crossfade. A zip of images counts as a folder of them.

- **A remote at the machine's own address**, with a QR code on the idle screen: search, queue, now
  playing, and whatever controls the song allows. Any phone on the network, no app to install.

- **A standalone offline remote** that keeps its own copy of a machine's catalog, so browsing,
  searching and favorites work with the machine switched off.

- **A television in another room.** Started with `--stream` the machine opens no window and serves
  what it would have shown — the words, the highlight moving through them, the wallpaper, the queue
  and the music — at one address. A smart television, a phone or a computer opens `/watch/`, and
  anything that plays a playlist takes `/stream/live.m3u8` and needs no browser.

- **One admin password, which the machine gives itself and shows on screen.** A six-digit PIN at
  first start, beside the address on the idle screen. Everything that reconfigures the machine wants
  it; everything a singer does wants nothing.

- **KaraokeMachine Admin finds pictures and instruments and sends them over.** Photographs the lyrics
  stay readable over, measured in the exact band of the screen the words occupy, and General MIDI
  banks from a table of sixty-three. It also sends a file you already have, which on a television box
  is the only way to hand the machine one. The machine downloads nothing itself: it may have no
  internet connection, and it should not hold your accounts.

- **A printed song book**, and a curation tool that turns a folder of files into a package — scanning,
  rating, naming, numbering and correcting a corpus a few hundred thousand files deep.

- **An HTTP API for the whole of it**, discovery on the network, and a development console that
  needs no password while the machine is in debug mode.

- **A demo mode, off unless asked for.** After a minute of quiet the machine starts a song of its
  own, and another when that ends. Queueing takes the deck off it at once, so nothing has to be
  skipped first.
