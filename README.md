<h1>
  <img src="icon/icon-64.png" width="48" align="absmiddle"
       alt="The KaraokeMachine icon: the letters KM on a near-black plate, over angular bands of
color, with the M in amber">
  KaraokeMachine
</h1>

A karaoke machine that behaves like a commercial home unit: pick a song by number, it plays, and the
words highlight in time. It runs full-screen on a television, and any phone on the network is a
remote: search the catalog, queue a song, change the key, skip.

A song is one of five things:

- a **MIDI file with embedded karaoke lyrics**;
- a **video file**;
- an **MP3+G pair**: an MP3 with a `.cdg` of the same stem beside it;
- an **UltraStar song**: the `.txt` a singing game times its words in, and the MP3 it names;
- an **LRC song**: an `.lrc` of timed lyrics, and the MP3 of the same name.

An MP3 on its own is not a song, because it has no words in it.

It runs on Windows, macOS and Linux, on Android and a Meta Quest, and on an iPhone and an iPad. The
pictures and the download are on the site,
**[rrgmc.github.io/karaokemachine](https://rrgmc.github.io/karaokemachine/)**.

## What it looks like

![The playing screen on a television: the song's number, title and artist across the top with key and
melody badges and a disc in the corner counting the songs waiting, the line being sung in large
letters with the current syllable half-filled in amber, the line that follows it below in gray, and a
progress bar along the bottom](docs/images/screen-playing.png)

<table>
<tr>
<td width="50%"><img src="docs/images/screen-idle-connect.png" alt="The idle screen: a partly typed
song number in blue, a disc in the corner counting the songs waiting, and a panel giving the
machine's address on the network beside a QR code"></td>
<td width="50%"><img src="docs/images/screen-queue.png" alt="The queue overlay drawn over a dimmed
playing screen, listing four waiting songs by number and title with the singer who asked for each in
blue at the right"></td>
</tr>
<tr>
<td><sub><b>Idle.</b> Type a number, or point a phone at the code.</sub></td>
<td><sub><b>The queue.</b> What is waiting, and who asked for it.</sub></td>
</tr>
</table>

<img src="docs/images/screen-singing.webp" alt="Two lines of the carol Angels From the Realms of
Glory sung on the playing screen: each syllable fills in amber as it is sung, the next line waits
below in gray, and when the first line ends the one after the next takes its row">

<sub><b>Singing.</b> Each syllable fills in time with the music, and the next line is already
waiting.</sub>

<table>
<tr>
<td width="25%"><img src="docs/images/remote-browse.png" alt="The singer's remote on a phone: mode
buttons for songs, artists and a printed book, a search box, a language filter, and a list of songs
with artist, duration and number, each with a button to look it up on YouTube and a button to add it
to the queue"></td>
<td width="25%"><img src="docs/images/remote-now.png" alt="The remote's now-playing page: the song,
the singer it was queued for, a progress bar, and steppers for key and tempo, a music volume slider
and a guide-melody toggle"></td>
<td width="25%"><img src="docs/images/remote-queue.png" alt="The remote's queue page: the song
playing across the top with its transport folded away behind a toggle, then the queued songs by
position, title, artist and the singer who asked for each, their move and remove buttons folded away
behind a second toggle"></td>
<td width="25%"><img src="docs/images/remote-offline.png" alt="The offline remote showing its song
list, with a favorites mode, an A to Z picker, and a star beside each song for filing it as a
favorite"></td>
</tr>
<tr>
<td><sub><b>Search and queue</b> from any phone. No app to install.</sub></td>
<td><sub><b>The song as it plays</b> — key, tempo, guide melody, music volume.</sub></td>
<td><sub><b>Who is up next</b>, changeable by anyone with the skip level. The controls stay
folded until asked for.</sub></td>
<td><sub><b>The offline remote</b> adds favorites and an A–Z picker, and works with the machine
switched off.</sub></td>
</tr>
</table>

<img src="docs/images/headset-room.webp" alt="The machine inside a Meta Quest: the playing screen
hanging on a bare wall with the room still visible around it, and beside it a second panel showing
the queue of six waiting songs with the singer who asked for each">

<sub><b>In a headset</b>, the screen hangs on your wall and the queue hangs beside it.</sub>

## Features

**Songs**

- **Five kinds of song**: MIDI and KAR, video, MP3+G, UltraStar and LRC. See
  [Song files and their formats](https://rrgmc.github.io/karaokemachine/docs/formats.html).
- **Songs arrive in packages** (`.kmpkg`). Double-click one or drag it onto the window, and it goes
  in without a restart. See
  [Getting songs in](https://rrgmc.github.io/karaokemachine/docs/songs.html).
- **Song numbers that cannot clash.** Each package has its own block of a thousand, the same on
  every machine, so a printed list travels with the file.
- **A language per song, and a 0–10 suitability rating** for every file, with a breakdown.
- **A printable song book** of every installed song, as a PDF. See
  [The song book](https://rrgmc.github.io/karaokemachine/docs/song-book.html).

**Playing**

- **Words highlight in time**, syllable by syllable, on the sequencer's own clock.
- **Transpose and tempo** per song, and a **guide melody** that can be muted.
- **A lyric timing offset**, adjustable mid-song, for the picture lag of a television.
- **A queue** with singer names, shown over whatever is playing.
- **Wallpapers** that cycle with a crossfade, and a **demo mode** that plays songs by itself.
- **Every function on the keyboard**, and on a television remote. See
  [The keyboard](https://rrgmc.github.io/karaokemachine/docs/keyboard.html).
- **A stream for a television in another room**, which a browser or a playlist player opens. See
  [Watching it in another room](https://rrgmc.github.io/karaokemachine/docs/streaming.html).
- **A Debian box becomes an appliance**: it starts with the power and draws with no desktop
  installed. See
  [On Debian, it is also an appliance](https://rrgmc.github.io/karaokemachine/docs/appliance.html).
- **A `.deb` for a Raspberry Pi**, on Raspberry Pi OS based on Debian 13. It is built for arm64 and
  is not tested on a board.

**Remotes and setup**

- **Any phone on the network is a remote**, with no app to install. An offline remote works with the
  machine switched off. See
  [The remotes](https://rrgmc.github.io/karaokemachine/docs/remotes.html).
- **Three levels for the room**: watch, queue, and queue plus skip. One admin password covers
  everything that reconfigures the machine.
- **Setup from a browser**: songs, pictures, sound banks, and whatever is wrong. See
  [Setting it up from a browser](https://rrgmc.github.io/karaokemachine/docs/setup.html).
- **An HTTP API with a WebSocket event stream**, and mDNS discovery. See
  [The HTTP API and the network](https://rrgmc.github.io/karaokemachine/docs/api.html).

**Tools**

- **Package tools** that curate a folder of files, build packages and check them. See
  [Getting a corpus into shape](https://rrgmc.github.io/karaokemachine/docs/packaging.html).
- **A lyric sync editor** that puts words on a MIDI file as you tap them. See
  [Putting words on a MIDI file](https://rrgmc.github.io/karaokemachine/docs/lyric-sync.html).
- **KaraokeMachine Admin** finds photographs the lyrics stay readable over, and offers General MIDI
  banks.

[What it does](https://rrgmc.github.io/karaokemachine/docs/what-it-does.html) has the whole list,
and what the machine leaves out by decision.

## Installing

**The downloads are on the [release page](https://github.com/rrgmc/karaokemachine/releases/latest)**, one
file per platform. The manual's [Installing](https://rrgmc.github.io/karaokemachine/docs/installing.html)
chapter says what each file is. To build from source, see [`BUILDING.md`](BUILDING.md).

**Windows shows *"Windows protected your PC"* when you open a setup program**, because the Windows
downloads are not signed. Click **More info**, then **Run anyway**.

## Starting

1. Install the machine and start it.
2. Double-click a `.kmpkg` package, or drag it onto the machine's window. The release page has one
   of sixteen Christmas carols.
3. Type a song number and press `Enter`.

The idle screen shows the machine's address and a QR code. Point a phone at the code to search the
catalog and queue a song.

## The manual

**The manual is at
[rrgmc.github.io/karaokemachine/docs](https://rrgmc.github.io/karaokemachine/docs/).** Its source is
[`docs/manual/`](docs/manual/).

| Part | What it covers |
|---|---|
| [Getting started](https://rrgmc.github.io/karaokemachine/docs/pictures.html) | What it looks like and what it does. Installing on each platform, the Debian appliance, signing for an iPhone, and removing it. |
| [Using it](https://rrgmc.github.io/karaokemachine/docs/keyboard.html) | The keyboard, getting songs in, the remotes, watching it in another room, setting it up from a browser, and the song book. |
| [For a technical reader](https://rrgmc.github.io/karaokemachine/docs/command-line.html) | The command line, the song file formats, the HTTP API, the tools that make a package, and the lyric sync editor. |

## Documentation

| File | What it is |
|---|---|
| [`docs/manual/`](docs/manual/) | **Installing and using it.** The manual, a chapter per file. |
| [`CHANGELOG.md`](CHANGELOG.md) | **What changed.** What a release gave you. |
| [`BUILDING.md`](BUILDING.md) | **Building it.** Prerequisites, the cargo aliases and the Taskfile, video, the release scripts and CI. |
| [`RELEASE.md`](RELEASE.md) | **Cutting a release.** The order the steps go in. |
| [`DEPLOYING.md`](DEPLOYING.md) | **Getting it onto a device.** Android and iOS, and a Linux box over SSH. |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | **Working on it.** What to run before a pull request, the conventions, and the invariants. |
| [`SECURITY.md`](SECURITY.md) | **Reporting a vulnerability.** Privately, through GitHub, and what counts as one. |
| [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) | **How people treat each other here.** The Contributor Covenant. |
| [`docs/decisions/`](docs/decisions/) | **Why it is the way it is.** Every product decision, in topic files, [indexed](docs/decisions/README.md). |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | **How it is built.** The overview and the crate map; [`docs/architecture/`](docs/architecture/) has a note per subsystem. |
| [`docs/research/`](docs/research/) | Investigations. Findings, not commitments. |
| [`docs/learning-rust.md`](docs/learning-rust.md) | What a C++ reader needs in order to read this codebase. |
| [`docs/HISTORY.md`](docs/HISTORY.md) | **Where it came from.** Five attempts at this machine over eighteen years, and where each one stopped. |
| [`CLAUDE.md`](CLAUDE.md) | Repository guide, and the rules those documents are kept under. |

## Related projects

- [KaraokeMachine Video Tools](https://github.com/rrgmc/km-video-tools)

### Sites with downloadable karaoke files

You may use the command-line app "km-site-pack" to make a package out of these sites.

* [Geoff Carters MIDI Page](https://midkar.com/Geoff_Carters_MIDI_Page/GeoffCartersMIDIs_A_to_Z.html)
* [karaokemusic weebly](https://karaokemusic.weebly.com/)
* [Jimmy Sears](http://jimmy-sears.awardspace.biz/music/music.htm)
* [mg20.vc-graz](https://mg20.vc-graz.ac.at/karaoke/songs/)

## License

`MIT OR Apache-2.0`, at your option, as every crate in the workspace declares. The texts are
[`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).

Unless you say otherwise, a contribution you deliberately submit falls under the same dual license,
with no additional terms.

**The license covers the application only.** A release carries the ffmpeg libraries under the LGPL,
with their terms in the folder that holds them. The instrument bank has its own license.

**The seven default wallpapers are CC0 photographs**, in `assets/wallpapers/default-wallpapers.zip`.
`CREDITS.md` beside the zip names each photographer and source page, and says how each image was
edited.

**No release includes a pack you build with
[`tools/cmd/assets/km-wallpaper-pack`](tools/cmd/assets/km-wallpaper-pack).** Only an Openverse pack
may travel on. A Pixabay or Pexels pack stays on the machine that built it, and `manifest.json`
records each image's license. See [`Where a wallpaper pack's photographs may come
from`](docs/decisions/repository.md#where-a-wallpaper-packs-photographs-may-come-from).

## Privacy

**No program sends telemetry, crash reports or usage data, and none checks for updates.** A crash
report stays in a file on the computer. A program contacts a host on the internet only when you ask
for a download or a picture search. The manual's
[Privacy](https://rrgmc.github.io/karaokemachine/docs/privacy.html) chapter names each case.

## Author

Rangel Reale (realerangel@gmail.com)
