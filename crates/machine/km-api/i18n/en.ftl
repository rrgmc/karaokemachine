# The printed song book's chrome — everything on a page that is not a song.
#
# This is the source catalog: every message is written here first, in US English, and every other
# locale is a translation of this file. See `The prose and the names are US English`.
#
# **Every character here must be in cp1252.** The book embeds no font — it uses the PDF base-14
# faces with WinAnsiEncoding — so anything outside that repertoire is transliterated or replaced
# with `?` and counted. `a_catalog_survives_the_books_encoding` is the test that says so.
#
# **The count reaches whoever made the book and nobody else**, which is why this test exists rather
# than a warning on the page: `km-pack book` and `karaokemachine --song-book` print it, and
# `GET /songs/book.pdf` has no terminal to print it to. A reader holding the PDF could not act on
# the number anyway — the person who can is the one who built the package.

## Every page

# Top left of every page: whose book this is. A proper noun, and deliberately not translated — see
# `What the product is called`. It is here rather than hardcoded so that `?name=` has one thing to
# override and the layout has one thing to measure.
book-name = KaraokeMachine
# Centered at the top of every page.
book-title = SONG LIST
# What the downloaded file calls the document.
#
# **`book-title` in the case and spacing a filename wants**, and a separate message rather than a
# transformation of it: `LISTA DE MÚSICAS.pdf` is a shout, and title-casing a shout back down is a
# rule that differs by language. Separate from `book-name` too — that one names the machine the book
# belongs to, this one names the document. `book_filename` puts the two together.
book-filename = Song Book

## The four columns

column-artist = ARTIST
column-code = CODE
column-title = TITLE
column-first-line = FIRST LINE

## What a book says when it has no rows

# Nothing is installed at all, so there is nothing any book could show.
book-empty-catalog = No songs are installed.
# Songs exist; this book's filter matched none of them.
book-empty-filter = No songs match.

## Sections

# The heading for songs with no language recorded.
#
# **Not the same thing as `und`.** The ISO table has `und` — "undetermined" — and a package built
# with `--default-language und` says so on purpose: somebody looked and could not tell. This is for
# a song whose language nobody ever wrote down at all.
book-unclassified = No language recorded

## The note under the title

# The song count. A plural, which is the whole reason the catalog is Fluent: the hand-written
# `if rows == 1 { "" } else { "s" }` this replaces has no answer in a language whose rule is not
# English's.
book-song-count = { $count ->
    [one] { $count } song
   *[other] { $count } songs
 }
# Appended when the book is of one package.
book-of-package = package { $package }
# The catalog version, which is what changes when a package is installed or removed — so it is the
# one number that tells two printed copies apart.
book-catalog-version = catalog { $version }

## Language names, for section headings

# **Deliberately partial**, and the parity test exempts this prefix by name. A machine shows the
# five to ten languages its packages actually carry, so translating all 186 rows of the ISO table
# would produce strings no build will ever render and no reviewer here can check. A code with no
# entry falls back to `km_kmpkg::Language::name()`, which is the behavior a code the table does not
# know already has: shown as itself rather than dropped.
language-en = English
language-pt = Portuguese
language-es = Spanish
language-it = Italian
language-fr = French
language-de = German
language-ja = Japanese
language-und = Undetermined
language-zxx = No words
