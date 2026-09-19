# Changelog

What a release of the machine, its remotes and its tools gives the person using them. The format
is [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the numbering is
[semantic versioning](https://semver.org/spec/v2.0.0.html) — one number covers every program here,
so a release moves all of them together.

Entries are written for somebody who has the machine. Why any of it is the way it is lives in
[`docs/decisions/`](docs/decisions/), and how it is built in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## [Unreleased]

### Added

- **The package builder has a Folders page.** It lists the corpus by folder, with how many distinct
  songs are in each one and beneath it. Each folder opens the songs list narrowed to that folder,
  and a song page links each copy to the songs beside it. A scan that finds changes builds the
  folder list as its last step, which adds several minutes to it on a large corpus.
- **A song with no language is placed by its own words.** Where neither the file's header nor its
  lyric encoding says anything, the words themselves are read, and what comes back counts as the
  song's language — so it can be filtered to, sorted by and packaged without anybody classifying it
  by hand. A reading is kept only when it is certain, so a song nothing can place stays blank rather
  than being guessed at, and a song page says which language was read and how sure the reading was.
  The first open of an existing corpus reads its songs once, showing its progress.
- **The browse bar can leave languages out**, any number of them at once, which is the short way to
  put a folder in a language you do not read out of every list at once. Songs nothing has classified
  stay, and the filter writes nothing to the corpus.
- **A song can be played without its words.** Some files are well made and their lyric track is
  mistimed, is the arranger's own name and telephone number, or belongs to another song. The
  Advanced tab of a song's page now says whether the machine draws its words, and the television
  says *no lyrics* in the corner while such a song plays. A file whose words are all timed to the
  first instant, or are credits, or are a chord chart, turns them off by itself; any other file is a
  judgement to make, and the choice survives every rebuild. Video and MP3+G songs carry their words
  in their own picture and are not offered the choice.
- **A song in the package builder can be thrown away.** The Delete tab takes the ticked rows or
  everything a filter matches, after saying how many there are and how many of them a package holds.
  A song thrown away is in no list but the bar's *only deleted* box, and a scan does not read its
  files again — so a corpus full of bad rips stops costing time on every scan. Nothing is removed
  from the disk, the same tab brings a song back, and a backup carries what you threw away.
- **The song list can show what the analysis found wrong with each file.** Tick *show file warnings*
  and every row carries its warnings as chips beside its title, so the files with no lyrics, with
  words timed a line at a time, or with words that stop halfway are visible down a page instead of
  one song page at a time.

### Changed

- **A song with barely any singing in it stops sorting above the real ones.** A file sung for less
  than three quarters of a minute rates 4 out of 10 or lower and is marked defective, so the package
  builder's default 8–10 band leaves it out and the under-5 band is where to find it. What counts is how long
  the words run rather than how long the file is, so a four-minute file holding one verse is caught
  by the same rule. Karaoke videos, MP3+G pairs and UltraStar songs are judged on it too: a
  thirty-second video is a clip rather than a karaoke track, whoever made it. The first scan after
  upgrading reads the whole corpus again to work the new number out, and `Recalculate suitability`
  does the same for a package.

## [1.18.0] - 2026-09-18

### Added

- **The package builder can number a package's only volume.** Tick the box under the volume name, and
  a package that will pass 999 songs is named `vol1` from its first build, so its file keeps that
  name when a second volume starts.

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
