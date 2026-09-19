# The curation tool

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

`tools/cmd/km-package-builder` is a local web server over a folder of source karaoke files. It
browses, rates, corrects and groups them into packages that `km-pack` builds. Loopback by default;
`--lan` opens it up and warns, because there is no password on the tool itself -- though it holds the
machine's, from a box on the Settings page, once somebody has typed it. The token it buys lives in
memory for the run; the password reaches a disk only if the checkbox beside the box is ticked, and
then into `machine-passwords.json` in this tool's per-user config folder rather than into the
curated folder. See `The password is remembered per machine, or not at all` in
[`curation.md`](../decisions/curation.md).

`--init` is the only thing on the command line that creates a database, so pointing it at the wrong
folder is an error rather than an empty index. With no folder it starts on an Open page — recent
folders, a browser, a path box — which is the other thing that may create one.

## The database is the document

`km-package-builder.kmbuild` is ordinary SQLite living in the curated folder, under a name the
operating system can hand to one program. Opening looks for whatever single `*.kmbuild` a folder
holds rather than a fixed name, so a corpus can be called `Brasil.kmbuild`.

Two tables carry the model:

- **`files`** — one row per file on disk, path relative to the root with forward slashes so the
  folder can move. `(size, mtime)` is what makes a re-scan nearly free.
- **`songs`** — one row per distinct recording, **keyed by the content hash of its bytes**. That one
  choice gives both halves of the brief: the same file always yields the same id, and identical
  copies land on one row with no grouping pass to get wrong. `files` carries the one-to-many.

Then `favorites`/`song_favorites` (named lists and what is in them), `folders` (the materialised
folder tree), `packages` and `package_volumes` (what a curator names, and each `.kmpkg` it is
written as), `package_songs` (recording *which file* each entry came from), `saved_filters` (a name and the query
string it stands for), `settings`, and two FTS5 tables.

`Db` is one type with one `rusqlite::Connection` and no pool, and its ~100 methods are **twelve files
under `db/`, not twelve types**: `open`, `songs`, `favorites`, `packages`, `saved`, `backup`, `scan`
are `impl Db` blocks split by the question being asked, and `migrate`, `sql`, `filter`, `model`,
`tests` are
what they all speak in. The seams are the `// -- packages ---` banners the single file already
carried, so the split was a move with no logic in it — and nothing outside `db` learned it happened,
because every type is re-exported flat from the module root.

### Three kinds of name, and why they cannot merge

`songs` keeps detection and correction in **separate columns**: `det_title` is never written by a
person, `title` is never written by the scanner. That is what lets a re-scan refresh the analysis
without touching a correction, and it is the same distinction `SongEntry::edited` makes inside a
package. NULL in a hand-set column means *nobody has said*, which is not empty.

**`artist` is where that distinction is load-bearing rather than tidy.** `eff_title` folds NULL and
`''` together with `nullif`; `eff_artist` has no `nullif`, so an empty `artist` stands in front of
`det_artist` where a NULL one falls back to it. That is what lets `set_names_from_stem` clear the
performer a sequencer left behind, and it is why the two sort into different ends of the browse list.

`songs.stem` is the third: the file's own name, written by the scanner through the same
`km_pack::file_stem` that `km-pack build` uses, so the two cannot disagree about what a nameless song
is called. It exists because a great many corpus files carry no metadata at all, and
`coalesce(title, det_title, '')` renders those as an invisible unclickable link sorted to the front.
It is deliberately **not** `det_title` — that column answers *what did the file say?*, and a filename
written there would be a lie a re-scan would preserve.

The fallback had to be a column rather than a template default, a row-mapper default or a SQL
expression over the joined path: only a column fixes display, `ORDER BY` and search at once, and only
a column is visible to the FTS triggers, which fire on `songs` and cannot see `files`. `eff_title()`
in `db.rs` is the one expression every query builds from, and `SongDetail::effective_title` is its
Rust twin.

`det_language_tag` is a second *detected* column holding the ISO 639-1 code that `det_language` and
`det_encoding` imply, while `det_language` goes on holding the file's literal `ENGL`.

### The backup is that split, read out to a file

`backup.rs` writes the hand-set half of `songs` plus the favorites and their membership, and
nothing else — the derived half is what a re-scan produces and would be a hundred megabytes of nulls
on a real corpus. `Db::hand_set_songs` is the whole of the read; its `WHERE` is built from
`HAND_SET_COLUMNS` so the predicate and the document cannot name different columns, and it carries an
`id IN (SELECT song_id FROM song_favorites)` disjunct so a song somebody has only filed — which
carries nothing in its own row — is still in the file.

Two identities have to be translated, because a rowid means nothing in another database. Songs use
`songs.id`, the content hash, which needs no translation and is why the whole thing works across a
rebuild. A favorite becomes its name, which `favorites_name` makes unique and which needs no escaping
rule of its own — `Rock/Pop` is a name somebody types and travels as one. A document written when
favorites nested names one by the array of its ancestors, and `FavoriteRef` joins such an array with
` / `, the separator those pages drew: the list a person was looking at is the list they get back.

Restoring is two phases. `backup::plan` reads and checks and writes nothing, so every refusal —
a language `Language::parse` will not take, a rating outside 0–10, a transposition that will not fit
`i8`, a merge that would chain — costs a line in a report and no recovery. `Db::apply_restore` then
writes the checked plan in one transaction, preparing its own statements rather than calling the
per-field setters: a `rusqlite::Transaction` holds `&mut self.conn`, so `self.edit_song(..)` does not
borrow and `self.add_to_package(..)` would issue a nested `BEGIN` SQLite refuses. Same arrangement as
`write_scanned` and `forget_missing`, for the same reason. The fill-blanks/overwrite policy is a
bound flag inside one `CASE`-per-column `UPDATE`, so both directions are one statement — and neither
can write NULL, since `coalesce` with the column itself is the whole rule.

`default_path` names the file after the tool and after the moment, and `backup::newest` reads the
data folder for the most recent one so the restore box can suggest it. Both lean on the stamp being
sortable, and on the stem being the only part that moves: `SUFFIX` is what the scan skips by and what
`db::database_in` must not mistake for a second database. The stamp itself is `scan::timestamp` with
its separators removed rather than a second clock, which is also what keeps `written_at` and the file
name talking about the same instant.

The one thing here that is written down rather than derived is the field list, and a hand-written
column list is how corrections get lost. It cannot be derived — a JSON key is not a column — so a
test partitions `pragma_table_info('songs')` into `HAND_SET_COLUMNS` and
`DERIVED_COLUMNS` and fails, naming the column, on any in neither.

### Search is two indexes, not one

`songs_fts` covers the effective title and artist, with `remove_diacritics 2` copied from
`km-catalog` because the corpus is largely Portuguese and `coracao` must find `coração`. It cannot
be an external-content table — the content would have to be the expression `coalesce(title,
det_title)`, which FTS5 does not allow — so it is a plain table kept in step by triggers.

**That difference matters:** the `INSERT INTO songs_fts(songs_fts, …) VALUES('delete', …)` form
`km-catalog` uses is valid only on external-content and contentless tables; against a plain one it
fails at run time with nothing but `SQL logic error`.

Both `_update` triggers are `AFTER UPDATE OF <the columns the statement reads>` rather than bare
`AFTER UPDATE`, which is a correction and not a refinement: setting one language over a filter writes
every matching row, and a bare `AFTER UPDATE` would delete and reinsert a title, an artist and a lyric row
for each of those to arrive at the text already in them. The rating setter pays that on every click.

`lyrics_fts` is a **second table, not a third column**, and this is load-bearing. An FTS5 `MATCH`
with no column filter searches every column, so putting lyrics beside title and artist would silently
turn the browse page's "title or artist" box into a lyric search — type `love` and half the corpus
comes back. Two questions on two pages are two indexes.

Its insert is conditional (`WHERE new.lyrics IS NOT NULL AND new.lyrics <> ''`) and its delete
unconditional: most of a real corpus is instrumental, and the asymmetric pair is also what makes a
song whose lyrics are cleared leave the index rather than linger.

**Triggers are dropped and recreated on every open**, never `IF NOT EXISTS`: a trigger is code, and a
database made by an older version must not keep a broken one.

`db::filter::fts_match_query` is what both boxes type through, and it emits three shapes and no
others: a quoted token, a quoted run of tokens (an FTS5 phrase), and either with a trailing `*`. A
`"` in the input is read as the mark that opens or closes a phrase and is never passed through, so
every `"` in the expression is one the function wrote — which is what keeps `OR`, `*` and an
apostrophe literal. `a_phrase_reaches_sqlite_as_a_phrase` runs all three through the real index,
because a string that looks like a phrase query is not evidence that SQLite reads it as one.

## A swappable folder

`State` holds `Arc<RwLock<Option<Arc<Workspace>>>>`, where `Workspace` is the database, the root and
the scan handles. **The inner `Arc` is the load-bearing part**: every accessor clones it out and drops
the read guard immediately, so no request holds the lock across its SQLite work. Held, a swap would
queue behind a multi-minute migration and the polling pages would starve the writer —
`std::sync::RwLock` promises no writer preference.

The scan and build handles live in `Workspace`, and `Db::close` is in its `Drop`. A scan is a detached
thread holding a clone of the connection for minutes; left on shared state it would go on writing into
the *previous* corpus after a swap, with no symptom until rows appeared in a folder nobody had open.

**The songs filter is emptied by the same call.** It is one string on `State` and reaches no file, so
a filter lives exactly as long as the folder it names: `publish` clears it before it takes the write
guard, and `close_folder` clears it on the way out — where `crate::finish` and the event loop's
`LoopDestroyed` both arrive. Coming back to a morning's work is what a *saved* filter is for.

**What the remembered filter is worth depends on one route reading it, and `GET /songs` is that
route.** Every render of the songs page writes down the filter that arrived, which is how the Folders
and Favorites pages hand it one the bar never set — so a render arriving with an empty query string
is a render that spends what was remembered, and the nav's seven bare hrefs are where that happens.
The handler therefore takes `RawQuery` beside its `Query<FilterQuery>` and answers
`None` with a redirect to the remembered filter, leaving `Some("")` — the *clear all* link — to
render the corpus and clear the record. `rows_for` clamps a page above the end back to the last one
before the record is written, so what is written down is the page being shown.

**The folder browser's cost is one directory read per row, so the page comes before the probe.**
`browse::indexed` opens the folder it is asked about — a badge saying *indexed* means a `read_dir` of
that subdirectory — so `browse::list` gathers names, narrows, sorts and cuts to a page *before* it
asks the question of anything. The order is the load-bearing part and reads as arbitrary: probing
inside the gathering loop is the obvious shape, and it is what made listing a folder of three
thousand subdirectories a minute of disk. `GET /open/list` is also the one route here that cannot
use `State::blocking`, since that wants a workspace and the picker is the page reached without one;
it spawns its own blocking task, and running on the executor thread is the fault that hides behind a
listing which is fast on every folder somebody tested.

`?rows=1` answers with the folders and their pager alone. The filter box has to sit outside what it
replaces or it is destroyed by its own result four hundred milliseconds after a keystroke, which is
the arrangement `#filters` and `#rows` already have on the browse page.

Handlers know nothing about the `Option`: a middleware redirects anything needing a folder to `/open`.
Two details there were only right because they were checked — **`HX-Redirect`, not a 302** (a browser
follows a 302 transparently, so htmx would paint the whole Open page inside whatever `<div>` the button
aimed at), and **`.layer`, not `.route_layer`**, so a path matching nothing is caught too.

**Opening is a job, not a request.** `Db::prepare` runs migrations, creates browse indexes, runs
backfills and finishes with an unbounded `ANALYZE` — minutes on a real corpus. None of it may happen
before the socket is listening, because a browser pointed at a tool that has not bound is refused and
reads as broken. So `begin_open` spawns a thread and `/open/progress` is polled.

**`Phase` reaches every stretch of that, `migrate` included.** It is `&dyn Fn(&str)`, threaded through
`Db::prepare` as an argument rather than a field because the ladder runs before a `Db` exists, and
`begin_open` is the only caller with anywhere to put the answer — everything else passes `&|_| {}`.
The sentences come out of `indexing_phase`, `folding_phase` and `rebuilding_phase` in `db.rs`, one
function each, so the console banner and the page cannot come to describe the same work differently.
Two of them are said under `announce`'s size threshold on the console and unconditionally to the page:
a terminal has something else to show, and the page has only the sentence it is already holding.

`Opening` carries a `started: Instant` and `OpeningView` an `elapsed_secs`, which is what moves through
the one step long enough to matter. The panel's bar is `.bar.working` — full width, since an open has
no denominator to draw, and given its resting opacity by the rule rather than by the keyframe's 0%, so
a tab the browser has stopped painting shows a bar rather than a faded-out one.

**The schema has a version, and its first job is to refuse.** `PRAGMA user_version` is stamped with
`SCHEMA_VERSION`, and a database outside `OLDEST_SCHEMA_VERSION` to `SCHEMA_VERSION` is turned away
with a sentence naming both numbers. That sentence is the most likely thing a double-click ever
produces — a corpus opened by a development build and then double-clicked into an installed one is a
schema ahead — so where it goes matters as much as what it says: the Open page, in red, with the
chooser beside it. SQLite hands back rows from a table with columns this build has never heard of,
so without the number a `.kmbuild` written by a newer build would open *silently*, and the tool would
curate a corpus while ignoring whatever that build had added.

**The ladder is `step_to`, one arm per version above the floor.** A schema change adds the arm for
the next number and bumps the constant in the same change. A current database answers in one
`PRAGMA`.

**An unstamped file is either new or refused.** A brand-new database reaches `migrate` at 0 with no
tables at all, because `Db::prepare` checks the version before the schema batch runs, so the triggers
the batch recreates find every column they name. It is stamped current there and then, so no step
ever runs against a database with no tables. An unstamped file that *has* a `songs` table carries no
number this build can place, and is refused.

**A step that changes a key defers the foreign keys rather than ordering its writes.** A parent key
changing under existing children is a violation whichever table is written first, and `foreign_keys`
is ON for the life of the connection. `PRAGMA defer_foreign_keys` holds the check to the commit, by
which time both sides agree; `foreign_keys` itself is a no-op inside a transaction and cannot be used
for this. The rows to change are collected before anything is written, because they are the rows
being read.

**Ending a job is a `Drop`.** Setting `finished` only on the worker's success paths makes a panic
anywhere in the open permanent: the slot stays in flight for the life of the process and every later
open answers *already opening…*, with nothing to clear it but a restart — in a window with no console.
`EndsTheJob` ends it however the thread leaves, and the reason is written *before* `finished` so a page
that sees a finished job sees why.

`busy_timeout` is five seconds. SQLite's default is zero — any contention is an immediate
`SQLITE_BUSY` — which was survivable only while a corpus could have exactly one curator, and a
database that can be double-clicked can be double-clicked twice.

## The window, and the two executables

Behind the `desktop` feature — off in cargo, on when staging for Windows and macOS, never for Linux.
`tao` owns the window, `wry` puts a platform webview in it pointed at loopback, so every handler,
template and route is unchanged. It is a *viewer*, not a second front end.

`main` is not `#[tokio::main]`: `tao`'s `run` must own the main thread, never returns, and calls
`process::exit` on the way out, so the runtime is built by hand, the server is spawned rather than
awaited, and shutdown happens in `Event::LoopDestroyed`. A failed webview falls back to the browser
rather than to nothing — WebView2 is absent on Server and some LTSC builds, and a version check would
go stale.

**On Windows the crate is a library with two three-line binaries on it.** `km-package-builder.exe` is
GUI-subsystem; `km-package-builder-console.exe` is the same library with a console, and is the one to
type and the one that answers `--help`. Only the subsystem stops a console being created at all —
`FreeConsole` can only make an already-created window vanish again, and the subsystem is a property of
a *binary crate root*.

Four traps here, each of which is invisible from a terminal:

- **`FreeConsole` would make every `println!` panic.** `std::io::_print` panics rather than failing
  when stdout is gone, so a double-clicked build would have aborted every time and never once when
  tested from a shell. `km-console` is the answer: print when there is a console, log when there is
  not, decided once before the first line. It is the workspace's only `unsafe`.
- **`GetConsoleProcessList` answering zero is not the same as having nowhere to print.** A
  GUI-subsystem process has no console, but standard handles are inherited whatever the subsystem, so
  `--version | cat` writes down a real pipe and the staging scripts rely on exactly that. Zero asks a
  second question — whether stdout's handle is null or invalid — through the safe `AsRawHandle`.
- **`cmd /c start` gets a console of its own** when its parent has none, which is every browser open
  and every Play click from a windowed build. Hence `CREATE_NO_WINDOW` in `osopen`.
- **An error returned out of `start` reaches nothing.** `main` returning `Err` prints through std's
  `Termination`, which writes to a standard error handle a windowed run does not have — and the
  reason was never logged either, being returned rather than written down, so `--log-file` captured
  the one run it exists for least well. It is logged first now, and then shown: a corpus that will
  not open is handed to the Open page instead of opened here, and everything earlier than a server —
  an address already taken, a path naming neither a folder nor a `.kmbuild` — opens a small `tao`
  window with the sentence in it. `failure_html` lives in `lib.rs` rather than beside the window,
  because `desktop` is a feature the test commands never turn on and the escaping is what needs
  asserting.

The split between the two is *where a refusal would be read*, not *whether there is a window*:
standard handles are inherited whatever the subsystem, so a folder named at a prompt keeps the eager
open and the exit status a script reads. See
[`A corpus that will not open is a page`](../decisions/curation.md#a-corpus-that-will-not-open-is-a-page-and-a-failure-before-the-page-is-a-window).
One consequence is bought deliberately: an ordinary double-click now shows the window at once, with
the migration running behind it, where it used to appear only once the open had finished.

`km-remote` and `km-admin` return the same bind failure into the same kind of windowless `main`.
Neither handles a document and neither has an Open page, so the shape here does not transfer whole —
it is a known hole rather than a solved one.

`--open` is implied when nothing can be read: with no console the URL is printed nowhere, so a build
waiting to be asked would start, serve and show nothing. A window satisfies `--open`, so a windowed
build never also opens a tab.

### Registering the file type

`--register` writes the association for **this** executable wherever it currently is, which is what
makes it work for a portable folder unzipped anywhere — and why moving the folder means running it
again. **Per-user everywhere**, needing no elevation.

| Platform | What it writes |
|---|---|
| Windows | Three keys under `HKCU\Software\Classes`: `.kmbuild` → `KaraokeMachine.PackageBuilder`, its `DefaultIcon`, and `shell\open\command` = `"<exe>" "%1"`. The quotes around `%1` are load-bearing — a corpus path with a space is the normal case. |
| Linux | A shared-mime-info XML with a `*.kmbuild` glob, a `.desktop` entry with `Exec=<abs> %f`, and six icon sizes into `hicolor`. `%f`, not `%U`: a double-click delivers a path, and `%U` would offer URLs this tool cannot read. `update-mime-database` and `update-desktop-database` are best-effort. |
| macOS | **Nothing.** The type is declared in the bundle's own `Info.plist` and LaunchServices reads it when the `.app` is placed; `--register` only runs `lsregister -f`, and refuses by name on a bare executable. |

Run from the console twin, `--register` registers the **windowed** executable beside it: `--help`
lives on the console one, so that is the one somebody has in hand, and a `.kmbuild` associated with it
would open a browser tab from a file manager.

The macOS bundle declares a document type and the machine's does not, so it is a second plist with an
identifier of its own — LaunchServices keys its database on that, and a collision means one bundle
silently replaces the other. `CFBundleDocumentTypes` says *this application opens that type*;
`UTExportedTypeDeclarations` defines the type. Exported, because this product invents it.

`--no-desktop` takes the bundle **and** the window together. That is correctness, not tidiness: a
document double-clicked on macOS arrives as an Apple Event, so a bundle around a build with no event
loop would declare the type, be handed a corpus and have nowhere to put it.

## Collapsing a group of versions is a filter, not a `GROUP BY`

The song list shows one row per recording, and the row it shows is the group's representative. **The
predicate is `s.duplicate_of IS NULL`, prepended by `Filter::to_sql` beside the `s.merged_into IS
NULL` already there.** It is left out when the filter names one favorite, because a list shows every
song filed in it. Every alternative shape breaks something this page is built on:

- The orderings in `Filter::order_by` are chosen to be index seeks — `s.file_count DESC`,
  `s.sort_title` — and grouping destroys index-only ordering. That trade is 9.71 ms against 4.21 s
  on the real corpus.
- `Db::songs_page` asks for `limit + 1` and reads *is there another page* off whether it got it. A
  group straddling a page boundary makes that flag lie.
- `Db::song_count` runs the same `Filter::to_sql`, so a count taken through a different `WHERE` than
  the rows offers a page that comes back empty. Going through `to_sql` is what keeps them equal, and
  the total travels in the URL.
- The bulk forms post one `song_id` per row, and `SongRowFragment` redraws exactly one song by id
  after an inline edit. Neither has an answer for a row standing for six.

`version_count` rides along as a column, for the reason `file_count` is one: a browse row asking
`COUNT(*)` of another table is the shape that column exists to avoid, and this one is drawn on every
row of every page. It is written by the grouping pass alone and gets no triggers — nothing but a
grouping pass can change what it counts, where `file_count` moves whenever a file appears. The pass
writes it on the representative only; `browse_columns` reads a hidden version's count off its
representative through `duplicate_of`, one primary-key lookup per hidden row, and selects
`duplicate_of` itself so the row can mark and link it.

## The browse list writes

Every score, corrected name and favorite can be set from the list, not only from a detail page — over
a corpus of hundreds of thousands of files, one song at a time is not a workflow.

**One row, defined once.** `templates/song_row.html` is included by both the list and the fragment
route, and `db::browse_columns()` is the single `SELECT` both go through. Every change answers with
that fragment, swapped by `hx-target="closest tbody"`. Two copies would disagree the moment one was
touched, and the disagreement reads as *the row I just edited looks different from its neighbors*.
`a_row_renders_the_same_alone_as_it_does_in_the_list` pins it.

**A song is a `<tbody>`, and the browse table has as many of them as it has rows.** The favorite
chooser is a second `<tr>` under the song's own, so the element a control swaps has to be the group
rather than the line — otherwise going back to one row leaves the chooser behind. `closest tbody`
then works in both directions with no out-of-band delete and no ids to keep in step, and htmx parses
a response inside a `<template>`, which keeps a bare `<tbody>` fragment intact where the old table
parsing would have dropped it.

Rename gets its own route rather than posting two fields to `edit_song`: that handler treats every box
on its form as authoritative — which is what makes clearing a wrong artist possible — so a partial
form would clear language, transpose and notes.

**Favoriting is membership of a named favorite, and nothing else.** There is no `songs.favorite`
boolean; the star opens a chooser rendered as the same row, and `db::toggle_favorite` decides file or
unfile by reading the database rather than trusting a row that may be minutes old. The row carries
`favorite_count`, not a flag — one subquery in `browse_columns`, so a page of rows costs no extra
queries, and the star can say *in two favorites*, which a boolean never could.

**Two counts, because the fill and the color answer different questions.** `permanent_count` counts
the same memberships through a join to `favorites.temporary`, so the star is filled by the first and
colored by the second — a song in nothing but working lists is in lists and filed in none. Appended
last in `browse_columns` so no existing index into the row moves, and its own subquery rather than a
narrowing of the first because the row needs both numbers.

**Showing file names is a class on `#rows`, not a flag on the row.** The name is written into every
row's markup and revealed by `#rows.filenames .filename`. Threading a bool down would have had to reach
the fragment routes too — which never see the browse query — so a row would have lost its file name the
moment anybody scored it.

### Two rules that are walked into repeatedly

**Nothing rendered inside `#rows` may use a name `FilterQuery` knows.** `#rows` is `hx-include`d whole
beside `#filters`, and serde answers a repeated *known* key with `duplicate_field` — so a row select
named `language` turns two working buttons into a 400. Hence `row_language` and `set_language`, and
hence the impossibility of a hidden `filename=0`. Keys the struct does not know are ignored however
often they repeat, which is what keeps a hundred `song_id`s legal. Two tests hold it, one from each end
of the wire.

**The count reads the bar; the write reads what was counted.** An `hx-post` attribute is rendered once
and the filter bar never re-renders the page, so a filter baked into an action's URL is a description of
the page as it *arrived*. Phase one takes the filter from the POST **body** (`hx-include="#filters"`),
because that is the only current copy. Phase two — `confirm=1` — takes it from the **query string the
confirmation handed back**, because a bar changed while a confirmation sat on screen must not widen a
write nobody was shown. `ConfirmQuery` is the whole discriminator. An unreadable body refuses in words:
`unwrap_or_default()` there is a package holding the entire corpus.

The same staleness had a quieter victim in the *showing* chips, so `filter_chips.html` is its own
fragment, its container is in the DOM **unconditionally** — htmx silently drops an out-of-band swap
whose id is not on the page — and `/songs/rows` sends it back beside the rows and pushes `HX-Push-Url`
so a reload or a bookmark keeps the filter. `saved_filters.html` is the same fragment shape for the
same reason, and its container is on the page even with nothing in it because the control that saves
the first one lives inside it.

It is a **sibling** of `#filters`, not a band in it: it holds a `<form>`, which a browser drops when
nested, and `#filters` fires on every `change`. The border round both comes from `.filter-box`, the
element `songs.html` wraps them in — so `form.filters`' own chrome stays where the six curation-tab
forms still need it and `.filter-box > form.filters.banded` gives it up. `#saved-filter-result` is
inside that box and outside the strip: a swap target inside the element an out-of-band swap replaces
is the hazard `filter_chips.html` documents from the other direction.

**Saving a filter is the exception to both, and the exception is worth the paragraph.** It sends no
bar and parses no filter out of its body. Every change to the bar goes through `/songs/rows`, which
writes the canonical query string into `State::songs_filter` before it answers — so the server
already holds the exact string the address bar shows, `offset` clamped and all. That makes *the count
reads the bar* vacuous here and the `duplicate_field` hazard unreachable. What it costs is a
dependency: a route that re-renders `#rows` without writing the filter down leaves this saving a page
nobody is on. Four routes redraw the rows — `GET /songs`, `GET /songs/rows`,
`POST /songs/titles-from-filename` and `POST /songs/fix-name-case` — and a test holds each of them to
writing down the offset it drew, which is the clamped one and not the one asked for. The last two
share `redraw_over_the_write`, so the rule has one home rather than two copies. The similar-names page
posts the same two routes with `?as=hits` and the search bar ahead of the ticks; `Redraw` reads that
bar with `SimilarQuery::from_fields` before the write, and the answer is `#hits` drawn by
`similar_for`, with no filter written down.

A chip is `saved_filter_chip.html`, included by the strip and rendered on its own by
`GET /songs/saved-filters/{id}/chip` — `song_rows.html` and `song_row_fragment.html` over the same
markup, so the strip and the one chip a rename reopens cannot drift. The `renaming` flag is a field
on **both** view structs because askama's `include` shares the enclosing scope, which is why
`SavedFilters` carries one it never sets. Rewriting and renaming answer with the out-of-band strip;
only opening and closing the box answers with the chip, so a second box somebody has open is left
alone.

### The quality hint, and why it does not redraw the rows

`crate::hint` holds the whole of the ordering: a `Key` of the columns a scan wrote and an `order`
that sorts them. `Db::quality_hint` fetches the keys for a list of ids restricted to `kind = 'midi'`
and hands them over — **no `ORDER BY`**, because no index covers seven keys over an arbitrary list,
the list is at most a page of ticks, and a comparator can carry the reasoning that produced it where
a formatted SQL string cannot. The decision is
`A quality hint is a position on the row, and it is rubbed out rather than kept` in
[`curation.md`](../decisions/curation.md).

The result lives in `State::quality_hint` as an ordered `Vec<String>`, position being the number, and
is emptied by `State::publish` along with the outgoing folder's filter. `State::mark_hints` writes
each row's place onto it in `rows_for`, which is the one place both `/songs` and `/songs/rows` build
their rows — the `tags` pattern, one level up, because where a song sits in this run's hint is not a
fact the browse query could select. `similar_for` calls it too, so `/similar` and `/similar/hits`
draw a match's number the same way. The three fragment routes that re-render one row call
`mark_hint`, for the reason they already carry the last-played highlight: a row swapped back after an
inline edit must not come back having lost its badge.

**The answer is out-of-band badges and never a redraw of `#rows`.** Two things point that way. A
badge is a few characters inside a row that is otherwise untouched, and redrawing the rows to place
one would put these two routes under the rule the paragraph above states — they would each owe
`State::songs_filter` the offset they drew, and the list of routes that redraw the rows would grow by
two. And a hinted song that is not on the page being looked at has no element to swap, which htmx
answers by dropping the swap; the row draws its number from `SongRow::hint` when that page is
reached. `hint_marks.html` carries an empty badge for every song that has *lost* its number, because
a row keeps whatever markup it was last given, and `.place:empty` is what takes the mark off it.

`POST /songs/quality-hint` reads `song_id` out of the body with `Fields` rather than through
`BulkAction`: what that extractor adds is a whole-filter scope and a confirmation pass, and this has
neither. The form is `hx-include="#rows"` alone — no filter is read here, so the bar has no reason to
be in the body, which is also the end of the `duplicate_field` hazard for this one. The similar-names
page posts the same route with `hx-include="#hits"`, where its ticks are.

### Forms, templates and the one script

**Form bodies are not read with axum's `Form` extractor.** It goes through `serde_urlencoded`, which
cannot represent a repeated key — and a repeated key is exactly what a form of ticked checkboxes sends.
Asking for a `Vec<String>` errors; asking for a `HashMap` is worse, silently keeping one value, so
ticking three favorites would file the song in one and say nothing. `src/form.rs` parses the body.

Templates are askama, compiled in and checked at build time. **htmx, the stylesheet and the license are
compiled in too** — a machine with a corpus on it may have no internet, and serving them off disk cost
an `--assets` flag, a three-directory search, a startup check and a failure mode where every button is
inert because a copy left one directory behind. The one thing `ServeDir` did that must now be done by
hand is the `Content-Type`; a test pins the bytes, since `include_str!` catches a missing file at build
time but not an empty one.

**htmx does not swap the response of a failed request**, by design, so a page turn that errored looked
identical to one with nowhere to go. Answering 200 so htmx will swap is right wherever a refusal has a
fragment and a slot to come back to — but `/songs/rows` has neither, and swapping error text into
`#rows` costs the rows you were reading. So the status stays honest and `static/ui.js` — the tool's only
JavaScript — listens for htmx's error events and puts a line in a toast tray. Its doc comment says what
it may not become: no client-side model, no templating in the browser, no state the server does not hold.

**The two selection gestures are in that file for the same reason**: the box in the table head and the
shift-click that ticks the run between two boxes are things htmx has no opinion about rather than gaps
in it. Neither breaks the rule above, and the run is the one that looks as though it might — what it
holds is the box the last press was on, and the selection stays where it has always been, in the ticked
boxes, read off the page by `hx-include="#rows"`. Both are bound to `document.body` rather than to the
elements, because `#rows` is replaced outright by every page turn and every filter change.

**The words are a catalog, and one argument carries it.** `i18n/en.ftl` and `i18n/pt-BR.ftl` sit
beside the templates and are `include_str!`-ed by `src/words.rs`; `{{ "nav-songs"|t }}` spends a key
and the five render seams in `src/views.rs` take a `km_locale::Locale` and pass a values store in
once. askama carries that store into a nested `{{ child|safe }}`, so no template struct holds a
locale and no fragment has to be told. `toast.html` and `message.html` take none: both are one
interpolation of a sentence somebody else already worded.

**The store is `&dyn Any`, which is neither `Send` nor `Sync`.** All five seams are synchronous for
that reason — a render held across an `.await` makes the whole handler non-`Send`, and axum reports
that as `Handler` not implemented for the function, naming neither the line nor the value.

**Markup carries a key and nothing else; a sentence carrying a value is composed in Rust.** A count
is a plural and a plural is arithmetic, so the header's counts, a confirmation's subject and button,
a pager's range and a row's four tooltips all arrive as fields. `State::say_rows` is where a page of
rows gets them, beside `mark_hints` and for the same reason: what a row *says* is a fact about the
page rather than about the corpus.

**What is worked out where no language is in reach travels as a fact.** A scan's phase is a key
(`scan::phase`), a build's is an enum carrying its own count or file name (`build::Phase`), an open's
is `db::OpeningPhase`, a scan status is `ScanStatus::key`, and `DbError` and `AppError` each keep an
English `Display` for the log and grow a `say` for the page. The console banner asks `OpeningPhase`
for its English rendering, so one set of words serves both.

**`static/ui.js` reads its five sentences off `<body>`** as `data-js-` attributes, the way
`km-remote-pages`'s `scan.js` already does: a static file cannot go through the `|t` filter, and an
English string in one would appear inside a Portuguese page with nothing to catch it. Two of them
carry `{what}` and `{status}`, which only the browser has, so the catalog composes the pattern with
those words standing in and the script does one replace into `textContent`.

**Four tests in `src/words.rs` are what make the whole of it checkable**: the two locales hold the
same keys, every key the markup or the Rust asks for exists, nothing in a catalog goes unspent, and
`no_template_carries_its_own_prose` reads every template with the markup stripped and fails on
whatever words are left. `server.rs`'s `no_page_draws_a_key_in_either_language` drives the real
router over every page in both languages and asserts no `⟦` reaches one.


Toast text goes in with `textContent`, never `innerHTML`: what arrives quotes file names out of a corpus
nobody wrote. Nothing about *becoming visible* is animated — a browser throttling animations in a
background tab left a fade-in at zero, so the one thing that had to be read was invisible; the element is
drawn by its own properties and the animation is a delayed fade *out*.

Successful list actions toast too, out-of-band rather than through an `HX-Trigger` header, because
`XMLHttpRequest.getResponseHeader` decodes as ISO-8859-1 and these messages are full of Portuguese
titles. A `MutationObserver` arms whatever appears in the tray — an out-of-band swap is announced by
different events depending on how htmx got there, and a toast that is never armed stays on screen for
ever with `pointer-events: auto`, swallowing clicks.

### Tags: two tables, and why the vocabulary is one of them

`tags` and `song_tags` have the shape of `favorites` / `song_favorites`, and what separates them is
what they mean: a favorite is somebody's own filing of this corpus, a tag is a word about the song
that the machine's own catalog carries. Both are new tables — so `schema.sql`, which runs in full on
every open, is the whole migration.

**The `tags` table is the vocabulary, and it exists so that *what tags are there?* is a read.**
`Db::languages_present` beside it has to reach for a `WITH RECURSIVE` skip-scan over `songs`, because
a language is a column and the distinct set has to be computed. A row appears the first time a tag is
used and goes when its last song loses it, so the picker is a list of words in use rather than a
museum of every word ever typed — which on hundreds of thousands of songs is the difference between a
useful datalist and one nobody reads.

**Neither `create_browse_indexes` nor the FTS triggers are touched, and both are worth stating**
because the rest of this page would suggest otherwise. The filter is an `EXISTS` against an indexed
join table rather than an expression on `songs`, so there is no `songs_browse_*` companion to add.
And a tag write touches neither `songs` nor the columns `songs_fts_update` names, so a bulk write
over a filter cannot retokenize a title — which the obvious alternative design, a `songs.tags` column
here as the machine's catalog has, would have done to every row it touched. There is a test.

**The suggested vocabulary is not in this database at all.** It is in the curator's own config
directory beside `recent.json`, because what a good tag looks like follows a person from one folder
to the next while a machine's address does not — see `settings.rs`. `spec_for` reads `song_tags`, so
a suggestion nobody has put on a song has no row and therefore cannot reach a package; a catalog is
built from packages, so it cannot reach a machine or a phone either. That path is asserted end to end
rather than described.

## Scanning

Shaped after `km-lyrics scan`: collect paths, chunk across scoped threads, `catch_unwind` per file, a
panic hook that records rather than prints. Results are written through **one writer thread owning the
connection**, batching 500 rows per transaction — `rusqlite::Connection` cannot be shared and a mutex
per row would be worse than either.

**The channel is bounded**, at four batches. Unbounded, a run over a whole corpus put hundreds of
thousands of
parsed songs into it: memory nobody budgeted, a progress count that was a claim about reading rather
than about the database, and a scan that could not be stopped promptly because stopping means draining.
Throughput is unchanged — a pipeline runs at the speed of its slowest stage whatever the buffer.

**Raising the bound needs `tune`'s mapping taken off in the same change.** Mapped, the pages the writer
descends live in the OS page cache rather than in SQLite's own, and a deeper channel is exactly what
lets the readers run far enough ahead to stream a corpus of files through that cache — which evicts the
database out from under the writer and turns every descent into a fault against the disk. Measured over
the real corpus, a bound raised tenfold with the mapping on gave a ninth of the throughput of the same
bound with it off, and of the shipped bound either way. The sort keys also decide more than they look
like they do: six of the nine browse indexes `create_browse_indexes` builds key on them through
`WITHIN_TITLE_KEY`, so a scan that clears and rewrites them rewrites those six as well.

**The writer is that slowest stage, and what it spends a scan doing is index maintenance.** Each batch
upserts into `songs` against its secondary indexes and fires the triggers that keep two full-text
indexes and the sort keys in step, all inside one transaction on one thread. So the `songs` triggers
each carry a `WHEN` comparing the columns their body reads: `UPDATE OF` fires on assignment rather
than on change, the upsert assigns every detected column on every row it writes, and a re-analysis
exists to move suitability while leaving a song named what the file already spells it. Without the
guard each of those rows is deleted from both full-text indexes and reinserted, and has both sort keys
cleared and written back, to arrive at what is already stored.

**Two further ways of making that writer cheaper were measured, and neither moved it.** Putting every
index on `songs` and `files` aside for the length of a forced pass and rebuilding them at the end read
16.6 songs a second against 17.3 without it, and cost 215 seconds of rebuilding on top. Writing each
batch's songs in id order, so the primary key is descended in order rather than at random, read 17.1.
**What is left after the guards is neither index maintenance nor write ordering**: it is reading a few
hundred scattered table pages off a platter per batch, and a song's id is the hash of its bytes, so
those rows are scattered by construction. Neither dropping an index nor sorting a batch changes how
many distinct pages a few hundred scattered rows sit on. A reordering wide enough to coalesce them
would have to buffer far more rows than one batch, which is the bound the note above warns against
raising while the database is mapped.

**The bar counts rows committed, not files read.** `written` is counted by the writer after each batch
commits, so the bar is `(skipped + written) / total`; a skipped file never reaches the writer and settles
at once. Paging is still `OFFSET`-based, so a song can be missed across a page boundary during a scan;
keyset paging on the existing `ORDER BY` tuple is the fix if that matters.

**The panel under the bar is the run's steps, listed before they run.** `run_inner` plans them from the
options first: a scoped run lists no forgetting and no duplicates, and the three steps that depend on
`changed` carry `if_changed` and are skipped when it is false. `Progress::say` closes the running step
and starts the next, `end` settles whatever is left, and `timings()` is read off the finished steps.
The walk goes through `km_pack::collect_songs_observed`, which reports the count after each folder into
`found` and breaks off when the run is asked to stop; a broken walk ends the run before reading,
because its list is part of the folder. **The time left comes from a 30-second window** of settled
counts, sampled at most once a second by `snapshot` while the reading step runs, and is shown only
once the window spans ten seconds.

**Ctrl-C stops the scan rather than killing it, and so does the page's Stop button.** `Progress`
carries a `cancel` flag the workers check *between* files — one file is milliseconds, and stopping
half-way through parsing gains nothing. The writer deliberately does not check it: draining what has
been read is the point, and the bound is what keeps that under a second. A second Ctrl-C exits at
once. `POST /scan/stop` sets the flag through `Workspace::ask_scan_to_stop` and does not join, so the
request answers at once and the panel's poll reports the end; joining stays with `Workspace::drop`.

**A stopped run draws none of a finished run's conclusions.** Recording `last_scan` would claim the
folder had been read when it had not. So a
canceled run commits and returns, and the page says *stopped*, noting that re-running resumes free.

**Incremental by default.** A path whose size and mtime match its row is skipped before it is opened.
`--force` re-analyzes everything, which is what to do after the analysis heuristics change and never
otherwise. Files gone from disk are forgotten **except** where a package still names the song: that one
is kept and shown as *source missing*, because losing a curated selection to an unmounted drive is the
one failure this must not have.

**The corpus is walked once, not once per kind.** `km_pack::collect_songs` classifies each entry off
the directory listing; the three per-kind collectors it replaces each opened every directory under the
root and stated every entry, so a mixed corpus was walked three times to answer one question. They stay
for `km-pack spec`, which wants one kind at a time.

**A file is read once.** Reading and hashing before dispatching wastes the read for an MP3+G pair,
whose identity is the hash of *both* halves — the audio ends up read three times and the graphics
twice. So the audio branch sits above the read and `km_cdg::probe_from` probes the bytes already in
hand. A video still reads twice — there the hash *is*
the identity, so the bytes must be gone through, and `km_video::probe` takes a path.

### The tail, and what a hold of the writer costs

The passes after the reading are the only stretch of a scan that holds the writing connection for
longer than a batch. **What waits on that connection is every write and no read**: a page is drawn
through a connection that can only read, so the tail is felt by a star somebody clicks rather than by
the pages. See [`Two connections, and which one a page is drawn through`](#two-connections-and-which-one-a-page-is-drawn-through).

**Two of those passes cannot be taken in bites, and that is what a write pays for.** A folder tree is
one pass over every file and `ANALYZE` is one statement, so neither can release the connection
part-way and `stand_off` reaches only the gap between them. Measured at the end of a scan over a whole
corpus that had something to write:

| phase | measured |
|---|---|
| looking for files | 3.0 s |
| reading and analyzing | 145 m 41 s |
| forgetting files that are gone | 252 ms |
| indexing folders | 12 m 59 s |
| measuring the corpus for the query planner | 11 m 17 s |

So for the best part of half an hour at the end of such a scan, a write waits out `WRITE_WAIT` and is
answered `DbError::Busy`. Reads are unaffected throughout. Breaking those two passes up, or deferring
them, is not attempted here and is what it would take to close that window. The folder tree is paid
here rather than on the Folders page so that the page opens at once after a scan; see
[`The corpus is browsed by folder`](../decisions/curation.md#the-corpus-is-browsed-by-folder).

**The whole tail is gated on a row having changed**, and nothing above that gate could reach a
different answer without one: the folder tree would be rebuilt from unchanged paths and `ANALYZE`
would measure a corpus whose shape had not moved. `last_scan` stays unconditional — it records that the
folder was *read*, which is true of a scan with nothing to do.

**`forget_missing` is handed what is gone, not what is present.** Handing it every path on disk means
inserting them all into a temp table one row at a time and deleting with two `NOT IN` anti-joins
that scan `files` and `songs` whole — paid in full on every completed scan, and the ordinary scan
deletes nothing. The set difference costs nothing where it sits instead: `known_files` already holds
every path in the table and the walk already holds every path on disk, so the caller subtracts one
from the other and passes only the rows that go, deleted by exact path on the `path` unique index.
Empty does no SQL at all.

The orphan sweep follows from that: **a song can only lose its last file when one of its files is
deleted**, so it is scoped to the songs whose files just went, two index seeks each, rather than a full
scan of `songs`. `file_count = 0` is kept as a backstop for an orphan an *earlier* version left behind,
which the whole-table statement caught as a side effect; it is a seek on `songs_file_count`, not a scan.

**A foreign key onto the tables this deletes from was unindexed.** `package_songs.file_id` is
`ON DELETE SET NULL` and nothing else reads it, so with `foreign_keys = ON` each deleted row scanned
the child table whole. It is indexed now — latent while nothing is deleted, quadratic the day a drive
is reorganized under the corpus. `duplicate_candidates.b_id` was the second half of this and went with
the table.

**The lock is held in bites.** Deletes re-take it per chunk, the way the writer already does per
batch, so the tool is never unanswerable for the length of a whole-corpus delete.

**Every phase is timed**, and the Scan page says so. Four of the seven are whole-corpus passes and the
only thing ever reported about any of them was its name, so "which phase" — the first question of every
complaint that the scan is slow — had no answer short of attaching a profiler to somebody's corpus.

Measured on the real corpus — **large, on a spinning disk** — re-scanning with nothing
altered on disk:

| phase | before | after |
|---|---|---|
| looking for files | three walks | **15.5 s**, one walk |
| reading and analyzing | | 1 m 01 s, every file skipped |
| forgetting files that are gone | minutes | **202 ms** |
| looking for near-duplicates | whole `songs` table | **not run** — a forced scan's phase, and a button |
| indexing folders | whole `files` table | **not run** |
| measuring the corpus | full `ANALYZE` | **not run** |
| **the whole scan** | | **80 s** |

The 202 ms is the set difference and nothing else: `gone` was empty, which is also the strongest
available evidence that `collect_songs` finds exactly what the three collectors found — every path
matched the table row for row, and a single kind dropped from that walk would have deleted every file
of it. `one_walk_finds_exactly_what_three_walks_found` is the test that stops it being evidence about
one corpus.

The freeze that started all this is now too short to observe, which is the honest limit of what was
verified: the shorter lock holds are argued rather than measured, because no tail is left long enough
to load a page during.

**A scan skips a file only when nothing about it *and* nothing about the analysis has moved.**
`songs.analysis_revision` carries `km_suitability::ANALYSIS_REVISION`, `known_files` joins it in
beside the size and time, and the skip check asks for all three. `files_song` and the `songs` primary
key make that join a keyed lookup per row rather than a second scan of `files` — the one
place this could have quietly cost minutes, and the thing to re-measure if an open ever slows down.
Schema ladder step 11 adds the column and leaves it NULL, which is what every row written before it
means: nobody recorded what produced them.

`the_analysis_revision_covers_what_the_fixtures_say` is the guard, and it hashes the tuning constants
as well as the fixtures' results. **Measured while writing it**: moving `sync_window_ms` from 120 ms
to 119 changes not one fixture, and would change thousands of rows in a corpus of hundreds of
thousands — so the results alone were not enough, and `Thresholds`' `Debug` output goes into the
digest for the same reason a derive is safer than a field list.

**`Db::promote_unreached_revisions` runs on every writing open**, after the text clean-up and before
anything counts stale songs. For each entry of `km_suitability::REVISIONS` in order, one `UPDATE`
raises the rows at the revision before it that its `Reach` excludes; `Reach::Everything` runs
nothing. So a row climbs until the first revision that reaches it. `LyricLinesAtLeast(n)` excludes a
`line_count` below `n` and a NULL one, which is every video and MP3+G row; `SyllablesAtLeast(n)` does
the same over `syllable_count`. A NULL revision is never promoted. On an open with nothing to promote
each statement matches no row.

| Revision | Reach | Why |
|---|---|---|
| 2 | 1 or more syllables | the melody's presence gate passes every channel when there are no words |
| 3 | 8 or more lyric lines | a chord chart needs `min_chord_lines` chord lines, each a lyric line |

**`--reanalyze` is that scan with the set chosen from the database rather than from the disk.** It
takes `paths_matching(&scan::every_song())` and hands it to `ScanOptions::only`, so it reads one copy
per song where an ordinary scan reads every copy: the skip check works from a snapshot taken before
the run, so the second and third copies of a song still look stale in the same pass that repaired the
first. `reanalyze_and_exit` polls the same `Progress` the page would and reports it through
`km_console::Meter`.
What it saves is the reading: `paths_matching` selects one path per song, **a bit over half the files**,
and a scoped run asks none of the whole-corpus questions — no forgetting, no `last_scan` stamp, no
near-duplicate pass.

**A re-analysis over a whole corpus on a spinning disk is bound by the writer, not by the reading.**
The readers outrun the one writer thread and spend the run blocked on the bounded channel, so what the
drive is busy with is a single thread's index maintenance rather than songs. Measured on the real
corpus, a batch commits about once a minute while every reader thread sits in a wait state at a
fraction of one core between commits, and the readers' own speed shows only in the moment a commit
frees the channel, when they empty it at streaming speed and block again.

**A process at a fraction of one core is what waiting looks like as much as what seeking looks like**,
so a rate alone says nothing about which stage is the slow one. Reading one folder end to end runs at
243 files a second, and extrapolating from that under-calls a scattered read by an order of magnitude;
neither figure says anything about the write path.

**Measure a change to any of this against a drive that is actually being read.** Repeating a
`--reanalyze` restarts it at the beginning of the sorted path list, so the second run and every one
after it re-reads files the page cache already holds: the disk counters go to zero reads, the figure
on screen climbs run after run, and what is being timed is how fast the machine appends to a
write-ahead log with the whole corpus in RAM. A measurement with no disk reads under it is not a
measurement of this, and comparing two settings in the order they were thought of credits the second
with the warming the first paid for.

What the scoping buys is real and is a fraction, not an escape: the
forced scan reads every file instead and ends with the near-duplicate pass on top. What it still pays is the walk and the `known_files` load, because `only` narrows
the path list *after* `collect_songs` rather than instead of it, so a scoped run reaches a file by the
route a whole one does and cannot disagree with it about what is there. At 15.5 s the walk is not what
anybody is waiting for.

**Two traps live in that function and both are load-bearing.** `Filter`'s default collapses a cluster
to its representative — `s.duplicate_of IS NULL` — so asking for the default would measure the
representatives and report having finished;
`a_version_set_aside_is_still_a_song_to_re_analyze` is what says the two counts differ. And
`ProgressView::percent` is the share *written*, not read, so a line pairing it with `done` reads as a
run doing nothing through the first batch; the meter counts songs read instead.

**The meter itself is `km-console`'s**, not this crate's, because every command grew its own and they
disagreed: `km-pack` rewrites a line for its walk and another for its packaging, neither says a rate,
and only one survives being piped. `Meter` says the count, the total, the percentage, the rate and the
time left, rewrites in place where there is a cursor and prints whole lines where there is not, and
holds its tongue about speed for the first two seconds — the first draw is immediate so the run is
visibly alive, and a count divided by a fraction of a second is a five-figure rate that has measured
nothing.

**It goes after the Ctrl-C watchdog where `--backup` goes before it**, and runs through `Workspace`
rather than calling `scan::run` directly. Both for the same reason: this one writes a quarter of a
million rows over twenty minutes, so it needs an interrupt that ends it cleanly and the `Db::close`
checkpoint that `Workspace::drop` performs, neither of which a backup has any use for.

**And the windowed executable refuses the flag**, which `detaches_from_the_shell` decides — a rule over
`Shell` rather than a platform test, so `only_the_windowed_executable_hands_its_prompt_back` runs on all
three. `main.rs` asks for a GUI subsystem wherever there is a window, and a command processor does not
wait for one of those: the prompt comes back, the run carries on behind it, and the progress this
function exists to print lands on a prompt that has moved on. The button is the windowed build's
answer, and the refusal names it.

**The button's field and the template's `name` are one string in two files**, which is the shape that
rots without a round trip: rename either and the button still renders, still posts, and quietly starts
a whole forced scan instead of the cheap pass it offers.
`re_analyzing_the_songs_does_not_claim_the_folder_was_scanned` posts what the page posts and asserts on
`last_scan`, which only a whole run moves.

**Exact duplicates need no pass, and the near-duplicate pass is the forced scan's alone.** The
content hash already collapses identical files onto one row, and the count is a column on it.
Bucketing a coarse structural fingerprint, keying every lyric, and joining the pairs into groups is a
whole-`songs`-table read — measured at 10.4 s over the whole corpus, which is worth asking for and not
worth paying for a file that moved. So it is the tail of **Re-analyze everything**, which has just
rewritten every fingerprint it reads, and a button on the Duplicates page for every other time. The
scan that found a changed file runs neither. See `Duplicate aggregation` in
docs/decisions/curation.md.

**The lyric key is computed from the stored `lyrics` column and never written back.** `Db::fingerprints`
folds each row's words as it reads them and keeps only the hash, so the text of the songs that
have any is read and dropped rather than held. Keeping the key out of the table is also what makes the
word floor and the credit rule cost a button press to revise rather than a `--force` re-scan.

**The lyric pass emits a star and the fingerprint pass emits a clique**, which is a difference in
pair count and nothing more, because grouping is what reconciles them.

**A dismissal is a constraint on the grouping rather than a deletion of an edge**, and it has to be,
for two reasons that arrive from opposite directions. Joining is transitive and a dismissal is not,
so a clique reconnects the two songs a person separated through any third member. And a star holds no
edge between two of its leaves at all, while the song page offers *Not the same* between every pair
of versions — so a dismissal there had nothing to update and was silently lost. `Db::dismiss_pair`
therefore inserts the verdict when no pair was suggested, and `DisjointSet::union` refuses an edge
that would put a dismissed pair in one group. Edges are sorted first: once an edge can be refused,
which one is dropped decides where a chain breaks, and that must not depend on the order SQLite
returned its rows in.


## Building a package

The build runs on its own thread in three phases: a short lock to read the description, then the
parsing, analysis, any ffmpeg re-encode and the archive write **with no lock at all**, then a short lock
to record what was built. `crate::build::build` therefore takes the `Arc<Mutex<Db>>` rather than a
`&Db` — a signature that looks like a downgrade and is the point, because it decides *when* to lock.

Run inside the request it took the workspace's single mutex for the whole build, so every other page
and every poll queued behind it; a progress bar added to that would have been the one request that
could not be answered.

- **`Workspace::drop` stops the build before the checkpoint.** A build holds no lock for most of its
  life, so `db.lock()` in `Drop` succeeds while one is running, closes the connection under it, and
  leaves the final `record_build` writing into a closed database.
- **Stopping costs at most one song.** `km_pack::build` asks whether to carry on *between* songs rather
  than inside ffmpeg. The archive goes through a temporary file and a rename, so a stopped build leaves
  the package it was replacing exactly as it was.
- **One build at a time**, deliberately serialized: two threads would both call `record_build`, and the
  page has one place to put a bar.

`packages.default_language` fills any song with none **in the package, never writing back to `songs`**.
That clause is the whole design: a package saying *call the rest English* is a statement about one
package, while the corpus goes on saying *nobody has said*. Absent and blank differ in the form — the
settings form carries the key and may leave it empty, which restores the strict refusal.

**A package's page also writes its description**, the same `*.kmspec.yaml` `km-pack build` takes, from
the same value the build here consumes — so there is no second code path that could describe a package
differently from the way it is built. A test asserts the two manifests are identical.

`dismissed_failures` keys on `(file_id, scan_status)` and needs no migration step: `schema.sql` runs
on every open and creates it with `IF NOT EXISTS`. A rescan updates a file row in place through
`ON CONFLICT(path)`, so an id survives one and a dismissal holds; a file deleted from disk takes its
dismissal with it through the cascade. Both sides of the panel come from one `Db::tally`, which
takes the side as a parameter so the count and the example it shows can never be drawn from
different populations.

**The version is raised across two of the three phases**, and the split is what keeps the archive and
the row in step. Phase one computes the raised version and puts it into the description the build is
about to consume; phase three writes it to `packages.package_version`, inside `if outcome.wrote()`
beside `record_build`. So a package that was never written spends no number, and one that was
carries the number the row reports. `spec_for` is untouched, which is why **Write spec** describes
the package as it stands: writing a description is not a build.

`packages.raise_version` is read and written through `Db::raise_version` and `Db::set_raise_version`
rather than as a field on `PackageRow`. `update_package` sets every editable column from a row
`handlers::package_row` builds out of a form, and neither the create form nor the settings form
carries the tick box — a field on the row would be cleared every time somebody saved a package's
name. `Db::set_package_version` is narrow for the matching reason: the build's last lock holds no
`PackageRow`, and must not overwrite a name or a language edited during the minutes it was unlocked.

## Sourcing a package from favorites

One table, `package_favorites`, and no migration step: it is new, and `schema.sql` runs in full on
every open with `IF NOT EXISTS` — the `tags` / `song_tags` arrangement. Nothing is added to
`packages`, because a row in that table is the whole of whether a package is sourced.

**One SQL constant is the union, and the count and the write both run it.** `WANTED_SQL` is
interpolated by four statements, all binding `:package`: the ordered read the placement walks, the
delete's `NOT IN`, and the three counts `package_sync_plan` answers with. A confirmation saying twelve
songs go in, over a write that reads a different set, is a number nobody can check — the discipline
`MERGE_SQL` already holds one method over. It is driven from `package_favorites` rather than from
`songs`, so a union costs the size of the lists and not the size of the corpus, and its `ORDER BY` is
`filter::WITHIN_TITLE` verbatim so a package numbers in the order the browse list draws.

**The deletes come first inside the one transaction, and that is what makes gap-filling possible.**
`number` is half `package_songs`' primary key, so a freed slot cannot be handed out while the row
holding it is still there. It also removes the need for `renumber_package`'s two-pass walk through
negative numbers: nothing already in the package moves, so there is no collision to park around.
`free_numbers` then walks each volume's `start_number ..= MAX_SLOT` filtering out what survived — 999
candidates a volume, so the naive filter is exact and free.

**`place_songs` is the one copy of the insert.** A free function over the connection, for
`next_number`'s reason: `add_to_package` hands it the append into the last volume that `package_room`
promises, the sync hands it `free_numbers` across every volume, and the already-there skip, the clash
count, the best-file lookup and the insert are written once. The iterator yields a volume and a number
together. Running out of numbers is the iterator running out, which is the ceiling said once rather
than once per caller. `package_room` and `next_number` answer the hand add's question, and only a sync
makes gaps.

**Two pages read the sources and one query answers both.** `package_sources_all` returns every link
with both names; the Packages page groups it by package to draw its mark and the Favorites page groups
it by favorite to word its Delete. A query apiece would be two ideas of what a source is, and a row
apiece would be a round trip per package.

**The member table is a fragment** — `templates/package_members.html`, carrying its own id and
`hx-swap-oob` — because a sync rewrites it. Remove and Re-flow still answer *reload to see them*: each
changes one row or every number, with the person who pressed it looking at the result, where a sync's
removals are rows that were never on the screen.

## Volumes

The decision is [`A package holds volumes`](../decisions/curation.md#a-package-holds-volumes).

**`package_volumes` holds what differs between two files of a package**: the id a manifest carries,
the version, the first number, the output path and the build time, keyed by `(package_id, volume)`.
`package_songs` carries `volume` beside `package_id`, with the primary key `(package_id, volume,
number)` and a foreign key onto the volume. **`UNIQUE (package_id, song_id)` stayed as it was, and
that constraint is what keeps a song in one volume**: the rule is enforced by the table rather than
by a check each write remembers.

**Migration arm 12 is an identity.** Every package gains volume 1 under its own id, carrying the four
columns `packages` gives up, and every member lands in volume 1 at its number. `package_songs` is
rebuilt rather than altered, because its primary key changes and SQLite cannot alter one; the arm
writes the table definitions out itself, in one transaction with `foreign_keys` off around it, and
`schema.sql` then finds both tables made. Arm 6, which renames typed ids, renames through
`package_volumes` as well when the table is there.

**`PackageRow` is a package seen through one volume**, read through `sql::PACKAGE_COLUMNS` joined to
`package_volumes`. `packages()` and `package()` read volume 1, `package_volume()` reads the one asked
for, and `package_volumes()` reads them all. The row carries the volume number, the volume's id, the
volume count and the whole package's song count beside the volume's own, and `volume_name()` is the
one place the numbered name is spelled. A second row type would have split every page's reads in two
for the one page that draws a strip.

**The sync plans the volumes it needs before it places anything.** `volume_starts` reads every volume,
the free numbers across them are counted, `volumes_needed` turns the overflow into a count of 999-wide
volumes, and `add_volume` inserts each under `PackageMeta::new_id` inside the sync's transaction. Only
then does one iterator chain every volume's free numbers into `place_songs`, so the placement walk has
no branch for running out. `package_sync_plan` answers `new_volumes` with the same arithmetic in SQL.

**The package page takes `?volume=` on every route that acts on one file**: the page itself, the
volume's own settings, build, the build's output name and progress, the description, install, Re-flow
and Remove. `VolumeQuery` reads it, absent meaning 1. `POST /packages/{id}/settings` writes only the
package's name, publisher and language through `update_package_details`, and
`POST /packages/{id}/volume` writes a version or a first number through `update_volume`, leaving the
field its form did not send. **The Build tab is its own fragment, `BuildPane`**, with a volume picker
that asks `GET /packages/{id}/build/pane` for the tab of the volume picked, so the Details strip and
the build choose independently. `BuildProgress` records the volume beside the package id, so
a page drawing volume 1 does not show volume 2's bar. **The sync is the exception, because its buttons
live in the sourcing panel, which knows nothing of volumes**: they `hx-include` a hidden
`#package-volume` that the volume strip carries, and the strip itself rides back out of band because a
sync is what lengthens it.

**Import reads the manifest's `volume` key.** `ensure_volume` adds the volume under the file's id, or
refuses one whose id disagrees, and `add_to_volume` places the songs into that volume whatever the
package's last volume is.

## Making it fast on a real corpus

Measured on the real corpus, on a spinning disk. The slow paths are all
whole-table reads, which is why the disk matters.

| | before | after |
|---|---|---|
| second open, to the banner | ~18 s cold | **0.14 s** |
| browse, first page | 0.24 s | **0.005 s** |
| browse, far down the list | 14.0 s | **0.010 s** |
| the A–Z bar | full scan | **0.002 s** |
| the folder page, at the root | 2.1 s | **0.034 s** |

**The browse order needs an index whose key is an expression.** `ORDER BY eff_title` is a `coalesce`
over three columns, which no ordinary index serves, so SQLite sorted every non-merged row into a temp
B-tree on every page. Worse at depth: with a sorter in the plan, `OFFSET` discards rows *after* the
result columns are computed, so the correlated subqueries in `browse_columns` ran for every skipped row
too. Partial expression indexes fix it, each `WHERE merged_into IS NULL`, which `Filter::to_sql` emits
first and always. With no sorter, `OFFSET` skips before the result columns. **No paging rewrite was
needed** — keyset paging and a two-stage join were both measured and both unnecessary.

They are created in `db.rs`, not `schema.sql`, and that is the point: SQLite uses an expression index
only when the query's expression matches the index's tree for tree, so both come from the one function
that spells the expression. A second copy in `schema.sql` would be free to drift, and drift would not
fail — it would silently stop the planner using the index.

**Six of the nine stopped being expression-keyed when the browse order learned to fold accents**, and
the reason was not performance. SQLite's default collation sorts every accented character after `Z` and
every capital before every lower-case letter, so `É o amor` sat past the end of a corpus-sized list
while filing correctly under `A` — a page nobody browsing by name reaches the bottom of. A fold has no
SQL spelling that is not a second copy of `km_song::text::fold`'s accent table, and two of those
disagreeing looks like bad data rather than like a bug. So the fold is stored: `songs.sort_title` and
`songs.sort_artist`, written by `Db::refold` from every path that writes a name, invalidated to NULL by
the `songs_refold_update` trigger when a path forgets, and backfilled once by `Db::backfill_sort_keys`
over the partial `songs_unfolded` index. **A change to the fold itself is the fourth way in**:
`Db::invalidate_folded_keys` compares `km_song::text::FOLD_REVISION` against the `fold_revision`
setting and, when they differ, blanks both columns and lets the backfill above do the work — the twin
of `language_tags_revision`, one column over. It writes no keys of its own, deliberately: there is no
reason for two things to know how to fold a whole corpus. Only the letter bucket (`title_initial`) and the language sort
(`eff_language`) are still expression-keyed, so the tree-for-tree fragility above now describes two
indexes rather than all of them.

Two consequences worth knowing. **An index whose key changes changes name with it**, and
`create_browse_indexes` drops any `songs_browse_*` its own list does not contain, because every `CREATE`
there is `IF NOT EXISTS` — which cannot see a key, so under one name an existing database would keep
the old index for ever. The name is load-bearing twice over: `missing_indexes` reports a renamed index
as absent, which is what asks for the banner and for the `ANALYZE` below, without which the planner
will not choose the replacement. And `title_initial` reads the folded column too, which retired this
crate's own accent table and makes the A–Z bar here the same alphabet as the offline remote's strip and
the printed book's sections: `¿Y ahora qué?` files under `Y` rather than under `#`.

**Every arm of `Filter::order_by` has an index whose key is that arm, term for term.** The "unrated
last" orders open with `x IS NULL`, which reads like something no index can carry; SQLite indexes
expressions, so naming it as the leading column serves the sort exactly as written. Mixed `ASC`/`DESC`
in the key is what lets `x IS NULL, x DESC` be one seek.

**Five of the arms end in the same four terms, and the six `*_artist` indexes carry them.**
`WITHIN_TITLE` is the order's spelling and `WITHIN_TITLE_KEY` the index key's, differing only by the
table alias a key has nowhere to put, and a test derives one from the other — the drift this guards
against costs the index and shows nothing on the page. What the terms are for is
`Within one title, by performer` in [`../decisions/songs.md`](../decisions/songs.md); what they cost is
two columns on six corpus-sized B-trees, paid on every written row, against a title order that
scattered a performer's recordings through everybody else's.

**`ANALYZE` runs unbounded, and `PRAGMA analysis_limit` is what made it look otherwise.** An expression
index is invisible to SQLite's built-in guesses, so without statistics the planner takes
`merged_into IS NULL` for the selective term. The obvious defense — bound the sampling so `ANALYZE`
stays cheap — *causes* the bad plan: at `analysis_limit = 400` a column that is NULL on every row is
recorded as 401 rows per distinct value, so the planner believes the seek returns 401 rows and sorting
them is free. A bare `PRAGMA optimize` has the same flaw. So the bound goes, and what it costs is paid
knowingly: warm, a full `ANALYZE` is about a second, and cold at the end of a corpus scan it is
minutes — see [`The tail, and what a hold of the writer costs`](#the-tail-and-what-a-hold-of-the-writer-costs).

**The folder tree is a table.** Computing it on each visit would be a `substr`/`instr` group-by over
`files`, whose grouping key is an expression, so no index could help and the root would read every
row. `beneath` cannot be a sum of children, because the count is of distinct *songs* and a corpus
files one recording in several folders; `rebuild_folders` walks `files` ordered by `song_id` and
tallies once per song, so the set held in memory is one song's ancestors rather than one folder's
songs.

**Derived tables must not answer for a corpus that has moved on.** A scan rebuilds the tree as its
last pass, including a *stopped* scan: that is the one tail pass a partial read may run, because it
describes the rows that were written rather than concluding anything about the corpus. As a backstop
the Folders page compares a cheap marker with the one stored at the last rebuild, and rebuilds when
they differ. **The staleness check
has to be cheaper than the query it replaces**: `COUNT(*) FROM files` reads every row and would add a
tenth of a second to every visit. `MAX(rowid)` is an index seek to the end.

**`songs.file_count` is a denormalized column maintained by three triggers on `files`** — insert,
delete, and `UPDATE OF song_id`. Triggers rather than the scan, because the scan is not the only writer
of `files`: a merge, a delete and a re-point all move rows. It is `NOT NULL DEFAULT 0`, which it has to
be — `NULL + 1` is NULL, so a nullable count would go silently blank on the first insert.

`songs_unfolded` is a partial index on exactly the sort-key fold's predicate, rather than a `settings`
flag, because a flag is a claim about the data kept outside it: empty on a folded database, so the
probe touches one page. An index cannot be wrong; it *is* the predicate.

**`songs.updated_at` is a stamp kept by a trigger, for `file_count`'s reason and for one more.** Nine
separate statements write a hand-set column of `songs`, so a stamp kept in Rust is a stamp that goes
wrong the first time somebody adds a tenth. The extra reason is the trigger's `UPDATE OF` list, which is
exactly `backup::HAND_SET_COLUMNS`: a bare `AFTER UPDATE` would stamp a whole corpus as edited on its
next rescan, because `write_scanned`'s `ON CONFLICT` rewrites the detected half of every row it
revisits. A `WHEN` comparing old against new is what makes the stamp mean *changed* rather than
*written* — the filter-wide language set writes every matching row whether or not it already holds that
language, and `unmerge` clears a column without asking whether it was set. `strftime` in SQL rather
than a value bound from Rust, because a trigger body takes no parameter; `'now'` is fixed for one step
of a statement, so a bulk edit lands one time on every row it changes. The sort it serves costs
`songs_browse_updated_artist`, whose entries on a corpus nobody has curated yet are
`(1, NULL, <the title terms>)` — a copy of `songs_browse_title_artist` behind a constant pair, so close to
another corpus-sized B-tree. The column is text in the shape every timestamp here uses, fixed width to
the second, which is what lets that index be a plain one rather than an expression.

**The added-date sort and filter read `songs.first_seen`**, which the scan's upsert writes on insert and
leaves out of its `ON CONFLICT` update. `songs_browse_added_artist` keys on `first_seen DESC` and then
the title terms. Its leading column also serves the filter's range when the list is sorted by it. Under
another sort the range is a residual predicate, like `kind`. The filter's bound is
`strftime(..., 'now', '-N days')` in SQL, in the stamp's own shape, so the comparison is text.

**The per-page `COUNT(*)` was never the right question.** It counted the filtered corpus on every page
turn to decide whether to draw a *next* button, when the answer is one row past the `LIMIT`.
`Db::songs_page` asks for `limit + 1` and reports whether it got it. The real total is still wanted for
the label and the five-page jump, but that is once per filter, carried in the paging links; `without()`
drops it, because a different filter matches a different number.

**The status bar's aggregates are cached against two of SQLite's own counters**, not a generation
number bumped by each of twenty-odd write paths, which works until somebody adds the twenty-first.
`sqlite3_total_changes` counts rows written on the connection asking: a read cannot move it and a
write cannot fail to, triggers included. It says nothing about any *other* connection, and a page is
drawn through one that never writes — so keyed on that alone the bar would show whatever was true
when the folder was opened. `PRAGMA data_version` is the complement: it moves when another connection
commits and stands still for this one's own writes. Either moving means recount.

**The red badge is a partial index**, because its question is `scan_status <> 'ok'` and an inequality
has no range in `files_status` to seek — so it walked every row of `files`, on every page of the tool,
for a number that is almost always zero. `files_failed` holds only the rows the question is about, and
is empty on a corpus that reads cleanly. It is named in `HEAVY_INDEXES` for that list's second job
rather than the banner: an index with no `sqlite_stat1` row is one the planner will not choose.

**The Songs tab carries the total it counted.** The remembered filter is what a bare `/songs` is
redirected to and what the nav link points at, so leaving the total out of it made every arrival at
the tab re-count the filtered corpus to label a page the count does not decide. It is the same reuse
the paging links make, and `FilterQuery::total` is trusted for the label and the five-page jump and
for nothing that decides what is on the page.

`db::tune` sets a 1 GiB `mmap_size` and `temp_store = MEMORY`, and a page cache sized for what the
connection is for — large for the writer, whose rebuild and whose scan batch walk indexes that fit in
it, and a quarter of that for a reader answering one page at a time, because `cache_size` is per
connection. All best-effort: an in-memory database and one on a network share are both real here.
`synchronous = NORMAL` is set only where the WAL switch took, since it is crash-safe under WAL and not
under a rollback journal.

### Two connections, and which one a page is drawn through

**One connection cannot both commit a scan batch and draw a page.** The writer holds it for the length
of a batch — hundreds of scattered rows against the disk the readers are also on — and a page that
needed the same connection waited for a gap between them. `std::sync::Mutex` promises no fairness, so
over a whole corpus that wait had nothing bounding it: the tool stopped answering, a browser's six
sockets to one host filled with requests that would never come back, and `GET /scan/progress` went on
reporting a healthy phase because it reads atomics and touches no database at all.

So an open folder holds two connections. `Db::open_reading` is `SQLITE_OPEN_READ_ONLY`, runs no
migration and no backfill — the writer has brought the schema to where it belongs before
`Workspace::new` opens this one — and `State::reading` is the door to it. `State::blocking` keeps the
writer.

**Write-ahead logging is what makes the pair safe**, and a database that did not take it gets no
second connection: the other journal modes are the ones where a writer excludes readers outright, so a
page there would wait out `busy_timeout` and answer *database is locked* — answered wrongly rather
than answered late. `Workspace::reader` hands back the writing connection in that case, which is also
how an in-memory database works at all, since `:memory:` opened twice is two empty databases. The
reading connection is released before the closing checkpoint, because `wal_checkpoint(TRUNCATE)`
cannot reset the log past a reader.

**Which door a handler takes is decided by what its closure does, not by the method it answers.** Only
seven methods take `&mut self`, so the signature catches the obvious half and the open flag catches
the rest — SQLite refuses a write on the reading connection outright, which is what
`a_reading_connection_refuses_a_write` pins. The Folders page is the one hybrid: reading the tree is a
read, refreshing it is a whole pass over `files` that writes, and a scan moves the marker it is
checked against on every batch. So the page refreshes the tree through the writing connection when
the marker is stale, and not while a scan is running, which the scan's own tail covers.

**A write gets in between batches, because the scan stands aside for it.** Releasing the connection
is not enough: `std::sync::Mutex` makes no fairness promise, so the writer unlocking and immediately
locking again for the next batch does not hand it over, and a write could be passed over for the
length of a run. So the mutex and a count of who is waiting for it live together in `db::Shared` — a
request counts itself in while it waits, and the scan asks between batches and between the tail's
chunks whether anybody is there. That costs a fraction of a batch, and only when somebody is
actually curating: pages do not come through this connection at all, so browsing does not light it.

**And a write that still cannot have it says so.** `State::blocking` gives it five seconds, the same
number `busy_timeout` uses, and then answers `DbError::Busy` as a 503 with a worded sentence. Through
the reading phase that deadline is a backstop, because the stand-off lets a write in at the next
batch boundary; **through the tail's two whole-corpus passes it is the ordinary answer**, since
neither can be released part-way. A request that never answers is what this whole arrangement exists
to remove, and a write is not exempt from it.

The condition that would earn a pool of readers is a page waiting on another page rather than on the
disk. One reader serializes renders against each other, which is one person clicking; the requests a
browser fires in parallel are the embedded assets, which touch no database.

**Numbers taken from this tool while anything else is building are not numbers about this tool.** The
symptom that prompted all of the above was an 80-second browse page whose dominant cause was four
tenants on one spindle; the same query was 0.15 s warm.

## Reaching the machine

`reqwest` with `default-features = false` — no TLS, because the only server this calls is the karaoke
app over plain HTTP on loopback or a home LAN. `debug/play-file` to hear a candidate on this box,
`debug/play-upload` to hear one on a machine that cannot see this disk, `admin/packages` and
`admin/packages/upload` to install a built package the same two ways, `discover` to say whether the
machine is there, and `admin/login` and `admin/debug` for the two things that need a token.

`multipart` and `stream` are on for the upload alone, and both were checked against that no-TLS
constraint before being added: `multipart` is `mime_guess` + `futures-util`, `stream` is `tokio/fs` +
`tokio-util`, and neither reaches any `*-tls` feature. `Part::file` needs `stream`, and it is what
puts a real `Content-Length` on the request so the machine can refuse an oversized one up front.

**Which call the Play button makes is decided by the address**, in `Client::is_loopback`: a path for
a machine on this box, the bytes for anything else. The whole of `127.0.0.0/8` and `::1` count, and
so does the name `localhost` with or without its trailing dot — but not `localhost.example.com` or
`127.0.0.1.nip.io`, which a `contains` would have got wrong both times. An address that will not
parse counts as remote, which is the safe direction of the two.

**The upload asks before it sends**, using `accepts_uploads` on the `discover` call the Settings page
already makes. Not an optimization: a machine that refuses mid-request and closes leaves `reqwest`
reporting a dropped connection rather than the 400, so the refusal below would arrive as "the karaoke
app is not answering". Found as a flaky test, which is what that looks like with a four-byte fixture.

**The first test-play on any machine is refused whichever route it takes, and the tool has to say
why.** The machine will not play a path outside `settings.debug.play_file_roots`, and will not take
an upload unless `settings.debug.enabled` is on; both are off in a shipped configuration, so
the answer is a 400 naming a setting. Each refusal is recognized and answered with the JSON to add
and where the file is, and both spell it **nested**, which is the trap they exist to avoid:
`debug.play_file_roots` is the setting's *name*, but a key spelled that way is ignored, because the
key is `play_file_roots` inside a `debug` object. `debug.enabled` is the same shape.

The path refusal names **the curated root**, not the clicked file's parent — one entry at the root
permits everything the tool can offer, where a leaf folder three levels down would be refused again
by the very next song. The upload refusal names no folder at all, which is the difference between the
two routes: there is no path to permit, because the song is sent rather than found.

Paths are tidied before they are shown, and before a file is opened to be sent: `canonicalize` on
Windows returns `\\?\C:\…`, which is valid and which nobody recognizes in a settings file.

**An MP3+G pair is found here rather than over there.** `km_kmpkg::pair_for` runs against this box's
disk — where the corpus is — and both halves go in one request, staged under one `stem` field so the
machine names them consistently. That is what makes the machine's own `pair_for` hit on its first
try instead of relying on the tolerance it has for a corpus that is not tidy.

**The token is on `State`, not on the `Client`.** Both install routes are admin, so the tool needs
one — and `app_client` builds a fresh client per request, because which address to use is a read of
the database and of the network. A token owned by the client would therefore be dropped between the
Settings form that obtained it and the Install button that needs it. `Client::sharing_token` hands
every client of a run the one `Arc<Mutex<Option<String>>>`. Choosing a machine clears it, because a
token is one machine's; following a machine that has *moved* does not, because that is still the same
machine.

`src/passwords.rs` is the other half and is the only thing here that writes a credential: a
`BTreeMap` of machine id to password in the per-user config folder, written only when the checkbox is
ticked, `0600` on unix, deleted rather than emptied when the last entry goes, and `None` under
`cfg(test)` and under an empty `KM_PACKAGE_BUILDER_PASSWORDS` for `recent`'s reasons. A remembered
password is spent lazily, in `State::sign_in_if_remembered`, at the moment a token is wanted.

**Discovery lists, and never sets.** This tool installs packages, so a version of it that re-pointed
itself at whatever answered a browse first would eventually write to the wrong machine. `adopts:
false` is the narrower half of that guarantee: `choose` refuses a different machine to any device that
already knows one, and the flag covers what is left, a workspace that has been told about none.

**It does follow the machine already chosen to a new address**, which is that same choice rather
than a new one -- and the policy is `known::choose` with `adopts: false`, not a follow of this
crate's own. `src/chosen.rs` keeps the record: one `Known` in the workspace, whose identity is
written only from a `/discover` that answered at the address in force, and which is replaced without
one whenever somebody sets an address by hand. It comes back from the one `blocking` call
`app_client` was already making, so the case where nothing has moved costs what it always did.

**And the browse does not wait.** Waiting the whole three seconds buys a list that shows both
machines in a house with two; a `Watcher` started after the bind has been listening since the tool
opened and has heard from both already, so the Discover button is instant and more complete than the
wait would make it. Started after the bind rather than in `State::empty`, because a great many tests
build one of those and opening a multicast socket in a test is what `CONTRIBUTING.md` forbids.

## Two platform traps worth carrying

**There is no `argv` on Windows.** `Command` flattens its arguments into a single string and quotes one
only when it is empty or holds a space or a tab; `cmd.exe` then re-parses that string with rules of its
own, in which `&` separates commands. A percent-encoded URL has no space anywhere, so it crossed bare
and `cmd` cut it at the first `&`. The target must be quoted for **`cmd`'s** parser and written verbatim
with `raw_arg` — Rust's own quoting is for MSVCRT. A test on `Command::get_args()` cannot see any of
this; the assertion has to be on the command line `cmd` will parse.

**`target="_blank"` does not leave a wry webview** — the click raises a new-window request and, with
nothing answering it, does nothing at all. `with_new_window_req_handler` answering `Deny` and handing
the address to `osopen` is the fix, with the scheme checked first, because what arrives is whatever the
page asked for and it ends up as an argument to `cmd /c start`. The open is on a spawned thread, since
the handler runs on the platform's event thread. An address on the tool's own server is the exception:
the handler sends it to the event loop as `Wake::Navigate`, and the loop loads it into the webview,
which does not exist yet when the handler is built.

## Not built, deliberately

No authentication, no editing of MIDI content, and no `.st3` support — the last would settle an open
question by accident.
