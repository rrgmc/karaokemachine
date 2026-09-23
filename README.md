<h1>
  <img src="icon/icon-64.png" width="48" align="absmiddle"
       alt="The KaraokeMachine icon: the letters KM on a near-black plate, over angular bands of
color, with the M in amber">
  KaraokeMachine
</h1>

A karaoke machine that behaves like a commercial home unit: pick a song by number, it plays, and the
words highlight in time. It runs full-screen on a television, and any phone on the network is a
remote: search the catalog, queue a song, change the key, skip.

A song is one of four things:

- a **MIDI file with embedded karaoke lyrics**;
- a **video file**;
- an **MP3+G pair**, an MP3 with a `.cdg` of the same stem beside it, which is what most commercial
  karaoke discs hold;
- an **UltraStar song**: the `.txt` a singing game times its words in, and the MP3 it names.

An MP3 on its own is not a song, because it has no words in it.

Native on Windows, macOS and Linux, on Android, on a Meta Quest, and on an iPhone and an iPad.

The site is **[rrgmc.github.io/karaokemachine](https://rrgmc.github.io/karaokemachine/)** — the
pictures, and the download.

## Contents

- [What it looks like](#what-it-looks-like)
- [What it does](#what-it-does)
- [Installing](#installing)
  - [On Debian, it is also an appliance — if you ask](#on-debian-it-is-also-an-appliance--if-you-ask)
  - [On an iPhone or an iPad, you sign it yourself](#on-an-iphone-or-an-ipad-you-sign-it-yourself)
  - [On an iPhone or an iPad, two things are different](#on-an-iphone-or-an-ipad-two-things-are-different)
  - [Removing it](#removing-it)
- [Using it](#using-it)
  - [The keyboard](#the-keyboard)
  - [Getting songs in](#getting-songs-in)
  - [The remotes](#the-remotes)
  - [Watching it in another room](#watching-it-in-another-room)
  - [Setting it up from a browser](#setting-it-up-from-a-browser)
  - [The song book](#the-song-book)
- [For a technical reader](#for-a-technical-reader)
  - [The command line](#the-command-line)
  - [Song files and their formats](#song-files-and-their-formats)
  - [Text encodings and writing systems](#text-encodings-and-writing-systems)
  - [The HTTP API and the network](#the-http-api-and-the-network)
  - [Getting a corpus into shape](#getting-a-corpus-into-shape)
- [Building it](#building-it)
- [Documentation](#documentation)
- [License](#license)
- [Author](#author)

---

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
<td><sub><b>Who is up next</b>, changeable by anyone. The controls stay folded until asked
for.</sub></td>
<td><sub><b>The offline remote</b> adds favorites and an A–Z picker, and works with the machine
switched off.</sub></td>
</tr>
</table>

---

## What it does

**Songs**

- **MIDI and KAR files**, in all three karaoke conventions that exist in the wild.
- **Video songs**, in an MP4.
- **MP3+G songs** — an MP3 with a `.cdg` of the same stem.
- **UltraStar songs** — the `.txt` a singing game times its words in, beside the MP3 it names.
- **Songs arrive in packages** (`.kmpkg`), each carrying its own queue number, title, artist and
  analysis. Drop one in the packages folder, or onto the machine's window, and it goes in without a
  restart.
- **A language per song**, so a catalog can be asked what Portuguese it has.
- **A 0–10 suitability rating** for every file, computed when it is packaged, with a breakdown.
- **The melody channel, when it can be found confidently** — and an abstention with a stated reason
  when it cannot. Detected once, at packaging time.
- **The first line or two of each song's words**, so a package carries something a person recognizes
  the song by. It skips the studio-name banner many karaoke files open with, and a real corpus set
  the rules for that.

**Playing**

- Full-screen on a television, drawn by SDL3. On Linux it draws straight to DRM/KMS from a bare TTY,
  with no desktop installed.
- **Or on a television in another room.** Started with `--stream`, the machine opens no window. It
  serves what it would have shown — the words, the wallpaper, the queue and the music — at one
  address. **The address is the whole of it**: a smart television, a phone or a computer opens
  `/watch/`. Any playlist player takes `http://<the machine>/stream/live.m3u8` with no browser.
  [Further down](#watching-it-in-another-room).
- **Words highlight in time**, syllable by syllable, on the sequencer's own clock.
- **Transpose and tempo** per song, a **guide melody** that can be muted, and per-song defaults.
- **A lyric timing offset in milliseconds**, adjustable mid-song, because a television adds picture
  lag and a mixer takes the sound out early. It moves the *highlight* only, never the audio, because
  the microphones are in that audio.
- **Wallpapers** cycling with a crossfade, from a folder of stills. A zip in that folder counts as a
  folder of images.
- **A queue** with singer names, shown over whatever is playing.
- **A demo mode**, off unless asked for: after a minute of quiet the machine starts a song of its
  own, then another when that ends. Queueing takes the deck off it at once, so choosing a song is
  what makes it play and nobody has to skip first.

**Remotes**

- **The machine serves a remote at its own address** — search, queue, now playing, and the controls a
  song allows. Any phone on the network, no app, and a QR code on the idle screen.
- **A standalone offline remote**, which keeps its own copy of a machine's catalog. Browsing,
  searching and favorites all work with the machine switched off.
- **One admin password, which the machine gives itself and shows on screen.** A six-digit PIN at
  first start, on the idle screen beside the address. Everything that reconfigures the machine needs
  it: its packages and pictures, a package's first number, its name, its audio output, demo mode.
  Everything a singer does needs nothing.

**Giving the machine pictures and instruments**

- **KaraokeMachine Admin** (`km-admin`) finds photographs the lyrics stay readable over, measured in
  the exact band of the screen the words occupy. It also offers General MIDI banks from a table of
  sixty-three, and it sends either to the machine.
- **It also sends a file you already have**: a package, a bank, a photograph of your own. A
  television box has no shell and no file manager reaching where the machine looks, so otherwise
  there is no way to hand it one.
- **The machine cannot download these itself.** It may have no internet connection, and it should not
  store your accounts. km-admin keeps a copy of every download on your computer, so you can send it
  to a switched-off machine later. A file you supply yourself goes straight through, and km-admin
  keeps no copy.
- **Pictures come from Openverse by default**, which needs no account and whose packs may travel on.
  Pixabay and Pexels need an API key of your own, and their terms forbid that, so the key field
  carries those terms beside it.

**Not supported, by decision**

- No scoring of singers.
- No bare audio files, and no CD+G disc images — only the file pair.
- No video wallpapers: a video song is not a video background.
- No pitch shifting of audio.
- No microphone processing in the app, because hardware mixes the microphones.
- No Thai, Arabic or Indic words on the screen, and no right-to-left, though the machine draws
  Japanese and Chinese.

These are deliberate decisions rather than missing features. To propose a change, start with
[`docs/decisions/`](docs/decisions/).

The HTTP API, discovery on the network, the file formats and the tools that make a package are
[further down](#for-a-technical-reader).

---

## Installing

**The downloads are on the [release page](https://github.com/rrgmc/karaokemachine/releases)**, one
file per platform, with the carol package beside them as a separate download. The table below says
what each carrier is. Building from source is the other way in — [`BUILDING.md`](BUILDING.md) is how,
and it is one command per platform once the prerequisites are in.

| Platform | What you get |
|---|---|
| **Windows** | A setup program, `karaokemachine-setup-<version>-windows-x86_64.exe`, holding all seven products behind component checkboxes. It installs **per-user** into `%LOCALAPPDATA%\Programs` and raises no UAC prompt, and offers to put itself on your `PATH` and to open `.kmbuild` files. Or a **portable folder**: unzip and run. |
| **macOS** | An installer package, `karaokemachine-setup-<version>-macos-<arch>.pkg` — the same seven products behind six component ticks. Applications go to `/Applications`, command-line tools to `/usr/local/karaokemachine` with symlinks in `/usr/local/bin`. It asks for your administrator password once and fetches nothing. Or `Karaoke Machine.app` on its own. |
| **Windows or macOS, the remote alone** | A second, small setup program: `km-remote-setup-<version>-windows-x86_64.exe` (about 5 MB) or `km-remote-setup-<version>-macos-<arch>.pkg`. It installs KM Remote and nothing else, for a computer that is never going to play a song — a laptop somebody holds while somebody else's machine does. It sits happily beside a full install and is removed on its own. |
| **Debian, Ubuntu** | A `.deb`. Its ffmpeg and font dependencies are named rather than bundled. It installs as an ordinary application — menu entry, icon, and `karaokemachine` as a command — and carries the television-appliance service, switched off. A second `.deb`, `karaokemachine-tools`, holds the package builder, the offline remote and the picture-and-bank tool; name both files in one `apt install` to get them, or take the machine alone for a box under a television. |
| **Any Linux** | A `.tar.gz`. Unpack anywhere, run it, delete it — no root, no package manager. It carries its own ffmpeg, because a folder can name no dependency. |
| **Android, Google TV** | An APK carrying both ABIs, so it installs on a phone and on a television. |
| **Meta Quest** | An APK of its own, which puts the machine on a screen hanging in the room with the room still behind it. The screen is flat or curved, and you pick which under it. The Android APK above also installs, as a flat system panel you move and resize, and the two live side by side. |
| **iPhone, iPad** | An `.ipa` for the machine and one for the remote, both **unsigned**: iOS takes no signature from a stranger, so you sign it yourself with your own Apple ID. It is the machine itself — the same synthesizer, catalog, display and API — and songs arrive through the Files app. The section below is the procedure. |

**Every carrier also installs a second launcher that starts the machine streaming**, for a television
in another room rather than this box's own. It is a way of starting the machine rather than a thing
to download, so the table above has no row for it.
[Watching it in another room](#watching-it-in-another-room) is what it serves.

**System-wide on macOS, per-user on Windows.** Everything the Windows installer configures beyond the
files lives in that user's registry. On macOS a bundle in `/Applications` declares the `.kmbuild`
association, and the `PATH` entry is a symlink in `/usr/local/bin`. Both belong to the machine.

### On Debian, it is also an appliance — if you ask

**Installing the `.deb` gives you an ordinary application**: a menu entry, an icon, and
`karaokemachine` as a command. Nothing starts automatically and nothing runs at boot.

The package also includes a systemd service, **switched off by default**. Enabling it turns the
computer into an appliance: it starts on its own when the power returns. It draws to the screen from
a bare virtual terminal, with no desktop installed. One command turns it on:

```sh
sudo systemctl enable --now karaokemachine
```

`sudo systemctl disable --now karaokemachine` turns it off again. There is one package either way;
you choose which of the two you get.

Two things to know first:

- **Do it on a machine with no desktop**, not on your laptop. The service takes over `tty1`, and a
  login screen — GDM, LightDM, SDDM — holds the graphics device it needs, so the television stays
  black.
- **It runs as a system user, `karaoke`**, so its songs, settings and catalog live under
  `/var/lib/karaoke`, not your home directory. Packages go in
  `/var/lib/karaoke/.local/share/karaokemachine/packages`. `sudo systemctl status karaokemachine`
  says whether it is running; `sudo journalctl -t karaokemachine` says what happened.

### On an iPhone or an iPad, you sign it yourself

**An iOS application can only be installed under a signature, and Apple issues none that a stranger
can hand you.** So both `.ipa` files say `unsigned` in the name, and the last step is yours. It takes
about five minutes and a computer, once per app.

You need an Apple ID, and the one on the phone is fine. You also need a free signing tool:
**[Sideloadly](https://sideloadly.io)** on macOS or Windows, or **[AltStore](https://altstore.io)**,
which installs from the phone afterwards.

1. Download `karaokemachine-<version>-ios-unsigned.ipa`, or
   `km-remote-<version>-ios-unsigned.ipa` for the remote alone.
2. Plug the phone or the iPad into the computer and unlock it.
3. Open Sideloadly, drag the `.ipa` onto it, enter your Apple ID and press **Start**. It signs the
   app for your devices and installs it.
4. On the device, open **Settings → General → VPN & Device Management**, tap your Apple ID under
   *Developer App*, and tap **Trust**. Until you do, the application will not open.
5. Launch it once with the network reachable. The certificate is verified on that first run.

Two limits come from the Apple ID rather than from the application:

- **A free Apple ID signs for seven days.** After that the app stops opening and you repeat step 3,
  and nothing inside it goes: your songs, settings and catalog stay on the device. A paid Apple
  Developer account signs for a year, and AltStore renews in the background if you leave it
  installed.
- **Three applications at a time** on a free Apple ID. The machine and the remote are two of them.

**The remote is the one most people want.** It is small, neither difference below touches it, and it
is what a guest holds while somebody else's machine plays.

### On an iPhone or an iPad, two things are different

**It does not announce itself on the network.** Apple grants the multicast entitlement only after a
reviewed request, so the machine advertises nothing and a remote will not find it by itself. The iOS
remote sweeps the network and finds it anyway; the Android remote and a browser need the address,
which the idle screen shows.

**A machine nobody is singing on stops answering when it goes to the background.** iOS suspends an
application that plays no audio. A song that is playing keeps playing with the screen off, which is
the case that matters in a room. A remote loses an idle machine until you bring it back to the
front.

### Removing it

**Your songs, settings and catalog stay where they are**, whichever way you remove it, and the
uninstaller names the folders on its way out.

On Windows, uninstall as you would anything else. On macOS, open `/usr/local/karaokemachine` and
double-click **Uninstall KaraokeMachine**; it lists what it will remove, asks, and only then asks for
your password. From a terminal, `sudo /usr/local/karaokemachine/uninstall.sh`, with `--dry-run` for
the list alone.

The remote-only setup programs come off the same way, and separately. On Windows that is its own
entry, **KM Remote**; on macOS, `/usr/local/km-remote` and **Uninstall KM Remote**. Where you have
both, removing one leaves the other alone.

On Debian, `sudo apt remove karaokemachine`, and where the appliance service is enabled, removing the
package stops and disables it first. `/var/lib/karaoke` stays even on a `purge`, because it holds the
catalog and settings, and `sudo userdel -r karaoke` removes it. The portable `.tar.gz` has nothing to
uninstall: delete the folder. Where you ran its `install.sh` for a menu entry,
`./install.sh --uninstall` takes that entry away.

---

## Using it

Start it, type a song number, and it plays. That is the whole of normal use: the screen shows the
words and nothing else, and a phone does everything except play the song.

### The keyboard

Mid-song a strip of buttons appears along the bottom whenever you press a key, and **each says which
function key presses it**. The function keys work whether or not the strip is on screen, and each
has a letter key that does the same.

| Key | Letter key | What it does |
|---|---|---|
| `F1` | `Space` | Pause and resume |
| `F2` | `,` | Go back ten seconds |
| `F3` | `.` | Go forward ten seconds |
| `F4` | `N` | Skip to the next song |
| `F5` | `R` | Start the song again |
| `F6` | `Q` | Show the queue |
| `F7` | `-` | Lower the key |
| `F8` | `+` | Raise the key |
| `F9` | `M` | Turn the guide melody on and off, for a song that has one |
| | `W` | Show the next wallpaper |
| | `I` | Show the address and QR code |
| | `F` | Fill the screen |
| | `T` | Keep the window in front of everything else |
| | `D` | Turn demo mode on and off |
| `0`–`9` | | Type a song number, on the number row or the keypad |
| `Enter` | | Queue the song number |
| `Backspace` | | Correct the song number |
| `Delete` | | Clear the song number |
| `F10` | | Open the packages folder |
| `Ctrl+F10` | | Read the packages folder again |
| `F11`, or `Ctrl+F11` on a Mac | | Open the remote in this computer's browser |
| `F12` | | Show how the picture is doing |
| `Ctrl+F12` | | Stop the strip of buttons timing out |
| `Ctrl+Q` | | Stop the machine |

**`T`** is for a machine sharing a screen with a browser or a chat window rather than driving a
television. The machine remembers how you left it. Some Linux desktops do not let an application
place itself, and there the key does nothing.

**`D`** is the machine picking songs and playing them by itself. A room hears what the box holds
without working out how to drive it. Turning the mode on starts a song straight away, rather than
waiting out the usual minute of quiet.

Where something is already playing or queued, the mode still goes on and takes over when the queue
runs out, and the screen says so. Turning it off leaves the song that is playing alone, and `N` is
what means stop. It lasts until the machine closes, and the `/admin/` page is where you make it
permanent. With the mode on, **`N` on a quiet machine starts the next song rather than waiting**.
With the mode off it says `nothing is playing`.

**`F10`** opens the folder in whatever this computer uses for folders, and a machine with nothing to
open a folder in says so. **`Ctrl+F10`** lets a package you just copied in play without a restart.

**`F11`** opens the same page a phone gets. It says so if the remote is switched off or the web
server never started. macOS keeps `F11` for itself, and the panel names whichever key this computer
answers to.

**`F12`** shows frames a second, how long each took to draw, and whether sound or video ran short. It
is the same measurement `--frame-stats` writes to the log, but it writes nothing to the log itself.
**`Ctrl+F12`** suits somebody changing how the strip looks rather than singing. The machine remembers
neither key when it closes.

**`Ctrl+Q`** closes the application on a computer. On the Linux appliance it switches the box off
instead, exactly as its power button does, because that box has nothing to go back to. Plain `Q`
is the queue, and the modifier keeps the two apart.

The machine draws a hint only where there is a keyboard, so a television or a phone gets the same
buttons without them.

### Getting songs in

**Songs arrive in packages.** A `.kmpkg` is one file carrying its own songs, queue numbers, titles,
artists and analysis. Videos and MP3+G pairs travel inside it, so nothing ever sits beside it to
copy.

**The shortest way in is to double-click the package.** It installs and the machine says so, on its
own screen where one is running, and otherwise by starting up with the songs already in. The Windows
installer offers to set this up, and macOS arranges it as it installs the app. On Linux, and for the
portable Windows folder, run `karaokemachine --register` once. Neither needs administrator rights.

**Or drag the file onto the machine's window**, which needs no setting up. It says `installing …`
across the top, then how many songs went in. A rebuilt package of the same name replaces the old one
rather than piling up beside it, and it refuses any other kind of file.

Either way the file lands in the packages folder, so it survives a restart even if you tidy the
original away. Two other ways work as well: put the file in that folder yourself, which `F10` opens
for you, or hand it to [the API](#the-http-api-and-the-network).

**Taking a package out of that folder uninstalls it**, at the next start or the next `Ctrl+F10`. The
folder is what says which packages are installed, and removing a package from the **Songs** page
deletes its `.kmpkg` to match.

**There is one package you can download: sixteen Christmas carols.** Silent Night, Joy to the World,
The First Noel, Hark! The Herald Angels Sing, O Come All Ye Faithful, What Child Is This and ten
more. Every one is public domain, four or five verses each, seventy-six minutes of singing in a
28 KiB file. It is a **separate download, and no install carries it**: a new install starts empty,
because you supply your own songs.

It is the only pack of its kind, because a karaoke MIDI is rarely free to distribute. One file is
four or five works at once: the tune, the arrangement, the words, any translation, whoever entered
the notes. Almost nothing has all of those in the public domain, and carols do. `CREDITS.md` beside
the pack names every source.

**A package holds at most 999 songs.** This is not a storage limit: the machine holds up to a
thousand packages, so nearly a million songs. It is there to encourage curating a volume rather than
packaging a whole folder at once. The corpus this was built against is a large one.

It also makes the numbers work. A song's number is **`bank × 1000 + slot`**: the slot is what the
package numbered it, and the bank is the block of a thousand it sits in. Two packages that both
number a song 500 cannot clash: one is 3500, the other 611500.

**A package's bank comes from its id, so its numbers are the same on every machine.** Install the
same volume here and at a friend's house, in any order, and it takes the same bank both times. A
printed song list therefore travels with the file, and a package that asks for a bank another one
holds takes the next one instead.

**A block is yours to choose**, from 1 to 9999, and bank 0 is the machine's own and holds no package.
Put the volume that gets sung from into a low block, and its songs dial in four digits instead of
six. The **Songs** page at `http://127.0.0.1:8177/admin/` has a box for it and needs nothing typed;
[the API](#the-http-api-and-the-network) is the other way.

A change renumbers every song in that package, so **anything already printed goes stale**. The machine
refuses while a song is playing or queued, because the queue holds numbers. Do it once, when the
package goes in.

**A folder of your own files becomes a package** with [two commands](#getting-a-corpus-into-shape).

### The remotes

**The machine serves a remote at its own address.** Any phone on the network reaches it with nothing
to install. Search the catalog, queue a song, see what is playing, use whatever controls the song
allows. The idle screen shows the address and a QR code, because nobody should type an IP address at
a party.

**The offline remote is a separate program**, `km-remote`. It keeps its own copy of a machine's
catalog, so browsing, searching and favorites work **with the machine switched off**. It finds a
machine on the network and remembers it. It also adds favorites and an A–Z picker, which the
machine's own remote does not have.

**It has its own download on every platform it runs on**: an APK, an `.ipa`, and a small setup
program for Windows and macOS. A computer that will never play a song needs none of the rest, and the
remote is what a guest holds for somebody else's machine.

### Watching it in another room

**`--stream` draws the screen for an encoder instead of for a television.** The machine opens no
window and serves what it would have shown, picture and sound together, as one continuous HLS stream.
It is a way of running the machine rather than a build of it, so it writes nothing down. A machine
started this way opens its television the next time, and it refuses `--headless`, which turns the
screen off rather than sending it somewhere.

**The playlist is the interface, and the page above it is a convenience.** Anything able to follow a
URL plays `http://<the machine>/stream/live.m3u8`: a television's own media pipeline, VLC, Kodi, a
set-top box. None of them needs a browser, or any knowledge that this project exists.
`http://<the machine>/watch/` is the same stream on a page, for the sets where opening an address is
easiest. A machine that is not streaming serves neither.

**Nothing has to be typed to start it.** A Start Menu entry on Windows, the stream action on the
Linux desktop menu, and `KM Stream.app` on macOS each start the machine streaming. Each sits beside
the launcher that opens its television, wearing the machine's mark with a broadcast badge in the
corner. That badge is how you tell the two apart.

**A streaming run puts an icon in the notification area on Windows and the menu bar on macOS.** A
program with no window has no other way to say that it is running. The icon names the address a phone
can reach, and its *Remote*, *Watch* and *Setup* entries open the three pages the machine serves. It
follows the address rather than fixing it at startup, so a machine started before its Wi-Fi came up
still names the right one.

**Two things it costs.** The stream runs several seconds behind, which is invisible while it is the
only screen and the only sound in the room. What it takes is control: pause, and the music runs on
for the length of the buffer. It also carries the backing track and never a singer, because hardware
mixes the microphones downstream of anything the machine can see.

### Setting it up from a browser

**The machine's address plus `/admin`** is the page for whoever owns the machine, as opposed to
whoever is singing. Five tabs:

* **This machine** — its name, where to reach it, the password, demo mode and what the screen speaks.
* **Songs** — what is installed, how many songs each package holds, and which thousand its numbers sit
  in. Add or remove a package.
* **Pictures** — how many are in the rotation, which is showing, and a way to add more. It tells you,
  before the first one, that your own pictures replace the ones that came with the machine.
* **Sound** — the sound banks on the machine, which one is playing, where the sound comes out, and a
  way to add a `.sf2`.
* **Problems** — packages the machine could not load, and anything else that is wrong, each with the
  control that fixes it. The tab carries a count, so you see it from wherever you were.

**You do not have to remember the address.** The remote the machine serves carries the link at the
foot of its Setup tab. A phone that reaches the machine at all reaches this page.

**The machine shows the password on its own screen.** It generates a six-digit PIN at first start and
displays it beside its address. Anyone in the room can read it, and nobody outside can. Change it on
the *This machine* tab, and until you do every tab shows a reminder linking there. The screen carries
it rather than a file, because a machine under a television has no keyboard.

That tab can also sign out every phone and browser at once, which is what a lost phone calls for. It
turns debugging on and off as well.

The singer's remote offers only what a singer needs: search, queue, and the controls a song allows.
Nothing on it can delete songs.

### The song book

**The song book is a printable PDF of everything installed**, for finding a song without the screen
or a phone. Four columns — artist, number, title and first line — sorted by artist within a section
per language, like a commercial machine's ring-binder book. It reads the
catalog as it stands, so start the machine once after adding a package, and `--book-name` sets the
heading, which defaults to `KaraokeMachine`.

```sh
karaokemachine --song-book ./songbook.pdf
karaokemachine --song-book ./songbook.pdf --book-name "Sitting room"
```

---

## For a technical reader

Everything below is for talking to the machine rather than singing on it. It covers the flags, the
formats a song file may be in, the API, and what finds a machine on the network. The tools that turn
a folder of files into a package come after. Compiling it is [`BUILDING.md`](BUILDING.md)'s subject.

### The command line

```sh
karaokemachine --show-paths              # settings, catalog, packages folder, assets
karaokemachine --set-password hunter2    # change the admin password (or the /admin page, or the API)
karaokemachine --reset-password          # back to a fresh PIN, shown on the machine's screen
karaokemachine --reset-sessions          # sign every phone and browser out at once
karaokemachine --set-name "Living Room"  # what phones call this machine on the network
                                         # (or set it at http://<the machine>/admin)
karaokemachine vol1.kmpkg                # install a package (what a double-click does)
karaokemachine --register                # make .kmpkg files open with this machine
karaokemachine --unregister              # ...and take that back off
karaokemachine --list-audio-devices      # output devices and their stable ids
karaokemachine --song-book ./songbook.pdf   # every installed song, as a PDF to print
karaokemachine --headless                # no window: API + engine + catalog
karaokemachine --stream                  # no window: the screen goes to http://<the machine>/watch/
karaokemachine --fullscreen              # fill the screen this run, whatever settings say
karaokemachine --windowed                # ...and open in a window instead
karaokemachine -v                        # say more; -vv for everything
karaokemachine --frame-stats             # fps, frame times and decode, once a second
karaokemachine --log-file                # also write this run's log to a file
```

`--show-paths` names the packages folder, which is where songs go. `--log-file` is for when something
went wrong: double-clicking opens no console, so the machine has nowhere else to say what happened.
It writes one file per run into a `logs` folder beside the catalog, and it keeps the ten newest.

`--fullscreen` and `--windowed` move one run and write nothing down. A machine an installer put there
fills the screen, one you built yourself opens in a window, and either takes the other flag.
`--stream` moves one run the same way and refuses `--headless` beside it;
[Watching it in another room](#watching-it-in-another-room) says what it serves, and where.

### Song files and their formats

- **MIDI and KAR**, in all three karaoke conventions that exist in the wild — Soft Karaoke
  `@`-headers, `Lyric` meta-events, and a named text track.
- **A video song** is H.264 with AAC audio, in an MP4.
- **An MP3+G song** is an MP3 with a `.cdg` of the same stem. This machine draws the CD+G graphics
  itself, in Rust.
- **An UltraStar song** is a `.txt` beside the MP3 it names. The machine reads its timed words and
  discards its pitches.
- **A song's language is an ISO 639-1 code**, so you can ask a catalog what Portuguese it holds.

### Text encodings and writing systems

- **The machine detects a legacy encoding rather than assuming one**: Shift-JIS and the Windows code
  pages.
- **CJK works**: a Japanese or Chinese song uses a font from the system.
- **No shaped scripts, and no right-to-left.** Thai, Arabic and Indic need a text shaper this build
  does not include.

### The HTTP API and the network

- **An HTTP API with a WebSocket event stream**, covering search, queue, transport, settings,
  packages, wallpapers, demo mode, microphones and audio output.
- **The URL prefix says which routes need the machine's password.** Everything that reconfigures it
  lives under `/api/v1/admin/`, and everything a singer does sits outside.
- **mDNS finds it on the network**, and a `/discover` endpoint answers as well.
- **A package installs over the API without a restart**, at `POST /api/v1/admin/packages`. Removing
  one over the API deletes its `.kmpkg` to match.
- **The song book is a route too**: `GET /api/v1/songs/book.pdf`, which takes `?language=`,
  `?package=` and `?name=`. `km-pack book` prints one from `.kmpkg` files no machine has seen yet.

Setting [a package's bank](#getting-songs-in) is an admin route, so it takes the machine's password
first:

```sh
# Songs 1001-1999 instead of 611001-611999, for the volume that gets sung from.
TOKEN=$(curl -s -H 'content-type: application/json' -d '{"password": "<the machine password>"}' \
        http://127.0.0.1:8177/api/v1/admin/login | jq -r .token)
curl -X PUT -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
     -d '{"bank": 1}' http://127.0.0.1:8177/api/v1/admin/packages/brasil/bank
```

### Getting a corpus into shape

<p align="center">
<img src="docs/images/package-builder-songs.png" width="90%"
     alt="km-package-builder in a browser: a wide filter panel whose narrowed list can be saved
under a name, over a table of songs with columns for artist, title, language, length, suitability,
your own rating, melody and duplicate count, and per-row buttons to play a song, file it in a
favorite, edit its title and artist, and look it up on YouTube">
<br><sub><b>km-package-builder</b>, for getting a folder of files into shape before it is packaged.</sub>
</p>

- **`km-package-builder`**, a local web server over a folder of source files. Browse, search *the
  lyrics themselves*, rate, fix names, group duplicates, pick songs into packages by hand.
- **`km-pack`** builds and validates packages, **`km-lyrics`** dumps one file's parsed timeline, and
  **`km-wallpaper-pack`** builds a wallpaper set, keeping what lyrics stay readable over.

Making a package from a folder is two commands:

```sh
# Curation: a local web server over a folder of source files, at http://127.0.0.1:8178.
# Browse, search the lyrics themselves, rate, fix the names, group duplicates, and select songs
# into packages by hand. `--init` is the only thing that creates its database, so a wrong folder
# is an error rather than an empty index.
km-package-builder ./songs --init --scan --open
km-package-builder ./songs                      # once it has a database

# Packaging, in two steps: describe the folder, edit the description, build it. `build` takes a
# description and never a folder, so what goes in a package is written down where you can read it.
#
# A folder of more than 999 songs is refused rather than truncated: split it, or narrow it with
# the flags below.
km-pack spec ./songs --out vol1.kmspec.yaml     # what is here, and what it is called
km-pack spec ./songs --out vol1.kmspec.yaml --min-suitability 6 --require-lyrics   # ...or be choosier
km-pack build vol1.kmspec.yaml                  # build what the description says
km-pack check vol1.kmpkg                        # validate + report suitability

# One file's parsed lyric timeline and analysis, when a song does not behave.
km-lyrics dump ./song.kar
```

---

## Building it

Compiling, testing, packaging and releasing are in [`BUILDING.md`](BUILDING.md).

---

## Documentation

| File | What it is |
|---|---|
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

## License

`MIT OR Apache-2.0`, at your option — what every crate in the workspace declares. The texts are
[`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).

Unless you say otherwise, a contribution you deliberately submit falls under the same dual license,
with no additional terms.

**Only the application is covered.** A release carries the ffmpeg libraries under the LGPL, with
their own terms in the folder that holds them. The instrument bank has its own license.

**The wallpapers are photographs, and they are CC0** — seven of them, in
`assets/wallpapers/default-wallpapers.zip`. `CREDITS.md` beside it names each photographer and source
page, and says that every image was cropped, resized, blurred and vignetted.

**A pack you build yourself with [`tools/cmd/assets/km-wallpaper-pack`](tools/cmd/assets/km-wallpaper-pack)
is separate, and no release includes one.** Of its three sources, only Openverse produces a pack that
may travel on. A Pixabay or Pexels pack is for the machine that built it, and the tool says so when
it finishes. `manifest.json` records each image's license either way. See
[`Where a wallpaper pack's photographs may come
from`](docs/decisions/repository.md#where-a-wallpaper-packs-photographs-may-come-from).

## Author

Rangel Reale (realerangel@gmail.com)
