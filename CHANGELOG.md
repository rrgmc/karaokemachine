# Changelog

What a release of the machine, its remotes and its tools gives the person using them. The format
is [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the numbering is
[semantic versioning](https://semver.org/spec/v2.0.0.html). One number covers every program here, so
a release moves all of them together.

Entries are written for somebody who has the machine. Why any of it is the way it is lives in
[`docs/decisions/`](docs/decisions/), and how it is built in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## [Unreleased]

### Added

- **A portable copy for Windows and Linux.** Unpack `karaokemachine-portable` anywhere and run it.
  Every program is in it, and each keeps its settings and songs in the `data` folder inside. It
  reads and writes nothing in your user profile, so it runs beside an installed copy without
  changing it. The `packages`, `soundfonts` and `wallpapers` folders are there already, each with a
  note saying what goes in it.
- **The portable copy says how to set it up.** Rename one file to have the machine download the
  recommended instrument bank on its next start. A starter settings file shows the settings people
  change most, and the folder's README lists each program's port.
- **A site's song files become a package in one command.** `km-site-pack` reads the pages of a site
  you name and downloads the `.kar`, `.mid` and `.midi` files they link. It opens `.zip` archives,
  and builds a package from the songs that have words. It honours the site's `robots.txt` and waits
  between requests.
- **The curation tool's Folders page shows each folder's average suitability.** The average covers
  the same songs as the count beside it. It appears after the next scan that changes a file, and a
  dash stands in until then.
- **A lyric sync editor puts words on a MIDI file.** `karaokemachine --sync song.mid --sync-words
  words.txt` plays the song while you press Space on each word. It writes a new `.kar` and never
  changes the MIDI file. Review shows the words as the machine draws them. There you can move a word,
  end it before a pause, or move every word onto the notes of the vocal line. Without a words file
  it opens the words a karaoke file already has, and `--sync-continue` goes on from a file saved
  part-way.

### Changed

- **A word can end before the pause after it.** A karaoke file that marks the end of a word with an
  empty event stops that word's highlight there. The highlight does not creep through the pause to
  the next word.

### Changed

- **Every song counts you back in after a break.** A bar above the next line fills during a long
  pause and is full when the line starts. A MIDI or `.kar` song shows it too, after its intro and
  after a solo of four bars or more.

### Fixed

- **The recommended instrument bank downloads again.** Its publisher replaced the file, and the
  machine refused the new one. The machine now takes the publisher's current release.
- **A bank download that fails says what to do.** The message on the screen tells you to try again
  and then to update. The README of the installed and the portable copy says how to get the bank
  yourself.

## [1.20.0] - 2026-10-06

### Added

- **The package builder's song list shows each song's first words.** Tick *show first words* for a
  column with the line the song book prints. The similar-names and same-words pages open with it
  ticked, so two files matched as one song can be told apart at a glance.
- **KM Simple Package leaves out every song below 8 with one button.** It sits beside the song
  count and covers every page. A song with no suitability stays in, and you can keep any song again
  by hand.

### Changed

- **Words written all in capitals are drawn in sentence case.** A screen of capitals is hard to read
  at singing speed. A credit line that keeps both cases stays as the file wrote it. Packages already
  built draw the new case.
- **A MIDI file takes its title from the file name where its own title is not one.** That covers a
  track named for an instrument, such as `Piano` or `BASS`, and an eight-letter abbreviation of the
  file name.
- **More songs get a guide melody.** The search finds a melody doubled in harmony, and one timed
  loosely against its words. It also finds one in a file that types its words a letter at a time.
- **Every song links to a YouTube search**, in the package builder and on the remote. The search
  takes the title of a song with no artist, and the file name of one with no title.
- **A saved filter in the package builder keeps its page.** It opens where it was saved, and the
  rewrite button on its chip stores the page on screen.
- **The rewrite button on a saved filter asks first.** One misclick replaced a saved position.
- **KM Simple Package shows the page links above the song list as well as below it.**

### Fixed

- **A song carries on when its headphones disconnect.** The song moves to the device the system
  now uses and keeps its position, and it moves back when the headphones return. The machine went
  silent until it was restarted.
- **KM Simple Package stays where you are when you keep a song or leave it out.** The page jumped
  to its foot on every click of a box.
- **A saved filter opens on songs when the list has shrunk.** It came back as an empty page.

## [1.19.0] - 2026-10-02

### Added

- **The admin Sound tab sets when the words light up.** A television that shows its picture late
  draws the words after the music. Move them up to half a second earlier or later on the machine's
  own admin page or in `km-admin`, and the machine remembers it.
- **An LRC file beside an MP3 of the same name is a song.** A file that times each word gets the
  word-by-word highlight. A file that times only its lines lights each whole line as it starts. A
  bar above the next line fills during a break, so you know when to come back in. The next line
  brightens just before it starts, so you see it coming. A package that holds an LRC song needs
  this release, and an older machine refuses the whole package.
- **A MIDI file that times its words a line at a time is drawn the same way.** Its lines light
  whole rather than being wiped across at a speed the file never gave.
- **A folder becomes a package without the curation tool.** KM Simple Package, `km-package-simple`,
  reads a folder and lists its songs, with each one's language and suitability. You can rename a
  song or leave it out, and a shift-click keeps or leaves out a whole run. A folder of more than 999
  songs becomes several packages. Every package it writes is marked *uncurated*, and so is one
  `km-pack spec` describes from a folder.
- **KM Simple Package installs like the other tools.** The setup programs for Windows and macOS
  offer it, and the `karaokemachine-tools` package carries it on Linux. Its icon is the package
  builder's with a lightning bolt.
- **An uncurated package says so.** The machine's list of packages on the admin page, the remote's
  package list and the API mark it, and `km-pack inspect` and `check` print it. The television does
  not show it. The package builder keeps the mark on a package it imports, and every rebuild keeps
  it too.
- **Who may do what, in four levels.** A phone can watch, queue songs, or also skip and play a song
  now. The owner does everything. Out of the box anybody in the room can queue. The
  *This machine* tab sets what a phone with no code gets, and holds a code to queue and a code to
  skip. A guest types a code on the remote's Setup tab, and the remote hides what that phone cannot
  use.

- **The machine runs on a Meta Quest, on a screen that hangs in the room.** It is a separate APK from
  the Android one, and it installs beside it rather than over it, so a headset can hold both. The
  room shows behind the screen, the controllers reach the on-screen keypad, and every song kind plays
  with the words highlighting in time. Only the wearer sees the words, so this suits one person
  practising rather than a room of singers. There is no microphone path either, because a headset
  has no mixer.

- **On a Quest, the screen goes where you put it.** It starts on the main wall of the room the
  headset scanned. Grab it with a hand or a controller to move it, and pull a corner to make it
  larger. It comes back to the same place on the next launch. The headset asks once to read the
  room. A refusal, or a room never scanned, opens the screen in front of you, still moving by hand.
- **On a Quest, a small bar of buttons rides on the screen's right.** It moves with the screen. Its
  buttons put the screen back on the wall, show the queue and open the window. The bar steps aside
  while a song plays.
- **On a Quest, one app shows the machine in the room or in a window.** The Window button beside the
  screen moves it into an ordinary system window, and a button in the window moves it back. The library
  tile opens whichever you used last. A switch restarts the machine, so it waits until no song is
  loaded and the queue is empty. The window's VR Mode button shows only then. A song package opened
  from Files goes to whichever screen is running.
- **On a Quest, the queue hangs beside the screen.** It is the same remote a phone gets, so a
  wearer can search and add songs without holding a phone. A button under the screen hides it.
  It faces you wherever you put it, and a corner resizes it.

- **The package builder has a Folders page.** It lists the corpus by folder, with how many distinct
  songs are in each one and beneath it. Each folder opens the songs list narrowed to that folder,
  and a song page links each copy to the songs beside it. A scan that finds changes builds the
  folder list as its last step, which adds several minutes to it on a large corpus.
- **A song's own words place it when nothing else names its language.** Where neither the file's
  header nor its lyric encoding says anything, the builder reads the words themselves. What comes
  back counts as the song's language, so you can filter to it, sort by it and package it without
  classifying anything by hand. A reading only counts when it is certain, so a song nothing can place
  stays blank rather than guessed at. A song page says which language came back, and how sure the
  reading was. The first open of an existing corpus reads its songs once, showing its progress.
- **The browse bar can leave languages out**, any number of them at once. It is the short way to take
  a folder in a language you do not read out of every list. Songs nothing has classified stay, and
  the filter writes nothing to the corpus.
- **A song can be played without its words.** Some files are well made and their lyric track is
  mistimed, is the arranger's own name and telephone number, or belongs to another song. The
  Advanced tab of a song's page now says whether the machine draws its words, and the television
  says *no lyrics* in the corner. A file whose words are all timed to the
  first instant, or are credits, or are a chord chart, turns them off by itself. Any other file is a
  judgement to make, and the choice survives every rebuild. Video and MP3+G songs carry their words
  in their own picture, so the choice does not reach them.
- **A song in the package builder can be thrown away.** The Delete tab takes the ticked rows, or
  everything a filter matches. It first says how many there are, and how many of them a package
  holds.
  A song thrown away is in no list but the bar's *only deleted* box, and a scan does not read its
  files again. A corpus full of bad rips therefore stops costing time on every scan. Nothing leaves
  the disk, the same tab brings a song back, and a backup carries what you threw away.
- **The song list can show what the analysis found wrong with each file.** Tick *show file warnings*
  and every row carries its warnings as chips beside its title. A file with no lyrics, with words
  timed a line at a time, or with words that stop halfway is then visible down a page. The song page
  is not the only place to find it.
- **The suitability filter takes any range you type into the address.** `suitability=2-5` narrows to
  the files scoring 2 to 5, `suitability=9` to the 9s alone, and `suitability=7-` to 7 and up. The
  dropdown still offers *any*, `8-10`, `5-7` and `<5`, which are the three questions worth a control.
  A range you have asked for appears beside them, selected, for as long as it holds. A range nothing
  can mean, such as `7-3`, shows the whole corpus rather than an empty page.

### Changed

- **A phone with no code queues songs and does not interrupt them.** Skip, play now, pause, stop,
  restart, moving or removing a queued song, the demo song and the next picture need the skip level.
  To give every phone those buttons, set the room level to *Queue, skip and play now* on the
  *This machine* tab. The owner signs in to `/admin` once more after the update.
- **A grayed-out key, tempo or melody button on the phone says why.** A video song says it has no key
  to change, and a MIDI file with no melody line says the search found none. With nothing playing,
  the buttons say nothing.
- **Changing a microphone needs the admin password.** Its name, gain, effects and mute move to
  `PUT /api/v1/admin/mics/{id}`. Reading them at `GET /api/v1/mics` stays open to anybody.
- **A song with barely any singing in it stops sorting above the real ones.** A file sung for less
  than three quarters of a minute rates 4 out of 10 or lower and counts as defective. The package
  builder's default 8–10 band therefore leaves it out, and the under-5 band is where to find it. How
  long the words run is what counts, not how long the file is, so the same rule catches a
  four-minute file holding one verse. The rule reaches karaoke videos, MP3+G pairs and UltraStar
  songs too: a thirty-second video is a clip rather than a karaoke track, whoever made it. The first
  scan after upgrading reads the whole corpus again to work the new number out, and
  `Recalculate suitability` does the same for a package.
- **A streamed television runs three or four seconds behind the machine.** Pause, skip and a newly
  queued song reach it that much sooner. A `settings.json` that already sets
  `stream.segment_seconds` keeps its number, so set it to `1` to get this. An older television that
  stops to buffer plays smoothly at `2`.
- **The `/watch/` page runs under half a second behind the machine.** It takes the stream over a
  WebSocket, so pause, skip and a newly queued song reach it almost at once. A player on the
  playlist stays three or four seconds behind. A television whose page stutters plays the playlist
  at `/watch/?hls`.
- **A browser that cannot decode the stream says so.** The `/watch/` page names the missing H.264
  or AAC support and gives the address to open in VLC.

## [1.18.0] - 2026-09-18

### Added

- **The package builder can number a package's only volume.** Tick the box under the volume name, and
  a package that will pass 999 songs takes the name `vol1` from its first build. Its file then keeps
  that name when a second volume starts.

### Changed

- **The machine is "Karaoke Machine" under its icon, and its streaming launcher is "KM Stream".**
  Launchers on Android, iOS, Linux, Windows and macOS cut the single word "KaraokeMachine"
  mid-word. On Windows the Start menu folder and shortcuts take the new names, and on macOS the
  applications are `Karaoke Machine.app` and `KM Stream.app`. Upgrading removes the old shortcuts
  and applications.

## [1.17.0] - 2026-09-17

### Added

- **A karaoke machine that behaves like a commercial home unit.** Pick a song by number, it plays,
  the words highlight in time. It runs full-screen on a television, and phones on the same network
  are the remotes. Native on Windows, macOS and Linux, on Android, and on an iPhone and an iPad. On
  Debian it draws straight to the screen from a bare TTY, with no desktop installed.

- **Four kinds of song file, and nothing else.** MIDI and KAR files in all three karaoke conventions
  that exist in the wild, and video songs in an MP4. MP3+G pairs, an MP3 with a `.cdg` of the same
  stem. UltraStar songs, the `.txt` a singing game times its words in beside the MP3 it names. An MP3
  on its own is not a song, because it has no words in it.

- **Songs arrive in packages.** A `.kmpkg` carries its own queue numbers, titles, artists and
  analysis. Drop one in the packages folder or onto the machine's window and it goes in without a
  restart.

- **Every song file is rated for suitability out of ten**, with a breakdown. It carries the language
  it is in, and the first line or two of its words. You can therefore ask a catalog what Portuguese
  it holds, and a list shows something a person recognizes a song by. Packaging finds the melody
  channel where it can find one confidently, and abstains with a stated reason where it cannot.

- **Words highlight syllable by syllable on the sequencer's own clock.** Transpose and tempo are per
  song, and the guide melody can be muted. A lyric timing offset in milliseconds is adjustable
  mid-song, because a television adds picture lag and a mixer takes the sound out early. The offset
  moves the highlight only, never the audio, because the microphones are in that audio.

- **A queue with singer names**, shown over whatever is playing, and wallpapers cycling behind it
  with a crossfade. A zip of images counts as a folder of them.

- **A remote at the machine's own address**, with a QR code on the idle screen: search, queue, now
  playing, and whatever controls the song allows. Any phone on the network, no app to install.

- **A standalone offline remote** that keeps its own copy of a machine's catalog, so browsing,
  searching and favorites work with the machine switched off.

- **A television in another room.** Started with `--stream`, the machine opens no window. It serves
  what it would have shown — the words, the wallpaper, the queue and the music — at one address. A
  smart television, a phone or a computer opens `/watch/`, and any playlist player takes
  `/stream/live.m3u8` with no browser.

- **One admin password, which the machine gives itself and shows on screen.** A six-digit PIN at
  first start, beside the address on the idle screen. Everything that reconfigures the machine wants
  it; everything a singer does wants nothing.

- **KaraokeMachine Admin finds pictures and instruments and sends them over.** Photographs the lyrics
  stay readable over, measured in the exact band of the screen the words occupy. General MIDI banks
  from a table of sixty-three. It also sends a file you already have, which on a television box is
  the only way to hand the machine one. The machine downloads nothing itself: it may have no internet
  connection, and it should not hold your accounts.

- **A printed song book**, and a curation tool that turns a folder of files into a package. It scans,
  rates, names, numbers and corrects a corpus a few hundred thousand files deep.

- **An HTTP API for the whole of it**, and discovery on the network. A development console needs no
  password while the machine is in debug mode.

- **A demo mode, off unless asked for.** After a minute of quiet the machine starts a song of its
  own, and another when that ends. Queueing takes the deck off it at once, so nothing has to be
  skipped first.
