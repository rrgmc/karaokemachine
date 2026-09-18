<h1>
  <img src="icon/icon-64.png" width="48" align="absmiddle"
       alt="The KaraokeMachine icon: the letters KM on a near-black plate, over angular bands of
color, with the M in amber">
  KaraokeMachine
</h1>

A karaoke machine that behaves like a commercial home unit: pick a song by number, it plays, the
words highlight in time. It runs full-screen on a television, and phones on the same network act as
remotes — search the catalog, queue a song, change the key, skip.

A song is a **MIDI file with embedded karaoke lyrics**, a **video file**, an **MP3+G pair** — an
MP3 with a `.cdg` of the same stem beside it, which is what most commercial karaoke discs hold — or
an **UltraStar song**, the `.txt` a singing game times its words in and the MP3 it names. An MP3 on
its own is not a song: it has no words in it.

Native on Windows, macOS and Linux, on Android, and on an iPhone and an iPad.

The site is **[rrgmc.github.io/karaokemachine](https://rrgmc.github.io/karaokemachine/)** — the
pictures, and the download.

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
- **The first line or two of each song's words**, so a package carries something a person can
  recognize a song by. It skips the studio-name banner many karaoke files open with; the rules for
  that were set against a real corpus.

**Playing**

- Full-screen on a television, drawn by SDL3. On Linux it draws straight to DRM/KMS from a bare TTY,
  with no desktop installed.
- **Or on a television in another room.** Started with `--stream` the machine opens no window and
  serves what it would have shown — the words, the highlight moving through them, the wallpaper, the
  queue and the music — at one address. **The address is the whole of it**: a smart television, a
  phone or a computer opens `/watch/`, and anything that plays a playlist takes
  `http://<the machine>/stream/live.m3u8` and needs no browser.
  [Further down](#watching-it-in-another-room).
- **Words highlight in time**, syllable by syllable, on the sequencer's own clock.
- **Transpose and tempo** per song, a **guide melody** that can be muted, and per-song defaults.
- **A lyric timing offset in milliseconds**, adjustable mid-song, because a television adds picture
  lag and a mixer takes the sound out early. It moves the *highlight* only — never the audio, because
  the microphones are in that audio.
- **Wallpapers** cycling with a crossfade, from a folder of stills. A zip in that folder counts as a
  folder of images.
- **A queue** with singer names, shown over whatever is playing.
- **A demo mode**, off unless asked for: after a minute of quiet the machine starts a song of its
  own, and another when that one ends. Queueing takes the deck off it at once — choosing a song is
  what makes it play, and nothing has to be skipped first.

**Remotes**

- **The machine serves a remote at its own address** — search, queue, now playing, and the controls a
  song allows. Any phone on the network, no app, and a QR code on the idle screen.
- **A standalone offline remote** keeping its own copy of a machine's catalog, so browsing, searching
  and favorites work with the machine switched off.
- **One admin password, which the machine gives itself and shows on screen.** A six-digit PIN at
  first start, on the idle screen beside the address. Everything that reconfigures the machine needs
  it — what packages and pictures are installed, where a package's numbers start, the machine's name,
  its audio output, demo mode — and everything a singer does needs nothing.

**Giving the machine pictures and instruments**

- **KaraokeMachine Admin** (`km-admin`) finds photographs the lyrics stay readable over — measured
  in the exact band of the screen the words occupy — and General MIDI banks from a table of
  sixty-three, then sends either to the machine.
- **It also sends a file you already have**: a package, a bank, a photograph of your own. On a
  television box there is no shell and no file manager that reaches where the machine looks, so
  otherwise there is no way to hand it one.
- **The machine cannot download these itself.** It may have no internet connection, and it should not
  store your accounts. Everything km-admin downloads is saved on your computer as well as sent, so a
  machine that is switched off can be sent to later. A file you supply yourself is passed straight
  through and is not copied.
- **Pictures come from Openverse by default**, which needs no account and whose packs may be
  redistributed. Pixabay and Pexels need your own API key, and their terms, which do not allow
  redistribution, are shown next to the key field.

**Not supported, by decision**

No scoring of singers. No bare audio files, and no CD+G disc images — only the file pair. No video
wallpapers: a video song is not a video background. No pitch shifting of audio. No microphone
processing in the app; mic audio is mixed in hardware. No Thai, Arabic or Indic words on the screen,
and no right-to-left. Japanese and Chinese are drawn.

These are deliberate decisions rather than missing features. To propose a change, start with
[`docs/decisions/`](docs/decisions/).

The HTTP API, discovery on the network, the file formats and the tools that make a package are
[further down](#for-a-technical-reader).

---

## Installing

**The downloads are on the [release page](https://github.com/rrgmc/karaokemachine/releases)**,
one file per platform, with the carol package beside them as a separate download. The table below says
what each carrier is. Building from source is the other way in — [`BUILDING.md`](BUILDING.md) is how,
and it is one command per platform once prerequisites are in.

| Platform | What you get |
|---|---|
| **Windows** | A setup program, `karaokemachine-setup-<version>-windows-x86_64.exe`, holding all seven products behind component checkboxes. It installs **per-user** into `%LOCALAPPDATA%\Programs` and raises no UAC prompt, and offers to put itself on your `PATH` and to open `.kmbuild` files. Or a **portable folder**: unzip and run. |
| **macOS** | An installer package, `karaokemachine-setup-<version>-macos-<arch>.pkg` — the same seven products behind six component ticks. Applications go to `/Applications`, command-line tools to `/usr/local/karaokemachine` with symlinks in `/usr/local/bin`. It asks for your administrator password once and fetches nothing. Or `Karaoke Machine.app` on its own. |
| **Windows or macOS, the remote alone** | A second, small setup program: `km-remote-setup-<version>-windows-x86_64.exe` (about 5 MB) or `km-remote-setup-<version>-macos-<arch>.pkg`. It installs KM Remote and nothing else, for a computer that is never going to play a song — a laptop somebody holds while somebody else's machine does. It sits happily beside a full install and is removed on its own. |
| **Debian, Ubuntu** | A `.deb`. Its ffmpeg and font dependencies are named rather than bundled. It installs as an ordinary application — menu entry, icon, and `karaokemachine` as a command — and carries the television-appliance service, switched off. A second `.deb`, `karaokemachine-tools`, holds the package builder, the offline remote and the picture-and-bank tool; name both files in one `apt install` to get them, or take the machine alone for a box under a television. |
| **Any Linux** | A `.tar.gz`. Unpack anywhere, run it, delete it — no root, no package manager. It carries its own ffmpeg, because a folder can name no dependency. |
| **Android, Google TV** | An APK carrying both ABIs, so it installs on a phone and on a television. |
| **iPhone, iPad** | An `.ipa` for the machine and one for the remote, both **unsigned**: iOS takes no signature from a stranger, so you sign it yourself with your own Apple ID. It is the machine itself — the same synthesizer, catalog, display and API — and songs arrive through the Files app. The section below is the procedure. |

**Every carrier also installs a second launcher that starts the machine streaming**, for a television
in another room rather than the one this box is plugged into. It is a way of starting the machine
rather than a thing to download, so it is not a row in the table above —
[Watching it in another room](#watching-it-in-another-room) is what it serves.

**System-wide on macOS, per-user on Windows.** Everything the Windows installer configures beyond the
files lives in that user's registry, while on macOS the `.kmbuild` association is declared by a bundle
in `/Applications` and the `PATH` entry is a symlink in `/usr/local/bin` — both of which belong to the
machine.

### On Debian, it is also an appliance — if you ask

**Installing the `.deb` gives you an ordinary application**: a menu entry, an icon, and
`karaokemachine` as a command. Nothing starts automatically and nothing runs at boot.

The package also includes a systemd service, **switched off by default**. Enabling it turns the
computer into an appliance: it starts on its own when the power returns and draws straight to the
screen from a bare virtual terminal, with no desktop installed. One command turns it on:

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

You need an Apple ID — the one you already use on the phone is fine — and one of the free signing
tools: **[Sideloadly](https://sideloadly.io)** on macOS or Windows, or
**[AltStore](https://altstore.io)**, which installs from the phone afterwards.

1. Download `karaokemachine-<version>-ios-unsigned.ipa`, or
   `km-remote-<version>-ios-unsigned.ipa` for the remote alone.
2. Plug the phone or the iPad into the computer and unlock it.
3. Open Sideloadly, drag the `.ipa` onto it, enter your Apple ID and press **Start**. It signs the
   app for your devices and installs it.
4. On the device, open **Settings → General → VPN & Device Management**, tap your Apple ID under
   *Developer App*, and tap **Trust**. Until you do, the application will not open.
5. Launch it once with the network reachable. The certificate is verified on that first run.

Two limits come from the Apple ID rather than from the application:

- **A free Apple ID signs for seven days.** After that the app stops opening and you repeat step 3;
  nothing inside it is lost, because your songs, settings and catalog stay on the device. A paid
  Apple Developer account signs for a year. AltStore renews in the background if you leave it
  installed.
- **Three applications at a time** on a free Apple ID. The machine and the remote are two of them.

**The remote is the one most people want.** It is small, it has none of the two differences below,
and it is what a guest holds while somebody else's machine plays.

### On an iPhone or an iPad, two things are different

**It does not announce itself on the network.** Apple grants the multicast entitlement only after a
reviewed request, so the machine advertises nothing and a remote will not find it by itself. The iOS
remote sweeps the network for machines and finds it anyway; the Android remote and a browser need the
address, which the idle screen shows.

**A machine nobody is singing on stops answering when it goes to the background.** iOS suspends an
application that is not playing audio. A song that is playing keeps playing with the screen off,
which is the case that matters in a room, and a remote loses an idle machine until you bring it back
to the front.

### Removing it

**Your songs, settings and catalog are left where they are**, whichever way you remove it, and the
uninstaller names the folders on its way out.

On Windows, uninstall as you would anything else. On macOS, open `/usr/local/karaokemachine` and
double-click **Uninstall KaraokeMachine**; it lists what it will remove, asks, and only then asks for
your password. From a terminal, `sudo /usr/local/karaokemachine/uninstall.sh`, with `--dry-run` for
the list alone.

The remote-only setup programs are removed the same way and separately. On Windows that is its own
entry, **KM Remote**; on macOS, `/usr/local/km-remote` and **Uninstall KM Remote**. Where both are
installed, whichever you remove leaves the other alone.

On Debian, `sudo apt remove karaokemachine` — if the appliance service was enabled, removing the
package stops and disables it first. `/var/lib/karaoke` stays even on a `purge`, because it holds the
catalog and settings; `sudo userdel -r karaoke` removes it. With the portable `.tar.gz` there is
nothing to uninstall — delete the folder — though if you ran its `install.sh` for a menu entry,
`./install.sh --uninstall` takes that entry away.

---

## Using it

Start it, type a song number, and it plays. That is the whole of normal use: the screen shows the
words and nothing else, and everything except playing a song is done from a phone.

### The keyboard

Mid-song a strip of buttons appears along the bottom whenever anything is pressed, and **each says
which function key presses it**: `F1` pauses, `F4` skips, `F6` shows the queue, `F7` and `F8` change
the key. The function keys work whether or not the strip is on screen, and the older letter keys still
work — `Space`, `N`, `R`, `Q`, `M`, `,` and `.` for ten seconds either way, `+` and `-` for the key,
`W` for the next wallpaper, `I` for the address and QR code, `F` for fullscreen. Song numbers are
typed on the number row or the keypad; `Enter` queues, `Backspace` corrects, `Delete` clears.

**`T` keeps the window in front of everything else**, for a machine sharing a screen with a browser
or a chat window rather than driving a television. Press it again to let the window fall back into
the stack. The machine remembers how you left it, so a window left in front starts in front. Some
Linux desktops do not let an application place itself, and there the key does nothing.

**`D` turns demo mode on and off** — the machine picking songs and playing them by itself, one after
another, so a room can hear what the box holds without working out how to drive it. Turning it on
starts a song straight away rather than waiting out the usual minute of quiet; if something is
already playing or queued the mode still goes on and takes over when the queue runs out, and the
screen says so. Turning it off leaves the song that is playing alone — `N` is what means stop. It
lasts until the machine is closed; the `/admin/` page is where you make it permanent.

With the mode on, **`N` on a quiet machine starts the next song rather than waiting** — the same key
that takes a demo's turn while one is playing, doing the same thing to a silence. With the mode off
it says `nothing is playing`, as it always does when there is nothing to skip.

**Three keys past the strip do the things that are not about the song.**

**`F10` opens the packages folder** in whatever this computer uses for folders — the window that opens
is the answer, and on a machine with nothing to open a folder in it says so. **`Ctrl+F10` reads that
folder again**, so a package just copied in is playable without restarting.

**`F11` opens the remote** in this computer's browser — the same page a phone gets. It says so if the
remote is switched off or the web server never started. **On a Mac it is `Ctrl+F11`**, because macOS
keeps `F11` for itself; the panel names whichever one this computer answers to.

**`F12` shows how the picture is doing** — frames a second, how long each took to draw, and whether
sound or video ran short. The same measurement `--frame-stats` writes to the log. Press again to put
it away; it writes nothing to the log unless you asked separately. **`Ctrl+F12` stops the strip of
buttons timing out**, which is for somebody changing how the strip looks rather than for singing;
press it again to give the six seconds back. Neither is remembered when the machine closes.

**`Ctrl+Q` stops the machine.** On a computer it closes the application, the way Control-Q does
everywhere else. On the Linux appliance — a box under a television with nothing to go back to — it
switches the box off instead, cleanly, exactly as pressing its power button does. Plain `Q` is still
the queue; the modifier is what keeps the two apart.

Hints are drawn only where there is a keyboard, so a television or a phone gets the same buttons
without them.

### Getting songs in

**Songs arrive in packages.** A `.kmpkg` is one file carrying its own songs, queue numbers, titles,
artists and analysis — videos and MP3+G pairs included, so there is never anything beside it to copy.

**The shortest way in is to double-click the package.** It installs and the machine says so, on its
own screen if one is running and otherwise by starting up with the songs already in. The Windows
installer offers to set this up and macOS arranges it when the app is installed; on Linux, and for the
portable Windows folder, run `karaokemachine --register` once. Neither needs administrator rights.

**Or drag the file onto the machine's window**, which needs no setting up. It says `installing …`
across the top, then how many songs went in. Dropping a rebuilt package of the same name replaces the
old one rather than piling up beside it, and anything that is not a `.kmpkg` is refused on screen.

Either way the file is copied into the packages folder, so it survives a restart even if you tidy the
original away. Two other ways work as well: put the file in that folder yourself, which `F10` opens
for you, or hand it to [the API](#the-http-api-and-the-network).

**Taking a package out of that folder uninstalls it**, at the next start or the next `Ctrl+F10`: the
folder is what says what is installed. Removing a package from the **Songs** page deletes its
`.kmpkg` to match.

**There is one package you can download: sixteen Christmas carols.** Silent Night, Joy to the World,
The First Noel, Hark! The Herald Angels Sing, O Come All Ye Faithful, What Child Is This and ten more
— every one public domain, four or five verses each, seventy-six minutes of singing in a 28 KiB
file. It is a **separate download and is not installed with the machine**: a new install starts
empty, because you supply your own songs.

It is the only pack of its kind, because a karaoke MIDI is rarely free to distribute. It is four or
five works at once — the tune, the arrangement, the words, any translation, and whoever entered the
notes — and almost nothing has all of those in the public domain. Carols do. `CREDITS.md` beside the
pack names every source.

**A package holds at most 999 songs.** This is not a storage limit: the machine holds up to a
thousand packages, so nearly a million songs. It is there to encourage curating a volume rather than
packaging a whole folder at once. The corpus this was built against is a large one.

It also makes the numbers work. A song's number is **`bank × 1000 + slot`**: the slot is what the
package numbered the song, 1 to 999, and the bank is the block of a thousand it sits in. Two packages
that both number a song 500 cannot clash — one is 3500, the other 611500.

**A package's bank comes from its id, so its numbers are the same on every machine.** Install
the same volume here and at a friend's house, in any order — it gets the same bank both times, so a
printed song list travels with the file. Nothing is refused for wanting a bank another package has; it
takes the next one.

**A block is yours to choose**, from 1 to 9999 — bank 0 is the machine's own and holds no package.
Put the volume that gets sung from into a low block and its songs dial in four digits instead of six.
The **Songs** page at `http://127.0.0.1:8177/admin/` has a box for it, which is the way that needs
nothing typed; [the API](#the-http-api-and-the-network) is the other.

Every song in that package is renumbered, so **anything already printed goes stale** — and the machine
refuses while a song is playing or queued, because the queue holds numbers. Do it once, when the
package goes in.

**A folder of your own files becomes a package** with [two commands](#getting-a-corpus-into-shape).

### The remotes

**The machine serves a remote at its own address.** Any phone on the same network, nothing to install:
search the catalog, queue a song, see what is playing, and use whatever controls the current song
allows. The idle screen shows the address and a QR code — nobody should be typing an IP address at a
party.

**The offline remote is a separate program**, `km-remote`. It keeps its own copy of a machine's
catalog, so browsing, searching and favorites work **with the machine switched off**. It finds a
machine on the network and remembers it. It also adds favorites and an A–Z picker, which the
machine's own remote does not have.

**It has a download of its own on every platform it runs on** — an APK, an `.ipa`, and a small setup
program for Windows and for macOS. A computer that is never going to play a song needs none of the
rest, and the remote is the half somebody is handed for somebody else's machine.

### Watching it in another room

**`--stream` draws the screen for an encoder instead of for a television.** The machine opens no
window and serves what it would have shown, picture and sound together, as one continuous HLS
stream. It is a way of running the machine rather than a build of it, so it writes nothing down: a
machine started this way opens its television the next time. It cannot be combined with
`--headless`, which turns the screen off rather than sending it somewhere.

**The playlist is the interface and the page above it is a convenience.**
`http://<the machine>/stream/live.m3u8` is what anything able to follow a URL plays — a television's
own media pipeline, VLC, Kodi, a player on a set-top box — with no browser and no knowledge that any
of this exists. `http://<the machine>/watch/` is the same stream on a page, for the sets where
opening an address is what is easiest. A machine that is not streaming serves neither.

**Nothing has to be typed to start it.** A Start Menu entry on Windows, the stream action on the
Linux desktop entry's right-click menu, and `KM Stream.app` on macOS each start the
machine streaming, beside the launcher that opens its television. They wear the machine's mark with
a broadcast badge in the corner, so the two ways of starting it are told apart wherever they sit
side by side.

**A streaming run puts an icon in the notification area on Windows and the menu bar on macOS**,
because a program with no window is otherwise a program with no way to tell it is running. The icon
names the address a phone can reach, and its *Remote*, *Watch* and *Setup* entries open the three
pages the machine serves. It follows the address rather than fixing it at startup, so a machine
started before its Wi-Fi came up ends up naming the right one.

**Two things it costs.** The stream runs several seconds behind. That is invisible while it is the
only screen and the only sound in the room, and what it takes is control: pause, and the music runs
on for the length of the buffer. And it carries the backing track and never a singer, because
microphone audio is mixed in hardware, downstream of anything the machine can see.

### Setting it up from a browser

**The machine's address plus `/admin`** is the page for whoever owns the machine, as opposed to
whoever is singing. Five tabs:

* **This machine** — its name, where to reach it, the password, demo mode and what the screen speaks.
* **Songs** — what is installed, how many songs each package holds, and which thousand its numbers sit
  in. Add or remove a package.
* **Pictures** — how many are in the rotation, which is showing, and a way to add more. It tells you,
  before the first one, that your own pictures replace the ones that came with the machine.
* **Sound** — the sound banks on the machine, which is playing, where the sound comes out, and a way
  to add a `.sf2`.
* **Problems** — packages the machine could not load, and anything else that is wrong, each with the
  control that fixes it. The tab carries a count, so you see it from wherever you were.

**You do not have to remember the address.** The remote the machine serves has the link at the foot of
its Setup tab — so a phone that can reach the machine at all can reach this page.

**The password is shown on the machine's screen.** The machine generates a six-digit PIN at its first
start and displays it beside its address, so anyone in the room can read it and no one outside can.
Change it on the *This machine* tab; until you do, every tab shows a reminder linking there. It is
shown on screen rather than written in a file because a machine under a television has no keyboard.

That tab can also sign out every phone and browser at once, which is what to use for a lost phone,
and turn debugging on and off.

The singer's remote offers only what a singer needs: search, queue, and the controls a song allows.
Nothing on it can delete songs.

### The song book

**The song book is a printable PDF of everything installed**, for finding a song without the screen
or a phone. Four columns — artist, number, title and the first line of the words — sorted by artist
within a section per language, modeled on the ring-binder book a commercial machine ships with. It
reads the catalog as it stands, so start the machine once after adding a package. `--book-name` sets
the heading and defaults to `KaraokeMachine`.

```sh
karaokemachine --song-book ./songbook.pdf
karaokemachine --song-book ./songbook.pdf --book-name "Sitting room"
```

---

## For a technical reader

Everything below is for talking to the machine rather than singing on it: the flags, the formats a
song file may be in, the API and what finds a machine on the network, and the tools that turn a
folder of files into a package. Compiling it is [`BUILDING.md`](BUILDING.md)'s subject.

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
went wrong: double-clicking opens no console, so there is normally nowhere for it to say what
happened. It writes one file per run into a `logs` folder beside the catalog and keeps the ten
newest. `--fullscreen` and `--windowed` move one run and write nothing down: a machine an installer
put there fills the screen, one you built yourself opens in a window, and either can be asked for the
other. `--stream` moves one run the same way and is refused alongside `--headless`:
[Watching it in another room](#watching-it-in-another-room) is what it serves and where.

### Song files and their formats

- **MIDI and KAR**, in all three karaoke conventions that exist in the wild — Soft Karaoke
  `@`-headers, `Lyric` meta-events, and a named text track.
- **A video song** is H.264 with AAC audio, in an MP4.
- **An MP3+G song** is an MP3 with a `.cdg` of the same stem. The CD+G graphics are drawn here, in
  Rust.
- **An UltraStar song** is a `.txt` beside the MP3 it names. Its timed words are read and its
  pitches are discarded.
- **A song's language is an ISO 639-1 code**, so a catalog can be asked what Portuguese it has.

### Text encodings and writing systems

- **Legacy encodings are detected rather than assumed** — Shift-JIS and the Windows code pages.
- **CJK is supported**: a Japanese or Chinese song uses a font from the system.
- **No shaped scripts, and no right-to-left.** Thai, Arabic and Indic need a text shaper this build
  does not include.

### The HTTP API and the network

- **An HTTP API with a WebSocket event stream** covering search, queue, transport, settings, packages,
  wallpapers, demo mode, microphones and audio output.
- **Which routes need the machine's password is fixed**: everything that reconfigures it lives under
  `/api/v1/admin/`, and everything a singer does is outside.
- **Found on the network by mDNS**, plus a `/discover` endpoint.
- **A package installs over the API without a restart** — `POST /api/v1/admin/packages` — and
  removing one over the API deletes its `.kmpkg` to match.
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

- **`km-package-builder`**, a local web server over a folder of source files: browse, search *the
  lyrics themselves*, rate, fix names, group duplicates, and select songs into packages by hand.
- **`km-pack`** builds and validates packages, **`km-lyrics`** dumps one file's parsed timeline, and
  **`km-wallpaper-pack`** builds a wallpaper set filtered to what lyrics stay readable over.

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

Unless you say otherwise, any contribution you deliberately submit for inclusion shall be dual
licensed as above, with no additional terms.

**Only the application is covered.** The ffmpeg libraries included in a release are LGPL and ship
their own terms in the folder that holds them, and the bundled instrument bank has its own license.

**The wallpapers are photographs, and they are CC0** — seven of them, in
`assets/wallpapers/default-wallpapers.zip`, with `CREDITS.md` beside it naming each photographer, its
source page and the fact that every image was cropped, resized, blurred and vignetted.

**A pack you build yourself with [`tools/cmd/assets/km-wallpaper-pack`](tools/cmd/assets/km-wallpaper-pack)
is separate, and no release includes one.** Of its three sources, only Openverse produces a pack that
may be redistributed; a Pixabay or Pexels pack is for the machine that built it, and the tool says so
when it finishes. `manifest.json` records each image's license either way. See
[`Where a wallpaper pack's photographs may come
from`](docs/decisions/repository.md#where-a-wallpaper-packs-photographs-may-come-from).

## Author

Rangel Reale (realerangel@gmail.com)
