# KaraokeMachine

A karaoke machine that behaves like a commercial home unit: pick a song by number, it plays, and the
words highlight in time. It runs full-screen on a television, and any phone on the network is a
remote: search the catalog, queue a song, change the key, skip.

![The playing screen on a television: the song's number, title and artist across the top with key and
melody badges and a disc in the corner counting the songs waiting, the line being sung in large
letters with the current syllable half-filled in amber, the line that follows it below in gray, and a
progress bar along the bottom](../images/screen-playing.png)

A song is one of five things:

- a **MIDI file with embedded karaoke lyrics**;
- a **video file**;
- an **MP3+G pair**: an MP3 with a `.cdg` of the same stem beside it;
- an **UltraStar song**: the `.txt` a singing game times its words in, and the MP3 it names;
- an **LRC song**: an `.lrc` of timed lyrics, and the MP3 of the same name.

An MP3 on its own is not a song, because it has no words in it.

It runs on Windows, macOS and Linux, on Android and a Meta Quest, and on an iPhone and an iPad.

Start it, type a song number, and it plays. The screen shows the words, and a phone does everything
else.

## Getting started

- [What it looks like](pictures.md)
- [What it does](what-it-does.md)
- [Installing](installing.md)
- [On Debian, it is also an appliance](appliance.md)
- [On an iPhone or an iPad, you sign it yourself](ios.md)
- [Removing it](removing.md)

## Using it

These chapters need the machine's screen, a phone, a browser or a double-click.

- [The keyboard](keyboard.md)
- [Getting songs in](songs.md)
- [The remotes](remotes.md)
- [Watching it in another room](streaming.md)
- [Setting it up from a browser](setup.md)
- [The song book](song-book.md)

## For a technical reader

These chapters need a shell or a program of your own. They cover the flags, the song file formats,
the API, discovery on the network, and the tools that make a package.
[`BUILDING.md`](https://github.com/rrgmc/karaokemachine/blob/master/BUILDING.md) covers compiling.

- [The command line](command-line.md)
- [Song files and their formats](formats.md)
- [The HTTP API and the network](api.md)
- [Getting a corpus into shape](packaging.md)
- [Putting words on a MIDI file](lyric-sync.md)

## About

- [Privacy](privacy.md)
