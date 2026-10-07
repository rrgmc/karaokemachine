# What it does

**Songs**

- **Songs arrive in packages** (`.kmpkg`). Each one carries its own queue numbers, titles, artists
  and analysis, and it goes in without a restart.
- **A language per song**, so a catalog can say what Portuguese it has.
- **A 0–10 suitability rating** for every file, with a breakdown.
- **The melody channel, where the machine can find it with confidence**, and a stated reason where
  it cannot.
- **The first line or two of each song's words**, so a person can recognize the song. The machine
  skips the studio-name banner that many karaoke files open with.

**Playing**

- Full-screen on a television, drawn by SDL3. On Linux it draws straight to DRM/KMS from a bare TTY,
  with no desktop installed.
- **Or on a television in another room**, as a stream a browser or a playlist player opens. See
  [Watching it in another room](streaming.md).
- **Words highlight in time**, syllable by syllable, on the sequencer's own clock.
- **Transpose and tempo** per song, a **guide melody** that can be muted, and per-song defaults.
- **A lyric timing offset in milliseconds**, adjustable mid-song, for the picture lag of a
  television. It moves the highlight only and never the audio, because the microphones are in that
  audio.
- **Wallpapers** cycling with a crossfade, from a folder of stills or a zip of them.
- **A queue** with singer names, shown over whatever is playing.
- **A demo mode**, off by default: after a minute of quiet the machine plays songs by itself until
  somebody queues one.

**Remotes**

- **The machine serves a remote at its own address.** Any phone on the network reaches it, with no
  app to install.
- **A standalone offline remote** keeps its own copy of a machine's catalog, and works with the
  machine switched off.
- **One admin password, which the machine shows on its own screen.** Everything that reconfigures
  the machine needs it, and nothing a singer does needs it.
- **Three levels below the owner: watch, queue, and queue plus skip.** Anybody in the room can queue
  a song out of the box. You can lower that to watching only, or raise it to skipping as well. Two
  codes you choose raise one phone at a time: one to queue, one to skip and play now.

**Giving the machine pictures and instruments**

- **KaraokeMachine Admin** (`km-admin`) finds photographs the lyrics stay readable over. It also
  offers General MIDI banks from a table of sixty-three. It sends either to the machine, and it sends
  a package, a bank or a photograph of your own.
- **The machine downloads none of these itself**, because it may have no internet and should not hold
  your accounts. `km-admin` keeps a copy of every download, to send to the machine later.
- **Pictures come from Openverse by default.** Openverse needs no account, and its packs may travel
  on. Pixabay and Pexels need an API key of your own, and their terms forbid that.

**Not supported, by decision**

- No scoring of singers.
- No bare audio files, and no CD+G disc images — only the file pair.
- No video wallpapers: a video song is not a video background.
- No pitch shifting of audio.
- No microphone processing in the app, because hardware mixes the microphones.
- No Thai, Arabic or Indic words on the screen, and no right-to-left, though the machine draws
  Japanese and Chinese.

Each of these is a decision with a reason. To propose a change, start with
[`docs/decisions/`](https://github.com/rrgmc/karaokemachine/tree/master/docs/decisions).
