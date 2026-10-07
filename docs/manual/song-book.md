# The song book

A singer picks a song by its number, and a printed book is the quickest place to find one. The
machine writes that book as a PDF of every installed song.

```sh
karaokemachine --song-book ./songbook.pdf
karaokemachine --song-book ./songbook.pdf --book-name "Sitting room"
```

**The book has four columns: artist, number, title and first line.** It sorts by artist, with a
section per language. `--book-name` sets the heading, and the default is `KaraokeMachine`.

**Start the machine once after you add a package, then write the book.** The book reads the catalog,
and a new package enters the catalog when the machine starts.

A printed number stays right while its package keeps its block of numbers. See
[How songs are numbered](songs.md#how-songs-are-numbered).
