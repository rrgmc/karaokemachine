# The song book

**The song book is a printable PDF of every installed song.** It has four columns: artist, number,
title and first line. It sorts by artist, with a section per language. It reads the catalog, so start
the machine once after you add a package. `--book-name` sets the heading, and the default is
`KaraokeMachine`.

```sh
karaokemachine --song-book ./songbook.pdf
karaokemachine --song-book ./songbook.pdf --book-name "Sitting room"
```
