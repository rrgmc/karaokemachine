A karaoke machine that plays MIDI files, video files, MP3+G pairs, and UltraStar and LRC songs. It
highlights the words in time with the music, takes song requests from a phone, and has an HTTP API
for search, queueing and control.

## Download

| File | For |
|---|---|
| [`karaokemachine-setup-@VERSION@-windows-x86_64.exe`](https://github.com/rrgmc/karaokemachine/releases/download/v@VERSION@/karaokemachine-setup-@VERSION@-windows-x86_64.exe) | Windows |
| [`karaokemachine-setup-@VERSION@-macos-aarch64.pkg`](https://github.com/rrgmc/karaokemachine/releases/download/v@VERSION@/karaokemachine-setup-@VERSION@-macos-aarch64.pkg) | macOS on Apple Silicon |
| [`karaokemachine_@VERSION@-1_amd64.deb`](https://github.com/rrgmc/karaokemachine/releases/download/v@VERSION@/karaokemachine_@VERSION@-1_amd64.deb) | Debian and Ubuntu |
| [`karaokemachine-@VERSION@-x86_64-unknown-linux-gnu.tar.gz`](https://github.com/rrgmc/karaokemachine/releases/download/v@VERSION@/karaokemachine-@VERSION@-x86_64-unknown-linux-gnu.tar.gz) | Any other Linux |
| [`karaokemachine-@VERSION@-android.apk`](https://github.com/rrgmc/karaokemachine/releases/download/v@VERSION@/karaokemachine-@VERSION@-android.apk) | Android and Google TV |
| [`karaokemachine-@VERSION@-quest.apk`](https://github.com/rrgmc/karaokemachine/releases/download/v@VERSION@/karaokemachine-@VERSION@-quest.apk) | Meta Quest |
| [`karaokemachine-@VERSION@-ios-unsigned.ipa`](https://github.com/rrgmc/karaokemachine/releases/download/v@VERSION@/karaokemachine-@VERSION@-ios-unsigned.ipa) | iPhone and iPad |

Each of these is the whole machine for its platform. **Every file** below describes them, and has
the portable copies and the remote on its own.

## What changed

- **A portable copy for Windows and Linux.** Unpack it anywhere and run it. Every program keeps its
  settings and songs in the `data` folder inside.
- **A lyric sync editor puts words on a MIDI file.** The song plays while you press Space on each
  word, and the editor writes a new `.kar`.
- **KM Song Sync starts the editor from a page.** Browse to a folder, select a song, paste its words
  and press Start.
- **A site's song files become a package in one command**, with `km-site-pack`.
- **Every song counts you back in after a break.** A bar above the next line fills during a long
  pause and is full when the line starts.
- **The curation tool's Folders page shows each folder's average suitability**, and sorts by name,
  by songs or by suitability.
- **The recommended instrument bank downloads again.** The machine takes the publisher's current
  release.

## Every file

| File | For |
|---|---|
| `karaokemachine-setup-@VERSION@-windows-x86_64.exe` | Windows 10 or 11, 64-bit. One installer for every program, with a checkbox per part. It installs for your account only and does not ask for an administrator password. |
| `karaokemachine-setup-@VERSION@-macos-aarch64.pkg` | macOS on Apple Silicon. The same seven programs behind six checkboxes. Applications go to `/Applications` and the command-line tools to `/usr/local/bin`. It asks for your administrator password once and downloads nothing. |
| `km-remote-setup-@VERSION@-windows-x86_64.exe` | Windows 10 or 11, 64-bit — **the remote on its own**, for a computer that is not the karaoke machine. About 5 MB. The installer above already contains it; this one is for a computer that wants nothing else. |
| `karaokemachine-portable-@VERSION@-windows-x86_64.zip` | Windows 10 or 11, 64-bit — **a portable copy**. Unzip it anywhere and run it. Every program is in it, and each keeps its settings and songs in the `data` folder inside. It reads and writes nothing in your user profile, so it runs beside an installed copy without touching it. |
| `km-remote-setup-@VERSION@-macos-aarch64.pkg` | macOS on Apple Silicon — **the remote on its own**. KM Remote goes to `/Applications` and nothing is put on your `PATH`. |
| `karaokemachine_@VERSION@-1_amd64.deb` | Debian 13 or later, amd64. Install it with `sudo apt install ./karaokemachine_@VERSION@-1_amd64.deb`. |
| `karaokemachine-tools_@VERSION@-1_amd64.deb` | The package builder, the offline remote and the picture-and-bank tool, for the same Debian. Download it beside the one above and install both at once: `sudo apt install ./karaokemachine_@VERSION@-1_amd64.deb ./karaokemachine-tools_@VERSION@-1_amd64.deb`. A box under a television needs only the machine. |
| `karaokemachine-@VERSION@-x86_64-unknown-linux-gnu.tar.gz` | Any other 64-bit Linux. Unpack it anywhere and run it. It includes the libraries it needs. |
| `karaokemachine-portable-@VERSION@-linux-x86_64.tar.gz` | Any 64-bit Linux — **a portable copy**. Unpack it anywhere and run it. It holds the machine and every tool, and each keeps its settings and songs in the `data` folder inside. It reads and writes nothing in your home directory. |
| `karaokemachine-@VERSION@-android.apk` | The machine on Android and Google TV. One file covers 32-bit and 64-bit ARM. |
| `km-remote-@VERSION@-android.apk` | The remote, for a phone. It works when the machine is not reachable. One file covers both ARM architectures. |
| `karaokemachine-@VERSION@-quest.apk` | The machine on a Meta Quest, on a screen that hangs in the room with the room still behind it. It installs beside the Android APK rather than over it. |
| `karaokemachine-@VERSION@-ios-unsigned.ipa` | The machine on an iPhone or iPad. Sign it before installing it; see **Before you install** below. |
| `km-remote-@VERSION@-ios-unsigned.ipa` | The remote on an iPhone or iPad. Sign it the same way. |

`@CAROLS@` is a song package and a separate download. It holds sixteen public-domain Christmas
carols from the Open Hymnal Project. Put it in the folder that `karaokemachine --show-paths` calls
`packages` and press `Ctrl+F10`, or send it to the machine from the curation tool. A new install has
an empty catalog.

## Before you install

<!-- platform: windows -->
- **Windows** shows *"Windows protected your PC"*. Click **More info**, then **Run anyway**.
<!-- /platform -->
<!-- platform: ios -->
- **Both `.ipa` files are unsigned, and you sign them yourself** with your own Apple ID. It takes
  about five minutes and a computer. The manual has the steps in
  [On an iPhone or an iPad](https://rrgmc.github.io/karaokemachine/docs/ios.html).
<!-- /platform -->

The manual's
[Installing](https://rrgmc.github.io/karaokemachine/docs/installing.html#what-each-system-asks)
chapter says what every system asks when you install.

Everything here is built from the sources in this repository, at tag `v@VERSION@`.
