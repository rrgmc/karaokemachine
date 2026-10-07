# Getting a corpus into shape

<p align="center">
<img src="../images/package-builder-songs.png" width="90%"
     alt="km-package-builder in a browser: a wide filter panel whose narrowed list can be saved
under a name, over a table of songs with columns for artist, title, language, length, suitability,
your own rating, melody and duplicate count, and per-row buttons to play a song, file it in a
favorite, edit its title and artist, and look it up on YouTube">
<br><sub><b>km-package-builder</b>, for getting a folder of files into shape before it is packaged.</sub>
</p>

**KM Simple Package, `km-package-simple`, makes a package from a folder in one step.** It is for
songs you want on the machine without curating them first. Choose the folder, rename a song or leave
it out, and build. Each package it writes is marked *uncurated*, and the machine's package lists
show the mark.

**Five more tools turn files into packages.** `km-package-builder` curates a folder,
`km-pack` builds and checks packages, and `km-lyrics` shows one file's parsed timeline.
`km-site-pack` downloads the song files a site links and builds a package from them.
`km-wallpaper-pack` builds a wallpaper set from pictures the lyrics stay readable over.

**KM Song Sync, `km-song-sync`, puts words on a MIDI file that has none.** Paste the words, choose
the file, and tap each word as it is sung.
[Putting words on a MIDI file](lyric-sync.md) has the rest.

```sh
# Curation: a local web server at http://127.0.0.1:8178. Browse, search the lyrics themselves,
# rate, fix names, group duplicates, and pick songs into packages. Only `--init` creates the
# database, so a wrong folder is an error rather than an empty index.
km-package-builder ./songs --init --scan --open
km-package-builder ./songs                      # once it has a database

# Packaging: describe the folder, edit the description, build it. `build` takes a description and
# never a folder. A folder of more than 999 songs is refused: split it, or narrow it with the flags.
km-pack spec ./songs --out vol1.kmspec.yaml     # what is here, and what it is called
km-pack spec ./songs --out vol1.kmspec.yaml --min-suitability 6 --require-lyrics   # ...or be choosier
km-pack build vol1.kmspec.yaml                  # build what the description says
km-pack check vol1.kmpkg                        # validate + report suitability

# One file's parsed lyric timeline and analysis, when a song does not behave.
km-lyrics dump ./song.kar

# A site's song files into a folder, and a package from the ones that have words. It honours the
# site's robots.txt and waits between requests. Whether you may keep the files is yours to judge.
km-site-pack <address> ./songs --dry-run        # list what a run would download
km-site-pack <address> ./songs                  # download, then build the package beside ./songs
```
