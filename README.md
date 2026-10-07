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

![The playing screen on a television: the song's number, title and artist across the top with key and
melody badges and a disc in the corner counting the songs waiting, the line being sung in large
letters with the current syllable half-filled in amber, the line that follows it below in gray, and a
progress bar along the bottom](docs/images/screen-playing.png)

## Installing

**The downloads are on the [release page](https://github.com/rrgmc/karaokemachine/releases)**, one
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
