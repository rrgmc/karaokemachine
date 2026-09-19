# The curation tool

> Part of the [architecture notes](../ARCHITECTURE.md). Product decisions live in
> [`docs/decisions/`](../decisions/); this file says how the thing is built.

`tools/cmd/km-package-builder` is a local web server over a folder of source karaoke files. It
browses, rates, corrects and groups them into packages that `km-pack` builds. It binds loopback by
default. `--lan` opens it up and warns, because the tool itself has no password.

The tool does hold the
machine's password, from a box on the Settings page, once somebody has typed it. The token that
password buys lives in memory for the run. The password reaches a disk only if the checkbox beside
the box is ticked. It then goes into `machine-passwords.json` in this tool's per-user config folder,
not into the curated folder. See `The password is remembered per machine, or not at all` in
[`curation.md`](../decisions/curation.md).

`--init` is the only thing on the command line that creates a database. Pointing it at the wrong
folder is therefore an error rather than an empty index. With no folder, the tool starts on an Open
page with recent folders, a browser and a path box. That page is the other thing that may create a
database.

## The database is the document

`km-package-builder.kmbuild` is ordinary SQLite that lives in the curated folder. Its name is one the
operating system can hand to one program. Opening looks for whatever single `*.kmbuild` a folder
holds rather than a fixed name, so a corpus can be called `Brasil.kmbuild`.

Two tables carry the model:

- **`files`** — one row per file on disk. The path is relative to the root with forward slashes, so
  the folder can move. `(size, mtime)` is what makes a re-scan nearly free.
- **`songs`** — one row per distinct recording, **keyed by the content hash of its bytes**. That one
  choice gives both halves of the brief. The same file always yields the same id. Identical copies
  land on one row, with no grouping pass to get wrong. `files` carries the one-to-many.

The other tables are these:

- `favorites` and `song_favorites`: the named lists, and what is in them.
- `folders`: the materialised folder tree.
- `packages` and `package_volumes`: what a curator names, and each `.kmpkg` it is written as.
- `package_songs`: recording *which file* each entry came from.
- `saved_filters`: a name, and the query string it stands for.
- `settings`, and two FTS5 tables.

`Db` is one type with one `rusqlite::Connection` and no pool. Its ~100 methods are **twelve files
under `db/`, not twelve types**. `open`, `songs`, `favorites`, `packages`, `saved`, `backup` and `scan`
are `impl Db` blocks, split by the question being asked. `migrate`, `sql`, `filter`, `model` and
`tests` are what they all speak in. The seams are the `// -- packages ---` banners that the single
file carried, so the split moved code and changed no logic. Nothing outside `db` sees the split,
because the module root re-exports every type flat.

### Three kinds of name, and why they cannot merge

`songs` keeps detection and correction in **separate columns**. A person never writes `det_title`,
and the scanner never writes `title`. That lets a re-scan refresh the analysis without touching a
correction. It is the same distinction `SongEntry::edited` makes inside a package. NULL in a
hand-set column means *nobody has said*, which is not empty.

**`artist` is where that distinction is load-bearing rather than tidy.** `eff_title` folds NULL and
`''` together with `nullif`. `eff_artist` has no `nullif`, so an empty `artist` stands in front of
`det_artist`, where a NULL one falls back to it. That lets `set_names_from_stem` clear the performer
a sequencer left behind. It is also why the two sort into different ends of the browse list.

`songs.stem` is the third: the file's own name. The scanner writes it through the same
`km_pack::file_stem` that `km-pack build` uses, so the two cannot disagree about what a nameless song
is called. It exists because a great many corpus files carry no metadata at all.
`coalesce(title, det_title, '')` renders those as an invisible unclickable link, sorted to the front.
It is deliberately **not** `det_title`. That column answers *what did the file say?*, and a filename
written there would be a lie that a re-scan would preserve.

The fallback had to be a column. A template default, a row-mapper default or a SQL expression over
the joined path would not do. Only a column fixes display, `ORDER BY` and search at once. Only a
column is visible to the FTS triggers, which fire on `songs` and cannot see `files`. `eff_title()`
in `db.rs` is the one expression every query builds from, and `SongDetail::effective_title` is its
Rust twin.

`det_language_tag` is a second *detected* column. It holds the ISO 639-1 code that `det_language` and
`det_encoding` imply, while `det_language` goes on holding the file's literal `ENGL`.
`det_language_guess` is a third, holding what `km_langguess` made of the song's own words.
`det_language_guess_confidence` sits beside it. `eff_language()` coalesces the three in order of what
stands behind each. The scanner writes the guess only above `km_langguess::MIN_CONFIDENCE`.

`Db::backfill_language_guess` fills that column on rows a scan wrote earlier. It reads only text the
database already holds. It works in chunks by the primary key, where `backfill_language_tags` reads
one `Vec`, because a chunk here holds lyric tracks rather than two folded names. It stores a cursor
in `language_guess_cursor`, so an interrupted open resumes. At corpus size the detector alone takes
around 36 seconds, measured at 60µs against a lyric track of a kilobyte.

**Adding a leg to `eff_language()` changes an index key**, since `songs_browse_language_artist` is an
expression index over it. `create_browse_indexes` therefore compares the stored `CREATE` statement
rather than the name. An index whose body moved keeps its name. `CREATE INDEX IF NOT EXISTS` would
then leave the old key in place, and the browse page would silently match no index at all.

### The backup is that split, read out to a file

`backup.rs` writes the hand-set half of `songs`, plus the favorites and their membership, and
nothing else. The derived half is what a re-scan produces, and it would be a hundred megabytes of
nulls on a real corpus. `Db::hand_set_songs` is the whole of the read. It builds its `WHERE` from
`HAND_SET_COLUMNS`, so the predicate and the document cannot name different columns. It also carries
an `id IN (SELECT song_id FROM song_favorites)` disjunct. A song somebody has only filed carries
nothing in its own row, and that disjunct keeps it in the file.

The backup translates two identities, because a rowid means nothing in another database. Songs use
`songs.id`, the content hash, which needs no translation. That is why the whole thing works across a
rebuild. A favorite becomes its name, which `favorites_name` makes unique. The name needs no escaping
rule of its own: `Rock/Pop` is a name somebody types, and it travels as one. A document written when
favorites nested names one by the array of its ancestors.

`FavoriteRef` joins such an array with ` / `, the separator those pages drew. The list a person was
looking at is therefore the list they get back.

Restoring is two phases. `backup::plan` reads and checks and writes nothing. Every refusal therefore
costs a line in a report and no recovery. It refuses a language `Language::parse` will not take and a
rating outside 0–10. It also refuses a transposition that will not fit `i8`, and a merge that would
chain.

`Db::apply_restore` then writes the checked plan in one transaction. It prepares its own statements
rather than calling the per-field setters, because a `rusqlite::Transaction` holds `&mut self.conn`.
`self.edit_song(..)` would not borrow, and `self.add_to_package(..)` would issue a nested `BEGIN` that
SQLite refuses.

`write_scanned` and `forget_missing` use the same arrangement, for the same reason. The
fill-blanks/overwrite policy is a bound flag inside one `CASE`-per-column `UPDATE`. Both directions
are therefore one statement. Neither can write NULL, since `coalesce` with the column itself is the
whole rule.

`default_path` names the file after the tool and after the moment. `backup::newest` reads the data
folder for the most recent one, so the restore box can suggest it. Both rely on a sortable stamp, and
on the stem being the only part that moves. `SUFFIX` is what the scan skips by, and what
`db::database_in` must not mistake for a second database. The stamp itself is `scan::timestamp` with
its separators removed, not a second clock. That keeps `written_at` and the file name talking about
the same instant.

The field list is the one thing here that is written down rather than derived. A hand-written column
list is how corrections get lost. It cannot be derived, because a JSON key is not a column. A test
therefore partitions `pragma_table_info('songs')` into `HAND_SET_COLUMNS` and `DERIVED_COLUMNS`. It
fails on any column in neither, and names that column.

### Search is two indexes, not one

`songs_fts` covers the effective title and artist. It copies `remove_diacritics 2` from `km-catalog`,
because the corpus is largely Portuguese and `coracao` must find `coração`. It cannot be an
external-content table. The content would have to be the expression `coalesce(title, det_title)`,
which FTS5 does not allow. It is therefore a plain table, and triggers keep it in step.

**That difference matters.** `km-catalog` uses the form
`INSERT INTO songs_fts(songs_fts, …) VALUES('delete', …)`. That form is valid only on external-content
and contentless tables. Against a plain one it fails at run time with nothing but `SQL logic error`.

Both `_update` triggers are `AFTER UPDATE OF <the columns the statement reads>` rather than bare
`AFTER UPDATE`. That is a correction, not a refinement. Setting one language over a filter writes
every matching row. For each of those, a bare `AFTER UPDATE` would delete and reinsert three rows: a
title, an artist and a lyric. It would arrive at the text already in them. The rating setter pays that on every
click.

`lyrics_fts` is a **second table, not a third column**, and this is load-bearing. An FTS5 `MATCH`
with no column filter searches every column. Lyrics beside title and artist would therefore make the
browse page's "title or artist" box a lyric search. Type `love`, and half the corpus comes back.
Two questions on two pages are two indexes.

Its insert is conditional (`WHERE new.lyrics IS NOT NULL AND new.lyrics <> ''`), and its delete is
unconditional. Most of a real corpus is instrumental. The asymmetric pair also makes a song whose
lyrics are cleared leave the index rather than linger.

**Triggers are dropped and recreated on every open**, never `IF NOT EXISTS`. A trigger is code, and a
database made by an older version must not keep a broken one.

`lyrics_vocab` is an `fts5vocab` view over `lyrics_fts`, so it costs no storage, no trigger and no
write. Asking it how many songs hold a word is a seek into a b-tree the index maintains anyway. It
is `IF NOT EXISTS` like the two tables and unlike the triggers. A view over an index is not code
that can go stale.

### Asking the lyric index for a whole song

The same-words search compares a song's whole lyric with another's. Its two halves divide the way the
similar-names search's do: the index gathers candidates, and Rust scores them. What differs is the
question put to the index. The three things that shape it are all properties of FTS5.

**A phrase, not a bag of words.** A song's words ORed one at a time match most of the corpus. That is
the reason `lyrics_fts` exists as its own table. A run of three folded words is specific enough that
the files matching it are nearly all worth scoring. This works because `km_song::text::fold` is
deliberately the same folding as `unicode61 remove_diacritics 2`. `fold_and_the_search_index_agree`
in `km-catalog` pins that, so a folded word is a token the index holds.

**The phrases are chosen for the rarity of their rarest word**, which is what `lyrics_vocab` answers.
A phrase is only as selective as its rarest word. `ORDER BY bm25` ranks every row a query matches
before any `LIMIT` cuts one. A dozen phrases of whatever words a song opened with would make SQLite
score a large part of the corpus to return a hundred rows. The phrases spread across the song first,
and rarity chooses within each stretch. A file missing its opening therefore still matches the rest.

**A phrase may not cross a line the banner rule dropped.** The index holds the whole `lyrics` column,
the sequencer's credits included. The word before a dropped line and the word after it are therefore
not adjacent there, however adjacent they are in the word list. A phrase built across that seam asks
for something no document holds, and it silently matches nothing: the worst shape a bug can take.

That is why `dupes::sung_runs` returns runs of consecutive kept lines, where `dupes::sung_words`
returns one flat list. The scoring wants the flat one, because one sequencer wraps its lines where
the next does not.

**This is the one place reading lyric text by the hundred is affordable.** `Db::fingerprints` refuses
to hold a lyric, because it reads every row of `songs`. This search reads the candidates it was given,
and drops each after scoring it.

Measured over a real corpus by `db::measure::where_the_same_words_threshold_sits`, mapped:

| what | figure |
|---|---|
| one search, median | 69 ms |
| one search, 95th | 125 ms |
| one search, worst of 50 | 198 ms |
| the phrases returned the other file, of 200 pairs known to be one recording | 81.5% |

**Recall is the figure the design stands on, and it is not the threshold.** A pair the phrases never
return is invisible, whatever the score would have been. The measurement therefore reports recall
separately, and the answer to a low figure is more `PROBES` or a shorter `SHINGLE`. At 81.5% it sits
about where the scoring ceiling is: a fifth of those pairs share no three-word run at all. The
phrases are not what is losing them.

**Recall is measured with every version asked for, and it is wrong without that.** The pairs it reads
as ground truth are the ones the duplicate pass grouped, so one of each hides behind the other. The
default filter collapses those in SQL, before a phrase is asked for anything. Measured with the
default, recall reads 17.5% and is reporting on the filter.

The two helpers are written separately rather than one in terms of the other. The reason is cost
rather than clarity. A pass over the whole corpus reads `sung_words` once per song, so it folds the
lyric once. `sung_runs` folds a line at a time, and a page reads it once.

`db::filter::fts_match_query` is what both boxes type through. It emits three shapes and no others: a
quoted token, a quoted run of tokens (an FTS5 phrase), and either with a trailing `*`. It reads a `"`
in the input as the mark that opens or closes a phrase, and never passes it through. Every `"` in the
expression is therefore one the function wrote, and that keeps `OR`, `*` and an apostrophe literal.
`a_phrase_reaches_sqlite_as_a_phrase` runs all three through the real index. A string that looks like
a phrase query is not evidence that SQLite reads it as one.

## A swappable folder

`State` holds `Arc<RwLock<Option<Arc<Workspace>>>>`, where `Workspace` is the database, the root and
the scan handles. **The inner `Arc` is the load-bearing part.** Every accessor clones it out and drops
the read guard immediately, so no request holds the lock across its SQLite work. If a request held
it, a swap would queue behind a multi-minute migration. The polling pages would then starve the
writer, because `std::sync::RwLock` promises no writer preference.

The scan and build handles live in `Workspace`, and `Db::close` is in its `Drop`. A scan is a detached
thread that holds a clone of the connection for minutes. Left on shared state, it would go on writing
into the *previous* corpus after a swap. Nothing would show until rows appeared in a folder nobody had
open.

**The same call empties the songs filter.** The filter is one string on `State` and reaches no file,
so it lives exactly as long as the folder it names. `publish` clears it before it takes the write
guard. `close_folder` clears it on the way out, where `crate::finish` and the event loop's
`LoopDestroyed` both arrive. Coming back to a morning's work is what a *saved* filter is for.

**What the remembered filter is worth depends on one route reading it, and `GET /songs` is that
route.** Every render of the songs page writes down the filter that arrived. That is how the Folders
and Favorites pages hand it one the bar never set. A render that arrives with an empty query string
therefore spends what was remembered, and the nav's seven bare hrefs are where that happens.

The handler therefore takes `RawQuery` beside its `Query<FilterQuery>`. It answers `None` with a redirect
to the remembered filter. `Some("")`, the *clear all* link, renders the corpus and clears the record.
`rows_for` clamps a page above the end back to the last one before it writes the record, so the
record holds the page being shown.

**The folder browser's cost is one directory read per row, so the page comes before the probe.**
`browse::indexed` opens the folder it is asked about: a badge saying *indexed* means a `read_dir` of
that subdirectory. `browse::list` therefore gathers names, narrows, sorts and cuts to a page *before*
it asks the question of anything. The order is the load-bearing part, and it reads as arbitrary.

Probing inside the gathering loop is the obvious shape, and it made a folder of three thousand
subdirectories a minute of disk to list.

`GET /open/list` is also the one route here that cannot use `State::blocking`. That wants a workspace,
and the picker is the page reached without one. The route spawns its own blocking task. Running on
the executor thread is a fault that hides behind a listing that is fast on every folder somebody
tested.

`?rows=1` answers with the folders and their pager alone. The filter box has to sit outside what it
replaces. Otherwise its own result destroys it four hundred milliseconds after a keystroke. `#filters`
and `#rows` already have that arrangement on the browse page.

Handlers know nothing about the `Option`: a middleware redirects anything needing a folder to `/open`.
Two details there are right only because somebody checked them:

- **`HX-Redirect`, not a 302.** A browser follows a 302 transparently, so htmx would paint the whole
  Open page inside whatever `<div>` the button aimed at.
- **`.layer`, not `.route_layer`**, so the middleware catches a path that matches nothing too.

**Opening is a job, not a request.** `Db::prepare` runs migrations, creates browse indexes and runs
backfills. It finishes with an unbounded `ANALYZE`, which takes minutes on a real corpus. None of it
may happen before the socket is listening. A browser pointed at a tool that has not bound gets a
refusal, and the tool reads as broken. So `begin_open` spawns a thread, and the page polls
`/open/progress`.

**`Phase` reaches every stretch of that, `migrate` included.** It is `&dyn Fn(&str)`, threaded through
`Db::prepare` as an argument rather than a field, because the ladder runs before a `Db` exists.
`begin_open` is the only caller with anywhere to put the answer, and everything else passes
`&|_| {}`. The sentences come out of `indexing_phase`, `folding_phase` and `rebuilding_phase` in
`db.rs`, one function each. The console banner and the page therefore cannot describe the same work
differently.

Two of the sentences appear on the console only above `announce`'s size threshold, and
on the page always. A terminal has something else to show, and the page has only the sentence it
already holds.

`Opening` carries a `started: Instant` and `OpeningView` an `elapsed_secs`. That is what moves
through the one step long enough to matter.

**Beside it `Opening` carries a `step::Ladder`, which is the Scan page's checklist under a name that
is not the scan's.** Its rungs are `db::OPENING_LADDER`, eleven catalog keys in the order an open
climbs them. `set_phase` moves it with `Ladder::advance_to` rather than `Ladder::say`. An open names
only the rungs it takes, so climbing past a rung nothing reported marks it skipped.

`advance_to` leaves a rung already running where it is. `Folding` and `ReadingWords` report on every
chunk they write. Restarting the clock at each would draw a step that has run for minutes as one
that has just begun. `Opening::finish` closes the list inside the lock that already decides which
caller wins. The rung a failed open stopped at therefore survives the `Drop` guard behind it.

The panel's bar is `.bar.working`, except while a rung that counts is running. That bar is full
width. The rule gives it its resting opacity, not the keyframe's 0%. A tab the browser has stopped
painting therefore shows a bar rather than a faded-out one. `OpeningPhase::counted` answers for the two rungs
that count, and `OpeningView::percent` is `Some` only there.

**The schema has a version, and its first job is to refuse.** The tool stamps
`PRAGMA user_version` with `SCHEMA_VERSION`. It turns away a database outside
`OLDEST_SCHEMA_VERSION` to `SCHEMA_VERSION`, with a sentence naming both numbers. That sentence is
the most likely thing a double-click ever produces. A corpus opened by a development build and then
double-clicked into an installed one is a schema ahead. Where the sentence goes therefore matters as
much as what it says: the Open page, in red, with the chooser beside it.

SQLite hands back rows from a table with columns this build has never heard of. Without the number,
a `.kmbuild` written by a newer build would open *silently*. The tool would then curate a corpus
while ignoring whatever that build had added.

**The ladder is `step_to`, one arm per version above the floor.** A schema change adds the arm for
the next number and bumps the constant in the same change. A current database answers in one
`PRAGMA`.

**An unstamped file is either new or refused.** A brand-new database reaches `migrate` at 0 with no
tables at all. `Db::prepare` checks the version before the schema batch runs, so the triggers the
batch recreates find every column they name. `migrate` stamps the new database current there and
then, so no step ever runs against a database with no tables. An unstamped file that *has* a `songs`
table carries no number this build can place, and the tool refuses it.

**A step that changes a key defers the foreign keys rather than ordering its writes.** A parent key
that changes under existing children is a violation, whichever table the step writes first.
`foreign_keys` is ON for the life of the connection. `PRAGMA defer_foreign_keys` holds the check to
the commit, and by then both sides agree. `foreign_keys` itself is a no-op inside a transaction, so it
cannot do this job. The step collects the rows to change before it writes anything, because they are
the rows it reads.

**Ending a job is a `Drop`.** Setting `finished` only on the worker's success paths would make a panic
anywhere in the open permanent. The slot would stay in flight for the life of the process. Every later
open would answer *already opening…*, and only a restart would clear it, in a window with no console.
`EndsTheJob` ends the job however the thread leaves. It writes the reason *before* `finished`, so a
page that sees a finished job sees why.

`busy_timeout` is five seconds. SQLite's default is zero, so any contention is an immediate
`SQLITE_BUSY`. That is survivable only while a corpus can have exactly one curator. A database that
can be double-clicked can be double-clicked twice.

## The window, and the two executables

The window sits behind the `desktop` feature. It is off in cargo, on when staging for Windows and
macOS, and never on for Linux. `tao` owns the window, and `wry` puts a platform webview in it pointed
at loopback, so every handler, template and route is unchanged. It is a *viewer*, not a second front
end.

`main` is not `#[tokio::main]`. `tao`'s `run` must own the main thread, never returns, and calls
`process::exit` on the way out. `main` therefore builds the runtime by hand and spawns the server
rather than awaiting it, and shutdown happens in `Event::LoopDestroyed`. A failed webview falls back
to the browser rather than to nothing. WebView2 is absent on Server and some LTSC builds, and a
version check would go stale.

**On Windows the crate is a library with two three-line binaries on it.** `km-package-builder.exe` is
GUI-subsystem. `km-package-builder-console.exe` is the same library with a console. It is the one to
type, and the one that answers `--help`.

Only the subsystem stops Windows from creating a console at all. `FreeConsole` can only make an
already-created window vanish again. The subsystem is a property of a *binary crate root*.

Four traps here, each of which is invisible from a terminal:

- **`FreeConsole` would make every `println!` panic.** `std::io::_print` panics rather than failing
  when stdout is gone. A double-clicked build would therefore abort every time, and never once in a
  test from a shell. `km-console` is the answer. It prints when there is a console and logs when there
  is not, and it decides once, before the first line. It is the workspace's only `unsafe`.
- **`GetConsoleProcessList` answering zero is not the same as having nowhere to print.** A
  GUI-subsystem process has no console. A process inherits standard handles whatever the subsystem,
  though, so `--version | cat` writes down a real pipe, and the staging scripts rely on exactly that.
  Zero asks a second question through the safe `AsRawHandle`: whether stdout's handle is null or
  invalid.
- **`cmd /c start` gets a console of its own** when its parent has none. That is every browser open
  and every Play click from a windowed build. Hence `CREATE_NO_WINDOW` in `osopen`.
- **An error returned out of `start` reaches nothing.** `main` returning `Err` prints through std's
  `Termination`, which writes to a standard error handle a windowed run does not have. A returned
  error also reaches no log, so `--log-file` would capture least well the one run it exists for. The
  tool therefore logs the error first and then shows it: a corpus that will not open goes to the Open
  page instead. A failure earlier than a server opens a small `tao` window with the
  sentence in it. An address already taken is one, and a path naming neither a folder nor a
  `.kmbuild` is another.

`failure_html` lives in `lib.rs` rather than beside the window. `desktop` is a feature the test
commands never turn on, and the escaping is what needs asserting.

The split between the two is *where a refusal would be read*, not *whether there is a window*. A
process inherits standard handles whatever the subsystem. A folder named at a prompt therefore keeps
the eager open and the exit status a script reads. See
[`A corpus that will not open is a page`](../decisions/curation.md#a-corpus-that-will-not-open-is-a-page-and-a-failure-before-the-page-is-a-window).
The design accepts one consequence deliberately: an ordinary double-click shows the window at once,
with the migration running behind it, before the open has finished.

`km-remote` and `km-admin` return the same bind failure into the same kind of windowless `main`.
Neither handles a document, and neither has an Open page, so the shape here does not transfer whole.
It is a known hole rather than a solved one.

`--open` is implied when nothing can be read. With no console the URL goes nowhere, so a build that
waits to be asked would start, serve and show nothing. A window satisfies `--open`, so a windowed
build never also opens a tab.

### Registering the file type

`--register` writes the association for **this** executable, wherever it currently is. That makes it
work for a portable folder unzipped anywhere. It is also why moving the folder means running it
again. **Per-user everywhere**, needing no elevation.

| Platform | What it writes |
|---|---|
| Windows | Three keys under `HKCU\Software\Classes`: `.kmbuild` → `KaraokeMachine.PackageBuilder`, its `DefaultIcon`, and `shell\open\command` = `"<exe>" "%1"`. The quotes around `%1` are load-bearing — a corpus path with a space is the normal case. |
| Linux | A shared-mime-info XML with a `*.kmbuild` glob, a `.desktop` entry with `Exec=<abs> %f`, and six icon sizes into `hicolor`. `%f`, not `%U`: a double-click delivers a path, and `%U` would offer URLs this tool cannot read. `update-mime-database` and `update-desktop-database` are best-effort. |
| macOS | **Nothing.** The type is declared in the bundle's own `Info.plist` and LaunchServices reads it when the `.app` is placed; `--register` only runs `lsregister -f`, and refuses by name on a bare executable. |

Run from the console twin, `--register` registers the **windowed** executable beside it. `--help`
lives on the console one, so that is the one somebody has in hand. A `.kmbuild` associated with it
would open a browser tab from a file manager.

The macOS bundle declares a document type and the machine's does not. It is therefore a second plist
with an identifier of its own. LaunchServices keys its database on that identifier, and a collision
means one bundle silently replaces the other. `CFBundleDocumentTypes` says *this application opens
that type*; `UTExportedTypeDeclarations` defines the type. It is exported, because this product
invents it.

`--no-desktop` takes the bundle **and** the window together. That is correctness, not tidiness. A
document double-clicked on macOS arrives as an Apple Event. A bundle around a build with no event
loop would declare the type and receive a corpus, with nowhere to put it.

## Collapsing a group of versions is a filter, not a `GROUP BY`

The song list shows one row per recording, and the row it shows is the group's representative. **The
predicate is `s.duplicate_of IS NULL`, prepended by `Filter::to_sql` beside the `s.merged_into IS
NULL` already there.** `Filter::to_sql` leaves it out when the filter names one favorite, because a
list shows every song filed in it. Every alternative shape breaks something this page is built on:

- The orderings in `Filter::order_by` are chosen to be index seeks: `s.file_count DESC` and
  `s.sort_title`. Grouping destroys index-only ordering. That trade is 9.71 ms against 4.21 s on the
  real corpus.
- `Db::songs_page` asks for `limit + 1` and reads *is there another page* off whether it got it. A
  group straddling a page boundary makes that flag lie.
- `Db::song_count` runs the same `Filter::to_sql`. A count taken through a different `WHERE` than the
  rows would offer a page that comes back empty. Going through `to_sql` keeps them equal, and the total
  travels in the URL.
- The bulk forms post one `song_id` per row, and `SongRowFragment` redraws exactly one song by id
  after an inline edit. Neither has an answer for a row standing for six.

`version_count` rides along as a column, for the reason `file_count` is one. A browse row asking
`COUNT(*)` of another table is the shape that column exists to avoid, and every row of every page
draws this one. Only the grouping pass writes it, and it gets no triggers. Nothing but a grouping pass
can change what it counts, where `file_count` moves whenever a file appears.

The pass writes `version_count` on the representative only. `browse_columns` reads a hidden version's count off its representative through
`duplicate_of`, one primary-key lookup per hidden row. It also selects `duplicate_of` itself, so the
row can mark and link it.

### Deleting is the third term, and both sides of it come from one function

A song somebody threw away carries `songs.deleted_at`, and every browse query excludes it. **The
predicate is `sql::browsable(alias)` and nothing spells it out anywhere else.** `Filter::to_sql`
composes it from `DeletedFilter::Live`'s own clause. The ten `songs_browse_*` indexes are partial on
the same function, with an empty alias. The *only deleted* box inverts the half this type owns and
keeps the other. A song both merged and discarded is still a merge, with no row of its own.

**A term on one side and not the other does not fail — it costs the indexes silently.** SQLite
serves a partial index only where the query implies its `WHERE`. A predicate with a term the indexes
do not carry therefore sends every sort back to reading the filtered corpus into a temp B-tree. That
is the 4.21 s above, with nothing on screen saying why. One copy of the predicate leaves one place for
that mistake instead of eleven. `create_browse_indexes` compares the *stored statement* rather than
the name, so a widened predicate rebuilds itself on the next open with no migration step.

**`songs_countable` takes the term as a partial predicate and never as a third key column.** The
difference between those two is four seconds a page. The obvious shape is
`songs(merged_into, duplicate_of, deleted_at)`, and measured against a whole corpus it is
catastrophic. Three equality terms in one key look like the best match available. The planner
therefore takes that index for the *browse* query as well. That index carries no order, so every
sorted page reads `USE TEMP B-TREE FOR ORDER BY` over the whole corpus.

As a partial predicate, the key stays two columns. The count still implies the predicate and stays
covering. A two-column key that supplies no order then does not out-bid one that does.

`songs_deleted` is the other side of the same predicate. It is keyed on `deleted_at`, so that
`IS NOT NULL` is a range to seek. Keyed on `id`, it would give no term at all. The planner would
then take `songs_duplicate_of`'s equality instead, which matches nearly every song and then filters.

**A query that leaves a term out loses the index entirely**, which is the same trap one step along.
A `languages_present` that asks only `merged_into IS NULL` matches none of the indexes, which are
partial on two terms. It falls back to a full scan per recursion step: five seconds, on every page
render. It therefore asks `browsable` for the whole predicate. That is also the honest list, because
a song somebody threw away should put no language in the picker.

**A table that caches an answer needs the term twice: the pass that fills it, and the delete that
makes it stale.** Two of them hold answers about browsable songs. `rebuild_folders` fills
`folders.song_count`, and it joins `songs` so a count promises what clicking the folder shows. Its
freshness marker reads `files` and the last scan. A delete moves neither, so `set_deleted_for` and
`set_deleted_of` mark the index stale themselves.

`Db::cluster` fills `songs.duplicate_of`, and it takes its pairs and its fingerprints through
`browsable`. A song thrown away therefore heads no group. A song deleted *after* a pass leaves a
group whose head no list draws, which hides every live copy behind it. The same two writers call
`release_behind_hidden`, so those copies go back on the page. The next pass collects them behind a
live head.

**A write that takes a list of ids owes the term what a filter-wide one inherits.** `Filter::to_sql`
emits the predicate always, so every *every matching song* action carries it. The ticked half of
each pair builds `id IN (…)` instead. Its ids come off a page. A page can hold a discarded song
through *only deleted*, a saved filter or a tab left open. Each of these writes reports how many rows
it changed, so a row no page draws is a number claiming work nobody asked for.

`set_language_of`, `add_tag_of`, `set_names_from_stem`, `fix_name_case`, `split_artist_from_title`,
`quality_hint` and `paths_of` each ask `browsable` for themselves.

**`package_songs` and `song_favorites` are the two tables a delete does not touch, so the reads
carry the term.** `build::spec_for` writes a `.kmpkg` from `package_members`, and the package page
draws it. `WANTED_SQL` is the union a sourced package follows. Without the term, a discarded song
ships in the next build. A sync would also count its surviving star as a song the package still
wants, and put it back after somebody removed it by hand. `WANTED_SQL` spells the clause out because
it is a `const`.

Neither asks `merged_into`. A merge has a survivor to stand in the song's place, and `WANTED_SQL`
resolves to it. Dropping the entry is the one thing a merge must not do.

**An index rebuilt under its own name invalidates the statistics that describe it.** The open reads
`missing_indexes` before `schema.sql` runs, and it holds names. An index whose key or predicate
changed keeps its name, so it is never missing, and nothing asks for an `ANALYZE`. Its `sqlite_stat1`
row survives, describing a shape the database no longer has, and the planner prices an index that is
gone. Measured, every browse sort abandons its index for a temp B-tree, at four seconds a page,
until somebody runs an `ANALYZE` by hand. `create_browse_indexes` therefore returns whether it rebuilt
anything, and the open gathers statistics when it did.

**The comparison that decides it must build the wanted statement without `IF NOT EXISTS`.** SQLite
stores a `CREATE INDEX` statement verbatim, except for `IF NOT EXISTS`, which it drops. A wanted
statement built *with* that clause never equals a stored index. All ten indexes would then be dropped
and rebuilt on every open. That is half a minute of a real corpus's open spent arriving back where it
started, with the statistics never regathered.

The scan reads the column through the join `known_files` already makes. It skips a deleted song's
files **before** the unchanged test, and regardless of `--force`. `seen` is the whole walk, and the
scan builds it before the per-file loop. A skipped file is therefore still seen, and `forget_missing`
passes it by. Undeleting needs no rescan to find the file again.

## The browse list writes

Every score, corrected name and favorite can be set from the list, not only from a detail page. Over
a corpus of hundreds of thousands of files, one song at a time is not a workflow.

**One row, defined once.** The list and the fragment route both include `templates/song_row.html`,
and `db::browse_columns()` is the single `SELECT` both go through. Every change answers with that
fragment, swapped by `hx-target="closest tbody"`. Two copies would disagree the moment somebody
touched one. The disagreement reads as *the row I just edited looks different from its neighbors*.
`a_row_renders_the_same_alone_as_it_does_in_the_list` pins it.

**A song is a `<tbody>`, and the browse table has as many of them as it has rows.** The favorite
chooser is a second `<tr>` under the song's own. The element a control swaps therefore has to be the
group rather than the line; otherwise, going back to one row leaves the chooser behind. `closest tbody`
then works in both directions, with no out-of-band delete and no ids to keep in step. htmx parses a
response inside a `<template>`. That keeps a bare `<tbody>` fragment intact, where plain table parsing
would drop it.

Rename gets its own route rather than posting two fields to `edit_song`. That handler treats every box
on its form as authoritative, which is what makes clearing a wrong artist possible. A partial form
would therefore clear language, transpose and notes.

**Favoriting is membership of a named favorite, and nothing else.** There is no `songs.favorite`
boolean; the star opens a chooser rendered as the same row. `db::toggle_favorite` decides file or
unfile by reading the database, rather than trusting a row that may be minutes old. The row carries
`favorite_count`, not a flag. It is one subquery in `browse_columns`, so a page of rows costs no extra
queries. The star can then say *in two favorites*, which a boolean never could.

**Two counts, because the fill and the color answer different questions.** `permanent_count` counts
the same memberships through a join to `favorites.temporary`. The first count fills the star, and the
second colors it. A song in nothing but working lists is in lists, and filed in none. The column is
appended last in `browse_columns`, so no existing index into the row moves. It is its own subquery
rather than a narrowing of the first, because the row needs both numbers.

**The two view boxes are classes on `#rows`, not flags on the row.** Every row's markup carries the
file name and the warning chips. `#rows.filenames .filename` and `#rows.warnings .song-warning`
reveal them. A bool threaded down would also have to reach the fragment routes, which never see the
browse query. A row would then lose both the moment anybody scored it.

`SongRows::block_class` joins the names in Rust rather than as two conditionals in the markup. The
second conditional would have to know whether the first had opened the attribute and owed a space.
That is a rule about HTML syntax, living in a template. `browse_columns` selects `s.warnings` on every
page, whether or not the box is ticked. The reason is the one that keeps the name always in the
markup.

### Two rules that are walked into repeatedly

**Nothing rendered inside `#rows` may use a name `FilterQuery` knows.** `#rows` is `hx-include`d whole
beside `#filters`. serde answers a repeated *known* key with `duplicate_field`, so a row select named
`language` turns two working buttons into a 400. Hence `row_language` and `set_language`, and hence
the impossibility of a hidden `filename=0`. serde ignores keys the struct does not know, however often
they repeat, and that keeps a hundred `song_id`s legal. Two tests hold it, one from each end of the
wire.

**The count reads the bar; the write reads what was counted.** The page renders an `hx-post`
attribute once, and the filter bar never re-renders the page. A filter baked into an action's URL
therefore describes the page as it *arrived*. Phase one takes the filter from the POST **body**
(`hx-include="#filters"`), because that is the only current copy. Phase two, `confirm=1`, takes it
from the **query string the confirmation handed back**. A bar changed while a confirmation sat on
screen must not widen a write nobody was shown.

`ConfirmQuery` is the whole discriminator. An unreadable body refuses in words:
`unwrap_or_default()` there is a package holding the entire corpus.

The same staleness has a quieter victim in the *showing* chips. `filter_chips.html` is therefore its
own fragment, and its container is in the DOM **unconditionally**. htmx silently drops an out-of-band
swap whose id is not on the page. `/songs/rows` sends the fragment back beside the rows, and pushes
`HX-Push-Url` so a reload or a bookmark keeps the filter. `saved_filters.html` is the same fragment
shape, for the same reason. Its container is on the page even with nothing in it, because the control
that saves the first filter lives inside it.

It is a **sibling** of `#filters`, not a band in it. It holds a `<form>`, which a browser drops when
nested, and `#filters` fires on every `change`. The border round both comes from `.filter-box`, the
element `songs.html` wraps them in. `form.filters`' own chrome therefore stays where the six
curation-tab forms still need it, and `.filter-box > form.filters.banded` gives it up.
`#saved-filter-result` is inside that box and outside the strip. A swap target inside the element an
out-of-band swap replaces is the hazard `filter_chips.html` documents from the other direction.

**Saving a filter is the exception to both, and the exception is worth the paragraph.** It sends no
bar, and it parses no filter out of its body. Every change to the bar goes through `/songs/rows`. That
route writes the canonical query string into `State::songs_filter` before it answers. The server
therefore already holds the exact string the address bar shows, `offset` clamped and all. That makes
*the count reads the bar* vacuous here, and the `duplicate_field` hazard unreachable.

What it costs is a dependency. A route that re-renders `#rows` without writing the filter down would
leave this saving a page nobody is on. Six routes redraw the rows: `GET /songs`, `GET /songs/rows`,
`POST /songs/titles-from-filename`, `POST /songs/fix-name-case`,
`POST /songs/split-artist-from-title` and `POST /songs/delete-bulk`. A test holds each of them to
writing down the offset it drew, which is the clamped one and not the one asked for. The last four
share `redraw_over_the_write_clearing`, so the rule has one home rather than four copies.

The similar-names page posts the same two routes, with `?as=hits` and the search bar ahead of the
ticks. `Redraw` reads that bar with `SimilarQuery::from_fields` before the write. The answer is
`#hits` drawn by `similar_for`, with no filter written down.

A chip is `saved_filter_chip.html`. The strip includes it, and `GET /songs/saved-filters/{id}/chip`
renders it on its own. That is the arrangement of `song_rows.html` and `song_row_fragment.html` over
the same markup, so the strip and the one chip a rename reopens cannot drift. The `renaming` flag is a
field on **both** view structs, because askama's `include` shares the enclosing scope. That is why
`SavedFilters` carries one it never sets. Rewriting and renaming answer with the out-of-band strip.

Only opening and closing the box answers with the chip, so a second box somebody has open stays as it
is.

### The quality hint, and why it does not redraw the rows

`crate::hint` holds the whole of the ordering: a `Key` of the columns a scan wrote, and an `order`
that sorts them. `Db::quality_hint` fetches the keys for a list of ids, restricted to `kind = 'midi'`,
and hands them over. It uses **no `ORDER BY`**, for three reasons. No index covers seven keys over an
arbitrary list, and the list is at most a page of ticks. A comparator can also carry the reasoning
that produced it, where a formatted SQL string cannot.

The decision is
`A quality hint is a position on the row, and it is rubbed out rather than kept` in
[`curation.md`](../decisions/curation.md).

The result lives in `State::quality_hint` as an ordered `Vec<String>`, and the position is the
number. `State::publish` empties it along with the outgoing folder's filter. `State::mark_hints`
writes each row's place onto it in `rows_for`, the one place both `/songs` and `/songs/rows` build
their rows. That is the `tags` pattern, one level up. Where a song sits in this run's hint is not a
fact the browse query could select.

`similar_for` calls it too, so `/similar` and `/similar/hits` draw a match's number the same way. The
three fragment routes that re-render one row call `mark_hint`. The reason is the one for which they
already carry the last-played highlight. A row swapped back after an inline edit must not come back
without its badge.

**The answer is out-of-band badges and never a redraw of `#rows`.** Two things point that way. A
badge is a few characters inside a row that is otherwise untouched. Redrawing the rows to place one
would put these two routes under the rule the paragraph above states. They would each owe
`State::songs_filter` the offset they drew, and the list of routes that redraw the rows would grow by
two.

And a hinted song that is not on the page being looked at has no element to swap. htmx answers that
by dropping the swap, and the row draws its number from `SongRow::hint` when somebody reaches that
page. `hint_marks.html` carries an empty badge for every song that has *lost* its number. A row keeps
whatever markup it was last given, and `.place:empty` is what takes the mark off it.

`POST /songs/quality-hint` reads `song_id` out of the body with `Fields` rather than through
`BulkAction`. That extractor adds a whole-filter scope and a confirmation pass, and this route has
neither. The form is `hx-include="#rows"` alone. The route reads no filter, so the bar has no reason
to be in the body. That also ends the `duplicate_field` hazard for this one. The similar-names
page posts the same route with `hx-include="#hits"`, where its ticks are.

### Forms, templates and the one script

**Form bodies are not read with axum's `Form` extractor.** It goes through `serde_urlencoded`, which
cannot represent a repeated key. A repeated key is exactly what a form of ticked checkboxes sends.
Asking for a `Vec<String>` errors. Asking for a `HashMap` is worse: it silently keeps one value, so
ticking three favorites would file the song in one and say nothing. `src/form.rs` parses the body.

Templates are askama, compiled in and checked at build time. **htmx, the stylesheet and the license are
compiled in too.** A machine with a corpus on it may have no internet. Serving them off disk costs an
`--assets` flag, a three-directory search and a startup check. It also adds a failure mode where every
button is inert, because a copy left one directory behind.

With the files compiled in, the handler sets the `Content-Type` by hand. A test pins the bytes, since `include_str!` catches a missing file at
build time but not an empty one.

**htmx does not swap the response of a failed request**, by design. A page turn that errored would
therefore look identical to one with nowhere to go. Answering 200 so htmx will swap is right wherever
a refusal has a fragment and a slot to come back to. `/songs/rows` has neither, and swapping error
text into `#rows` costs the rows you were reading. So the status stays honest. `static/ui.js`, the
tool's only JavaScript, listens for htmx's error events and puts a line in a toast tray.

The doc comment of `static/ui.js` says what the script may not become. It may hold no client-side
model, do no templating in the browser, and keep no state the server does not hold.

**A body that is only out of band tells htmx to leave the target alone.** `views::toast_only` empties
whatever the caller aimed at. That clears a stale message, and it is what a slot wants. A caller aimed
at `#rows` with `outerHTML` wants the opposite: htmx lifts the toast out, and the empty remainder left
behind takes the table away.

`views::toast_only_leaving_the_target` sends `HX-Reswap: none` for those.
The header rides on the response, because one route answers both kinds of caller. htmx runs
out-of-band swaps before it reads the swap style, so the sentence still reaches the tray.

`views::with_toast_clearing` is the mirror of it. An answer aimed at the list can empty a slot
outside it. That is how the bulk-delete confirmation goes away once somebody has acted on it.

**The two selection gestures are in that file for the same reason.** One is the box in the table head,
and the other is the shift-click that ticks the run between two boxes. htmx has no opinion about
either; they are not gaps in it. Neither breaks the rule above, and the run is the one that looks as though it
might. What it holds is the box the last press was on. The selection stays in the ticked boxes, where
`hx-include="#rows"` reads it off the page.

Both gestures are bound to `document.body` rather than to the elements, because every page turn and
every filter change replaces `#rows` outright.

**The words are a catalog, and one argument carries it.** `i18n/en.ftl` and `i18n/pt-BR.ftl` sit
beside the templates, and `src/words.rs` compiles them in with `include_str!`; `{{ "nav-songs"|t }}`
spends a key. The five render seams in `src/views.rs` take a `km_locale::Locale` and pass a values
store in once. askama carries that store into a nested `{{ child|safe }}`, so no template struct holds
a locale, and nobody has to tell a fragment. `toast.html` and `message.html` take none. Both are one
interpolation of a sentence somebody else already worded.

**The store is `&dyn Any`, which is neither `Send` nor `Sync`.** All five seams are synchronous for
that reason. A render held across an `.await` makes the whole handler non-`Send`. axum reports that as
`Handler` not implemented for the function, and it names neither the line nor the value.

**Markup carries a key and nothing else; a sentence carrying a value is composed in Rust.** A count
is a plural, and a plural is arithmetic. The header's counts, a confirmation's subject and button, a
pager's range and a row's four tooltips therefore all arrive as fields. `State::say_rows` is where a
page of rows gets them. It sits beside `mark_hints`, for the same reason: what a row *says* is a fact
about the page rather than about the corpus.

**What is worked out where no language is in reach travels as a fact.** A scan's phase is a key
(`scan::phase`). A build's is an enum carrying its own count or file name (`build::Phase`). An open's
is `db::OpeningPhase`, and a scan status is `ScanStatus::key`. `DbError` and `AppError` each keep an
English `Display` for the log, and grow a `say` for the page. The console banner asks `OpeningPhase`
for its English rendering, so one set of words serves both.

**`static/ui.js` reads its five sentences off `<body>`** as `data-js-` attributes, the way
`km-remote-pages`'s `scan.js` already does. A static file cannot go through the `|t` filter. An
English string in one would appear inside a Portuguese page, with nothing to catch it. Two of the
sentences carry `{what}` and `{status}`, which only the browser has. The catalog therefore composes
the pattern with those words standing in, and the script does one replace into `textContent`.

**Four tests in `src/words.rs` are what make the whole of it checkable.** The first checks that the
two locales hold the same keys. The second checks that every key the markup or the Rust asks for
exists. The third checks that nothing in a catalog goes unspent. The fourth,
`no_template_carries_its_own_prose`, reads every template with the markup stripped, and fails on
whatever words are left. `server.rs`'s `no_page_draws_a_key_in_either_language` drives the real
router over every page in both languages, and asserts no `⟦` reaches one.


Toast text goes in with `textContent`, never `innerHTML`, because it quotes file names out of a corpus
nobody wrote. Nothing about *becoming visible* is animated. A browser that throttles animations in a
background tab can leave a fade-in at zero. The one thing that had to be read is then invisible. The
element's own properties draw it, and the animation is a delayed fade *out*.

Successful list actions toast too, out-of-band rather than through an `HX-Trigger` header.
`XMLHttpRequest.getResponseHeader` decodes as ISO-8859-1, and these messages are full of Portuguese
titles. A `MutationObserver` arms whatever appears in the tray. htmx announces an out-of-band swap with
different events, depending on how it got there. A toast that is never armed stays on screen for ever
with `pointer-events: auto`, and swallows clicks.

### Tags: two tables, and why the vocabulary is one of them

`tags` and `song_tags` have the shape of `favorites` / `song_favorites`. What separates them is what
they mean. A favorite is somebody's own filing of this corpus. A tag is a word about the song that the
machine's own catalog carries. Both are new tables, so `schema.sql`, which runs in full on every open,
is the whole migration.

**The `tags` table is the vocabulary, and it exists so that *what tags are there?* is a read.**
A language is a column, so `Db::languages_present` beside it has to compute the distinct set with a
`WITH RECURSIVE` skip-scan over `songs`. A `tags` row appears the first time
somebody uses a tag, and goes when its last song loses it. The picker is therefore a list of words in
use, not a museum of every word ever typed. On hundreds of thousands of songs, that is the difference
between a useful datalist and one nobody reads.

**Tags touch neither `create_browse_indexes` nor the FTS triggers.** Both facts are worth stating,
because the rest of this page would suggest otherwise. The filter is an `EXISTS` against an indexed
join table rather than an expression on `songs`, so there is no `songs_browse_*` companion to add.
And a tag write touches neither `songs` nor the columns `songs_fts_update` names, so a bulk write over
a filter cannot retokenize a title. The obvious alternative, a `songs.tags` column as the
machine's catalog has, would retokenize every row it touched. There is a test.

**The suggested vocabulary is not in this database at all.** It is in the curator's own config
directory beside `recent.json`. What a good tag looks like follows a person from one folder to the
next, while a machine's address does not; see `settings.rs`. `spec_for` reads `song_tags`, so a
suggestion nobody has put on a song has no row, and cannot reach a package. A catalog is built from
packages, so the suggestion cannot reach a machine or a phone either. A test asserts that path end to
end, rather than this page describing it.

## Scanning

The scan is shaped after `km-lyrics scan`. It collects paths, chunks them across scoped threads, and
runs `catch_unwind` per file, with a panic hook that records rather than prints. **One writer thread
owns the connection**, and writes the results in batches of 500 rows per transaction.
`rusqlite::Connection` cannot be shared, and a mutex per row would be worse than either.

**The channel is bounded**, at four batches. An unbounded channel lets a run over a whole corpus put
hundreds of thousands of parsed songs into it. That is memory nobody budgeted. The progress count
becomes a claim about reading rather than about the database. The scan also cannot stop promptly,
because stopping means draining. The bound leaves throughput unchanged, because a pipeline runs at
the speed of its slowest stage, whatever the buffer.

**Raising the bound needs `tune`'s mapping taken off in the same change.** With the mapping on, the
pages the writer descends live in the OS page cache rather than in SQLite's own. A deeper channel
lets the readers run far enough ahead to stream a corpus of files through that cache. That evicts the
database out from under the writer, and turns every descent into a fault against the disk. Measured
over the real corpus, a bound raised tenfold with the mapping on gave a ninth of the throughput. The
comparison is the same bound with the mapping off, and the shipped bound either way.

The sort keys also decide more than they look like they do. Six of the nine browse indexes that
`create_browse_indexes` builds key on them through `WITHIN_TITLE_KEY`. A scan that clears and
rewrites the sort keys therefore rewrites those six as well.

**The writer is that slowest stage, and what it spends a scan doing is index maintenance.** Each batch
upserts into `songs` against its secondary indexes. It fires the triggers that keep two full-text
indexes and the sort keys in step, all inside one transaction on one thread. The `songs` triggers
therefore each carry a `WHEN` comparing the columns their body reads. `UPDATE OF` fires on assignment
rather than on change, and the upsert assigns every detected column on every row it writes. A
re-analysis exists to move suitability, and it leaves a song named what the file already spells it.

Without the guard, each of those rows leaves both full-text indexes and goes back in. Both its sort
keys are cleared and written back, only to arrive at what is already stored.

**Two further ways of making that writer cheaper were measured, and neither moved it.** The first put
every index on `songs` and `files` aside for the length of a forced pass, and rebuilt them at the end.
It read 16.6 songs a second against 17.3 without it, and cost 215 seconds of rebuilding on top. The
second wrote each batch's songs in id order, so the writer descends the primary key in order rather
than at random. It read 17.1.

**What is left after the guards is neither index maintenance nor write ordering.** It is reading a
few hundred scattered table pages off a platter per batch. A song's id is the hash of its bytes, so
the construction itself scatters those rows. Neither dropping an index nor sorting a batch changes how
many distinct pages a few hundred scattered rows sit on. A reordering wide enough to coalesce them
would have to buffer far more rows than one batch. That is the bound the note above warns against
raising while the database is mapped.

**The bar counts rows committed, not files read.** The writer counts `written` after each batch
commits, so the bar is `(skipped + written) / total`. A skipped file never reaches the writer, and
settles at once. Paging is still `OFFSET`-based, so a scan can make a song miss a page boundary.
Keyset paging on the existing `ORDER BY` tuple is the fix, if that matters.

**The panel under the bar is the run's steps, listed before they run.** They are a `step::Ladder`,
the same type as the Open page's rungs. `run_inner` plans them from the options first. A scoped run
lists no forgetting and no duplicates. The three steps that depend on `changed` carry `if_changed`,
and the run skips them when it is false.

`Ladder::say` closes the running step and starts the next. `skip` names the steps this run will not
take, and `end` settles whatever is left. `timings()` reads off the finished steps.

The walk goes through `km_pack::collect_songs_observed`. It reports the count after each folder into
`found`, and breaks off when somebody asks the run to stop. A broken walk ends the run before reading,
because its list is part of the folder. **The time left comes from a 30-second window** of settled
counts. `snapshot` samples them at most once a second while the reading step runs. The page shows the
time left only once the window spans ten seconds.

**Ctrl-C stops the scan rather than killing it, and so does the page's Stop button.** `Progress`
carries a `cancel` flag that the workers check *between* files. One file takes milliseconds, and
stopping half-way through parsing gains nothing. The writer deliberately does not check it. Draining
what the readers read is the point, and the bound keeps that under a second. A second Ctrl-C exits at
once.

`POST /scan/stop` sets the flag through `Workspace::ask_scan_to_stop`, and does not join. The request
therefore answers at once, and the panel's poll reports the end. Joining stays with
`Workspace::drop`.

**A stopped run draws none of a finished run's conclusions.** Recording `last_scan` would claim the
folder had been read when it had not. A canceled run therefore commits and returns. The page says
*stopped*, and notes that re-running resumes free.

**Stop reaches the tail as well.** `conclude` checks the flag before grouping duplicates, before
indexing folders and before measuring. It hands the flag to `rebuild_folders_unless`. A stop there
sets `canceled` and `tail_skipped`. `last_scan` is already stamped, so the panel says the folder is
scanned, and that the Folders page rebuilds the tree.

**Incremental by default.** The scan skips a path whose size and mtime match its row, before opening
it. `--force` re-analyzes everything, which is what to do after the analysis heuristics change and
never otherwise. The scan forgets files gone from disk, **except** where a package still names the
song. That song stays, shown as *source missing*. Losing a curated selection to an unmounted drive is
the one failure this must not have.

**The corpus is walked once, not once per kind.** `km_pack::collect_songs` classifies each entry off
the directory listing. A per-kind collector opens every directory under the root and stats every
entry. Three of them would walk a mixed corpus three times to answer one question. The three stay for
`km-pack spec`, which wants one kind at a time.

**A file is read once.** Reading and hashing before dispatching wastes the read for an MP3+G pair,
whose identity is the hash of *both* halves. The audio would be read three times, and the graphics
twice. The audio branch therefore sits above the read, and `km_cdg::probe_from` probes the bytes
already in hand. A video still reads twice. There the hash *is* the identity, so the scan must go
through the bytes, and `km_video::probe` takes a path.

### The tail, and what a hold of the writer costs

The passes after the reading are the only stretch of a scan that holds the writing connection for
longer than a batch. **What waits on that connection is every write and no read.** A page is drawn
through a connection that can only read. A star somebody clicks therefore feels the tail, and the
pages do not. See [`Two connections, and which one a page is drawn through`](#two-connections-and-which-one-a-page-is-drawn-through).

**Two of those passes cannot be taken in bites, and that is what a write pays for.** A folder tree is
one pass over every file, and `ANALYZE` is one statement. Neither can release the connection
part-way, and `stand_off` reaches only the gap between them. Measured at the end of a scan over a
whole corpus that had something to write:

| phase | measured |
|---|---|
| looking for files | 3.0 s |
| reading and analyzing | 145 m 41 s |
| forgetting files that are gone | 252 ms |
| indexing folders | 12 m 59 s |
| measuring the corpus for the query planner | 11 m 17 s |

For the best part of half an hour at the end of such a scan, a write therefore waits out
`WRITE_WAIT` and is answered `DbError::Busy`. Reads are unaffected throughout. Closing that window
would take breaking those two passes up, or deferring them, and this tool does neither. Stop ends the
folder pass between rows and skips `ANALYZE`, but it releases the connection only by ending the pass.

The scan pays for the folder tree here rather than on the Folders page. The page then opens at once
after a scan; see
[`The corpus is browsed by folder`](../decisions/curation.md#the-corpus-is-browsed-by-folder).

**The whole tail is gated on a row having changed.** Nothing above that gate could reach a different
answer without one. The folder tree would be rebuilt from unchanged paths, and `ANALYZE` would measure
a corpus whose shape had not moved. `last_scan` stays unconditional. It records that the folder was
*read*, which is true of a scan with nothing to do.

**`forget_missing` is handed what is gone, not what is present.** Handing it every path on disk means
inserting them all into a temp table one row at a time. It then deletes with two `NOT IN` anti-joins
that scan `files` and `songs` whole. Every completed scan would pay that in full, and the ordinary
scan deletes nothing. The set difference costs nothing where it sits instead. `known_files` already
holds every path in the table, and the walk already holds every path on disk.

The caller therefore subtracts one from the other, and passes only the rows that go.
`forget_missing` deletes them by exact path on the `path` unique index. Empty does no SQL at all.

The orphan sweep follows from that. **A song can only lose its last file when one of its files is
deleted.** The sweep is therefore scoped to the songs whose files just went, at two index seeks each,
rather than a full scan of `songs`. `file_count = 0` stays as a backstop for an orphan an *earlier*
version left behind. A whole-table statement catches that as a side effect. `file_count = 0` is a
seek on `songs_file_count`, not a scan.

**A foreign key onto the tables this deletes from needs an index.** `package_songs.file_id` is
`ON DELETE SET NULL`, and nothing else reads it. Without an index, and with `foreign_keys = ON`, each
deleted row scans the child table whole. The cost is latent while nothing is deleted, and quadratic
the day a drive is reorganized under the corpus. `package_songs.file_id` is therefore indexed.

**The lock is held in bites.** Deletes re-take it per chunk, the way the writer already does per
batch. The tool is therefore never unanswerable for the length of a whole-corpus delete.

**Every phase is timed**, and the Scan page says so. Four of the seven are whole-corpus passes. Without
the timings, the page reports only a phase's name. "Which phase" is the first question of every
complaint that the scan is slow. Without timings, it has no answer short of attaching a profiler to
somebody's corpus.

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

The 202 ms is the set difference and nothing else. `gone` was empty, and that is also the strongest
available evidence that `collect_songs` finds exactly what the three collectors found. Every path
matched the table row for row. A single kind dropped from that walk would have deleted every file of
it. `one_walk_finds_exactly_what_three_walks_found` is the test that stops it being evidence about one
corpus.

The lock holds in the tail are now too short to observe while loading a page. That is the honest
limit of what was verified: the shorter holds are argued rather than measured.

**A scan skips a file only when nothing about it *and* nothing about the analysis has moved.**
`songs.analysis_revision` carries `km_suitability::ANALYSIS_REVISION`. `known_files` joins it in
beside the size and time, and the skip check asks for all three. `files_song` and the `songs` primary
key make that join a keyed lookup per row, rather than a second scan of `files`. That join is the one
place this could quietly cost minutes, and the thing to re-measure if an open ever slows down.

Schema ladder step 11 adds the column and leaves it NULL. NULL is what every row written before it
means: nobody recorded what produced them.

`the_analysis_revision_covers_what_the_fixtures_say` is the guard. It hashes the tuning constants as
well as the fixtures' results. **Measured while writing it**: moving `sync_window_ms` from 120 ms to
119 changes not one fixture. It would change thousands of rows in a corpus of hundreds of thousands,
so the results alone are not enough. `Thresholds`' `Debug` output goes into the digest, for the same
reason a derive is safer than a field list.

**`Db::promote_unreached_revisions` runs on every writing open**, after the text clean-up and before
anything counts stale songs. It takes each entry of `km_suitability::REVISIONS` in order. For each,
one `UPDATE` raises the rows at the revision before it that its `Reach` excludes.
`Reach::Everything` runs nothing. A row therefore climbs until the first revision that reaches it.

`LyricLinesAtLeast(n)` excludes a `line_count` below `n`, and a NULL one, which is every video and
MP3+G row. `SyllablesAtLeast(n)` does the same over `syllable_count`. A NULL revision is never
promoted. On an open with nothing to promote, each statement matches no row.

**`Reach::EverySong` raises no song, and still raises the `files` rows that hold none.** That is the
one thing separating it from `Reach::Everything`. A readme, an orphan `.cdg` and a MIDI file that
does not parse have no suitability that could have been decided differently. Re-reading them buys
nothing. A revision that could turn one of those into a song reaches everything instead.

| Revision | Reach | Why |
|---|---|---|
| 2 | 1 or more syllables | the melody's presence gate passes every channel when there are no words |
| 3 | 8 or more lyric lines | a chord chart needs `min_chord_lines` chord lines, each a lyric line |
| 4, 5 | 32 or more syllables | the divider is judged no lower, by `MIN_JUDGED_SYLLABLES` |
| 6 | every song | no stored column bounds how long a song is sung for, and the number is written for a video, an MP3+G pair and an UltraStar song for the first time |

**`--reanalyze` is that scan with the set chosen from the database rather than from the disk.** It
takes `paths_matching(&scan::every_song())` and hands it to `ScanOptions::only`. It therefore reads
one copy per song, where an ordinary scan reads every copy. The skip check works from a snapshot
taken before the run. The second and third copies of a song therefore still look stale in the same
pass that repaired the first. `reanalyze_and_exit` polls the same `Progress` the page would, and
reports it through `km_console::Meter`.

What it saves is the reading. `paths_matching` selects one path per song, **a bit over half the
files**. A scoped run asks none of the whole-corpus questions: no forgetting, no `last_scan` stamp,
no near-duplicate pass.

**A re-analysis over a whole corpus on a spinning disk is bound by the writer, not by the reading.**
The readers outrun the one writer thread, and spend the run blocked on the bounded channel. The drive
is therefore busy with a single thread's index maintenance rather than songs. Measured on the real
corpus, a batch commits about once a minute. Between commits, every reader thread sits in a wait
state at a fraction of one core. The readers' own speed shows only in the moment a commit frees the
channel, when they empty it at streaming speed and block again.

**The readers do not make the writer slower either, and the byte counts are why.** The corpus and
its `.kmbuild` sit on one platter. The worry is that the readers seek against the arm the writer
descends its scattered pages with. Measured over a real corpus at six reader counts, each count twice
and each arm over 4,000 files no other arm read:

| readers | files a second, the two arms | drift-adjusted mean |
|---|---|---|
| 1 | 32.0, 29.4 | 30.7 |
| 2 | 30.6, 28.1 | 29.4 |
| 4 | 32.1, 34.0 | 33.1 |
| 8 | 35.1, 31.8 | 33.5 |
| 16 | 34.5, 32.3 | 33.4 |
| 24 | 31.9, 26.6 | 29.1 |

The spread across the counts is 13.9%, and the largest spread *within* one count is 16.8%, so no
count is distinguishable from any other. **An arm moves 2 to 3.6 GB off the platter, and its 4,000
song files are about 120 MB of that.** The traffic is the writer's index maintenance, and the readers
are a twenty-fifth of it. Dividing a twenty-fifth differently cannot make it matter. The same arms,
run with the files in the page cache, read 112 files a second against 32. That measures how far this
is from being bound by anything but the disk.

**That measurement needs two things, and a sweep without either reports its own artefacts.** The arms
drifted 14.5% slower across the run, for reasons nothing here explains. Each count therefore runs
twice at mirrored positions, which cancels a linear drift exactly. And an arm whose files are already
cached reads zero bytes, and runs three times too fast. The platform's disk counters therefore
bracket each arm, and an arm reading zero is discarded rather than averaged in.

The count stays settable for the disk that disagrees, through `--jobs`, `KM_SCAN_JOBS` and
`scan_jobs` in the settings file. `db::measure::how_many_readers_a_disk_wants` is how to ask another
disk. `KM_SLICE` lets each arm be its own process, and that gives each one its own page cache and its
own counters. The decision is
[`How many files a scan reads at once`](../decisions/curation.md#how-many-files-a-scan-reads-at-once).

**A process at a fraction of one core is what waiting looks like as much as what seeking looks like.**
A rate alone therefore says nothing about which stage is the slow one. Reading one folder end to end
runs at 243 files a second. Extrapolating from that under-calls a scattered read by an order of
magnitude, and neither figure says anything about the write path.

**Measure a change to any of this against a drive that is actually being read.** Repeating a
`--reanalyze` restarts it at the beginning of the sorted path list. The second run, and every one
after it, therefore re-reads files the page cache already holds. The disk counters go to zero reads,
and the figure on screen climbs run after run. What is being timed is how fast the machine appends to
a write-ahead log with the whole corpus in RAM. A measurement with no disk reads under it is not a
measurement of this.

Comparing two settings in the order they were thought of credits the second with the warming the
first paid for.

What the scoping buys is real, and it is a fraction, not an escape. The forced scan reads every file
instead, and ends with the near-duplicate pass on top. What the scoped run still pays is the walk and
the `known_files` load. `only` narrows the path list *after* `collect_songs` rather than instead of
it. A scoped run therefore reaches a file by the route a whole one does, and cannot disagree with it
about what is there. At 15.5 s the walk is not what anybody is waiting for.

**Two traps live in that function and both are load-bearing.** `Filter`'s default collapses a cluster
to its representative: `s.duplicate_of IS NULL`. Asking for the default would therefore measure the
representatives and report having finished. `a_version_set_aside_is_still_a_song_to_re_analyze` is
what says the two counts differ. And `ProgressView::percent` is the share *written*, not read. A line
pairing it with `done` would show a run doing nothing through the first batch, so the meter counts
songs read instead.

**The meter itself is `km-console`'s**, not this crate's. A meter per command lets the commands
disagree. `km-pack` rewrites a line for its walk and another for its packaging, neither says a rate,
and only one survives being piped. `Meter` says the count, the total, the percentage, the rate and the
time left. It rewrites in place where there is a cursor, and prints whole lines where there is not.

`Meter` holds its tongue about speed for the first two seconds. The first draw is immediate, so the
run is visibly alive. A count divided by a fraction of a second is a five-figure rate that has
measured nothing.

**It goes after the Ctrl-C watchdog where `--backup` goes before it**, and runs through `Workspace`
rather than calling `scan::run` directly. Both have the same reason. This one writes a quarter of a
million rows over twenty minutes. It therefore needs an interrupt that ends it cleanly, and the
`Db::close` checkpoint that `Workspace::drop` performs. A backup has no use for either.

**And the windowed executable refuses the flag**, which `detaches_from_the_shell` decides. That is a
rule over `Shell` rather than a platform test, so `only_the_windowed_executable_hands_its_prompt_back`
runs on all three. `main.rs` asks for a GUI subsystem wherever there is a window, and a command
processor does not wait for one of those. The prompt comes back, and the run carries on behind it.
The progress this function exists to print then lands on a prompt that has moved on. The button is
the windowed build's answer, and the refusal names it.

**The button's field and the template's `name` are one string in two files.** That is the shape that
rots without a round trip. Rename either, and the button still renders and still posts, but quietly
starts a whole forced scan instead of the cheap pass it offers.
`re_analyzing_the_songs_does_not_claim_the_folder_was_scanned` posts what the page posts. It asserts
on `last_scan`, which only a whole run moves.

**Exact duplicates need no pass, and the near-duplicate pass is the forced scan's alone.** The
content hash already collapses identical files onto one row, and the count is a column on it. The
near-duplicate pass buckets a coarse structural fingerprint, keys every lyric, and joins the pairs
into groups. That is a whole-`songs`-table read, measured at 10.4 s over the whole corpus. It is worth
asking for, and not worth paying for a file that moved.

The pass is therefore the tail of **Re-analyze everything**, which has just rewritten every
fingerprint it reads. At every other time it is a button on the Duplicates page. The scan that found
a changed file runs neither. See `Duplicate aggregation` in docs/decisions/curation.md.

**The lyric key is computed from the stored `lyrics` column and never written back.** `Db::fingerprints`
folds each row's words as it reads them, and keeps only the hash. The text of the songs that have any
is therefore read and dropped rather than held. With the key out of the table, revising the word
floor or the credit rule costs a button press, not a `--force` re-scan.

**The lyric pass emits a star and the fingerprint pass emits a clique.** That is a difference in pair
count and nothing more, because grouping is what reconciles them.

**A dismissal is a constraint on the grouping rather than a deletion of an edge**, and it has to be.
Two reasons arrive from opposite directions. Joining is transitive and a dismissal is not, so a clique
reconnects the two songs a person separated through any third member. And a star holds no edge
between two of its leaves at all. The song page offers *Not the same* between every pair of versions.
A dismissal there would have nothing to update, and would be silently lost.

`Db::dismiss_pair` therefore inserts the verdict when no pair was suggested. `DisjointSet::union`
refuses an edge that would put a dismissed pair in one group. The edges are sorted first. Once an
edge can be refused, which one is dropped decides where a chain breaks. That must not depend on the
order SQLite returned its rows in.


## The Advanced tab follows the words, not the channels

The tab is drawn for a file with channels to show, which is a MIDI song whose bytes are still
readable. It holds two unrelated things: the channel table, and one control saying whether the
machine draws the song's words. `SongPage::shows_advanced` is therefore *has channels **or** draws
its own words*. The channel table keeps a condition of its own inside the pane, and an UltraStar song
opens onto the words control alone. A video or an MP3+G song gets no tab, because its words are
pixels in a picture.

**One method answering for the label and the pane** is what the split buys. A tab whose label is
drawn and whose pane is not opens onto nothing. Two copies of the condition in a template are two
chances to change only one.

**A route of its own, beside the two forms the page already has.** The Details form treats every box
on it as authoritative, so a field it does not send is a field it clears. The corrections form posts
the whole channel table, which an UltraStar song does not have. `POST /songs/{id}/lyrics-hidden`
writes one column, and leaves the rest of `SongEdit` at `None`. That keeps the three forms from
writing over each other.

The select spells three answers, because the column holds three states. `auto` stores null and hands
the song back to the analysis. `show` and `hide` store a person's answer. **`show` storing a `0`
rather than a null is the part that matters.** `Db::hand_set_predicate` reads *set at all* as
`IS NOT NULL`. Without the `0`, a person overruling the analysis and a person who never opened the
page would leave the same row.

The *Automatic* option names what the analysis concluded, the way the encoding box names a guess.
The page reads that answer off the warnings the row already carries. It checks
`km_pack::warnings_hide_words` against the spelling `warning_code` wrote. The automatic half
therefore costs no rescan and no column.

## Building a package

The build runs on its own thread in three phases. A short lock reads the description. Then the
parsing, analysis, any ffmpeg re-encode and the archive write run **with no lock at all**. A last short
lock records what was built. `crate::build::build` therefore takes the `Arc<Mutex<Db>>` rather than a
`&Db`. That signature looks like a downgrade, and it is the point, because it decides *when* to lock.

A build run inside the request would take the workspace's single mutex for the whole build. Every
other page and every poll would queue behind it. A progress bar added to that would be the one
request that could not be answered.

- **`Workspace::drop` stops the build before the checkpoint.** A build holds no lock for most of its
  life. `db.lock()` in `Drop` would therefore succeed while one is running, and close the connection
  under it. The final `record_build` would then write into a closed database.
- **Stopping costs at most one song.** `km_pack::build` asks whether to carry on *between* songs rather
  than inside ffmpeg. The archive goes through a temporary file and a rename, so a stopped build leaves
  the package it was replacing exactly as it was.
- **One build at a time**, deliberately serialized. Two threads would both call `record_build`, and the
  page has one place to put a bar.

`packages.default_language` fills any song with none **in the package, never writing back to `songs`**.
That clause is the whole design. A package saying *call the rest English* is a statement about one
package, while the corpus goes on saying *nobody has said*. Absent and blank differ in the form. The
settings form carries the key and may leave it empty, which restores the strict refusal.

**A package's page also writes its description**, the same `*.kmspec.yaml` `km-pack build` takes. It
writes it from the same value the build here consumes. No second code path could therefore describe a
package differently from the way it is built. A test asserts the two manifests are identical.

`dismissed_failures` keys on `(file_id, scan_status)`, and needs no migration step. `schema.sql` runs
on every open and creates it with `IF NOT EXISTS`. A rescan updates a file row in place through
`ON CONFLICT(path)`, so an id survives a rescan, and a dismissal holds. A file deleted from disk takes
its dismissal with it through the cascade. Both sides of the panel come from one `Db::tally`. It takes
the side as a parameter, so the count and the example it shows come from the same population.

**The version is raised across two of the three phases**, and the split keeps the archive and the row
in step. Phase one computes the raised version, and puts it into the description the build is about
to consume. Phase three writes it to `packages.package_version`, inside `if outcome.wrote()` beside
`record_build`. A package that was never written therefore spends no number. One that was written
carries the number the row reports. `spec_for` is untouched, so **Write spec** describes the package
as it stands: writing a description is not a build.

`Db::raise_version` and `Db::set_raise_version` read and write `packages.raise_version`, rather than
a field on `PackageRow`. `update_package` sets every editable column from a row that
`handlers::package_row` builds out of a form. Neither the create form nor the settings form carries
the tick box. A field on the row would therefore be cleared every time somebody saved a package's
name. `Db::set_package_version` is narrow for the matching reason. The build's last lock holds no
`PackageRow`, and must not overwrite a name or a language edited during the minutes it was unlocked.

## Sourcing a package from favorites

One table, `package_favorites`, and no migration step. It is new, and `schema.sql` runs in full on
every open with `IF NOT EXISTS`: the `tags` / `song_tags` arrangement. Nothing is added to
`packages`, because a row in that table is the whole of whether a package is sourced.

**One SQL constant is the union, and the count and the write both run it.** Four statements
interpolate `WANTED_SQL`, all binding `:package`. They are the ordered read the placement walks, the
delete's `NOT IN`, and the three counts `package_sync_plan` answers with. A confirmation saying twelve
songs go in, over a write that reads a different set, is a number nobody can check. `MERGE_SQL`
already holds that discipline, one method over.

The union is driven from `package_favorites` rather than from `songs`. It therefore costs the size of
the lists, not the size of the corpus. Its `ORDER BY` is `filter::WITHIN_TITLE` verbatim, so a package
numbers in the order the browse list draws.

**The deletes come first inside the one transaction, and that is what makes gap-filling possible.**
`number` is half `package_songs`' primary key, so a freed slot cannot be handed out while the row
holding it is still there. The order also removes the need for `renumber_package`'s two-pass walk
through negative numbers. Nothing already in the package moves, so there is no collision to park
around. `free_numbers` then walks each volume's `start_number ..= MAX_SLOT`, filtering out what
survived. That is 999 candidates a volume, so the naive filter is exact and free.

**Holds are written before the deletes, from the same `NOT IN`.** `package_held` keeps one row per
held number, keyed like `package_songs` by `(package_id, volume, number)`. The title and performer
are copied in as the song had them. `song_id` is `ON DELETE SET NULL`, so a hold outlives the song it
names. A number is a member or a hold, and never both. Every write that puts a song at a number
deletes the hold there in the same transaction.

`free_numbers` treats a hold as taken, and `next_number` takes its maximum over both tables.
`renumber_package` computes its targets with the holds filtered out. `place_songs` asks
`HELD_FOR_SQL` first for each song, which matches a hold through `coalesce(merged_into, id)`. A song
with a hold takes that number back. `fill_held` is the one write that moves a song between volumes.

**`place_songs` is the one copy of the insert.** It is a free function over the connection, for
`next_number`'s reason. `add_to_package` hands it the append into the last volume that `package_room`
promises. The sync hands it `free_numbers` across every volume. The already-there skip, the clash
count, the best-file lookup and the insert are written once.

The iterator yields a volume and a number together. Running out of numbers is the iterator running
out, which states the ceiling once rather than once per caller. `package_room` and `next_number`
answer the hand add's question, and only a sync makes gaps.

**Two pages read the sources and one query answers both.** `package_sources_all` returns every link
with both names. The Packages page groups it by package to draw its mark. The Favorites page groups it
by favorite to word its Delete. A query apiece would be two ideas of what a source is, and a row
apiece would be a round trip per package.

**The member table is a fragment**, `templates/package_members.html`, carrying its own id and
`hx-swap-oob`, because a sync rewrites it. Remove, Re-flow, a number change, and the fill and release
on a held row send it back as well, through `said_with_members`. `MembersTable::new` merges the
members and the holds into one list of `MemberRow`s in number order.

## Volumes

The decision is [`A package holds volumes`](../decisions/curation.md#a-package-holds-volumes).

**`package_volumes` holds what differs between two files of a package.** That is the id a manifest
carries, the version, the first number, the output path and the build time, keyed by
`(package_id, volume)`. `package_songs` carries `volume` beside `package_id`, with the primary key
`(package_id, volume, number)` and a foreign key onto the volume. **`UNIQUE (package_id, song_id)`
keeps a song in one volume.** The table enforces the rule, rather than a check each write remembers.

**Migration arm 12 is an identity.** Every package gains volume 1 under its own id, carrying the four
columns `packages` gives up. Every member lands in volume 1 at its number. The arm rebuilds
`package_songs` rather than altering it, because its primary key changes, and SQLite cannot alter
one. The arm writes the table definitions out itself, in one transaction with `foreign_keys` off
around it, and `schema.sql` then finds both tables made. Arm 6, which renames typed ids, renames through
`package_volumes` as well when the table is there.

**`PackageRow` is a package seen through one volume**, read through `sql::PACKAGE_COLUMNS` joined to
`package_volumes`. `packages()` and `package()` read volume 1, `package_volume()` reads the one asked
for, and `package_volumes()` reads them all. The row carries the volume number, the volume's id and
the volume count. It also carries the whole package's song count beside the volume's own.
`volume_name()` is the one place the numbered name is spelled. A second row type would split every
page's reads in two, for the one page that draws a strip.

**The sync plans the volumes it needs before it places anything.** `volume_starts` reads every volume,
and the sync counts the free numbers across them. `volumes_needed` turns the overflow into a count of
999-wide volumes. `add_volume` inserts each under `PackageMeta::new_id` inside the sync's transaction.
Only then does one iterator chain every volume's free numbers into `place_songs`, so the placement
walk has no branch for running out. `package_sync_plan` answers `new_volumes` with the same arithmetic
in SQL.

**The package page takes `?volume=` on every route that acts on one file.** Those are the page itself,
the volume's own settings, build, the build's output name and progress, the description, install,
Re-flow and Remove. `VolumeQuery` reads it, and absent means 1. `POST /packages/{id}/settings` writes
only the package's name, publisher and language through `update_package_details`.
`POST /packages/{id}/volume` writes a version or a first number through `update_volume`, and leaves
the field its form did not send.

**The Build tab is its own fragment, `BuildPane`.** Its volume picker asks
`GET /packages/{id}/build/pane` for the tab of the volume picked, so the Details strip and the build
choose independently. `BuildProgress` records the volume beside the package id, so a page drawing
volume 1 does not show volume 2's bar. **The sync is the exception, because its buttons live in the
sourcing panel, which knows nothing of volumes.** They `hx-include` a hidden `#package-volume` that
the volume strip carries. The strip itself rides back out of band, because a sync is what lengthens
it.

**Import reads the manifest's `volume` key.** `ensure_volume` adds the volume under the file's id, or
refuses one whose id disagrees. `add_to_volume` places the songs into that volume, whatever the
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
over three columns, which no ordinary index serves. Without an expression index, SQLite sorts every
non-merged row into a temp B-tree on every page. It is worse at depth. With a sorter in the plan,
`OFFSET` discards rows *after* the result columns are computed. The correlated subqueries in
`browse_columns` then run for every skipped row too.

Partial expression indexes fix it, each `WHERE merged_into IS NULL`, which `Filter::to_sql` emits
first and always. With no sorter, `OFFSET` skips before the result columns. **No paging rewrite is
needed.** Keyset paging and a two-stage join were both measured, and both are unnecessary.

`db.rs` creates the indexes, not `schema.sql`, and that is the point. SQLite uses an expression index
only when the query's expression matches the index's tree for tree. Both therefore come from the one
function that spells the expression. A second copy in `schema.sql` would be free to drift. Drift
would not fail: it would silently stop the planner using the index.

**Six of the nine are keyed on stored columns, because the browse order folds accents.** The reason
is not performance. SQLite's default collation sorts every accented character after `Z`, and every
capital before every lower-case letter. `É o amor` would then file correctly under `A`, and still
sit past the end of a corpus-sized list. Nobody browsing by name reaches the bottom of that page.

A fold has no SQL spelling that is not a second copy of `km_song::text::fold`'s accent table. Two of
those disagreeing looks like bad data rather than like a bug.

The fold is therefore stored, in `songs.sort_title` and `songs.sort_artist`. `Db::refold` writes them
from every path that writes a name. The `songs_refold_update` trigger invalidates them to NULL when a
path forgets. `Db::backfill_sort_keys` backfills them once, over the partial `songs_unfolded` index.

**A change to the fold itself is the fourth way in.** `Db::invalidate_folded_keys` compares
`km_song::text::FOLD_REVISION` against the `fold_revision` setting. When they differ, it blanks both
columns, and lets the backfill above do the work. It is the twin of `language_tags_revision`, one
column over. It deliberately writes no keys of its own: there is no reason for two things to know how
to fold a whole corpus.

Only the letter bucket (`title_initial`) and the language sort (`eff_language`) are still
expression-keyed. The tree-for-tree fragility above therefore describes two indexes rather than all
of them.

Two consequences are worth knowing. **An index whose key changes changes name with it.**
`create_browse_indexes` drops any `songs_browse_*` its own list does not contain. Every `CREATE` there
is `IF NOT EXISTS`, which cannot see a key, so under one name an existing database would keep the old
index for ever.

The name is load-bearing twice over. `missing_indexes` reports a renamed index as absent. That asks
for the banner and for the `ANALYZE` below, without which the planner will not choose the
replacement.

And `title_initial` reads the folded column too, so this crate needs no accent table of its own. The
A–Z bar here is therefore the same alphabet as the offline remote's strip and the printed book's
sections. `¿Y ahora qué?` files under `Y` rather than under `#`.

**Every arm of `Filter::order_by` has an index whose key is that arm, term for term.** The "unrated
last" orders open with `x IS NULL`, which reads like something no index can carry. SQLite indexes
expressions, so naming it as the leading column serves the sort exactly as written. Mixed `ASC`/`DESC`
in the key is what lets `x IS NULL, x DESC` be one seek.

**Five of the arms end in the same four terms, and the six `*_artist` indexes carry them.**
`WITHIN_TITLE` is the order's spelling and `WITHIN_TITLE_KEY` the index key's. They differ only by
the table alias a key has nowhere to put, and a test derives one from the other. The drift this
guards against costs the index and shows nothing on the page. What the terms are for is
`Within one title, by performer` in [`../decisions/songs.md`](../decisions/songs.md). They cost two
columns on six corpus-sized B-trees, paid on every written row, against a title order that scatters a
performer's recordings through everybody else's.

**`ANALYZE` runs unbounded, and `PRAGMA analysis_limit` is what makes it look otherwise.** An
expression index is invisible to SQLite's built-in guesses. Without statistics, the planner therefore
takes `merged_into IS NULL` for the selective term. The obvious defense is to bound the sampling, so
`ANALYZE` stays cheap, and that *causes* the bad plan. At `analysis_limit = 400`, SQLite records a column
that is NULL on every row as 401 rows per distinct value. The planner then believes the seek returns
401 rows, and that sorting them is free.

A bare `PRAGMA optimize` has the same flaw. The bound therefore goes, and the tool pays the cost
knowingly. Warm, a full `ANALYZE` is about a second. Cold at the end of a corpus scan, it is minutes;
see [`The tail, and what a hold of the writer costs`](#the-tail-and-what-a-hold-of-the-writer-costs).

**The folder tree is a table.** Computing it on each visit would be a `substr`/`instr` group-by over
`files`. Its grouping key is an expression, so no index could help, and the root would read every
row. `beneath` cannot be a sum of children, because the count is of distinct *songs*, and a corpus
files one recording in several folders. `rebuild_folders` therefore walks `files` ordered by `song_id`, and
tallies once per song. The set held in memory is one song's ancestors, not one folder's songs.

**Derived tables must not answer for a corpus that has moved on.** A scan that finishes rebuilds the
tree as its last pass, and a stopped scan does not. The Folders page compares a cheap marker with the
one stored at the last rebuild, and rebuilds when they differ. **The staleness check has to be
cheaper than the query it replaces.** `COUNT(*) FROM files` reads every row and would add a tenth of
a second to every visit. `MAX(rowid)` is an index seek to the end.

**A folder pass can be abandoned, and then writes nothing.** `rebuild_folders_unless` checks its stop
closure every 4,096 rows and returns `None` before its transaction. The query reads in
`files_song_path` order, so no sort runs before the first row. A check between rows therefore reaches
the whole pass.

**`songs.file_count` is a denormalized column maintained by three triggers on `files`**: insert,
delete, and `UPDATE OF song_id`. Triggers rather than the scan maintain it, because the scan is not
the only writer of `files`. A merge, a delete and a re-point all move rows. It is `NOT NULL DEFAULT 0`,
and it has to be. `NULL + 1` is NULL, so a nullable count would go silently blank on the first insert.

`songs_unfolded` is a partial index on exactly the sort-key fold's predicate, rather than a `settings`
flag. A flag is a claim about the data, kept outside it. The index is empty on a folded database, so
the probe touches one page. An index cannot be wrong; it *is* the predicate.

**`songs.updated_at` is a stamp kept by a trigger, for `file_count`'s reason and for one more.** Nine
separate statements write a hand-set column of `songs`. A stamp kept in Rust would go wrong the first
time somebody adds a tenth. The extra reason is the trigger's `UPDATE OF` list, which is exactly
`backup::HAND_SET_COLUMNS`. A bare `AFTER UPDATE` would stamp a whole corpus as edited on its next
rescan. `write_scanned`'s `ON CONFLICT` rewrites the detected half of every row it revisits.

A `WHEN` comparing old against new makes the stamp mean *changed* rather than *written*. The
filter-wide language set writes every matching row, whether or not it already holds that language.
`unmerge` clears a column without asking whether it was set. The trigger uses `strftime` in SQL
rather than a value bound from Rust, because a trigger body takes no parameter. `'now'` is fixed for
one step of a statement, so a bulk edit lands one time on every row it changes.

The sort it serves costs `songs_browse_updated_artist`. Its entries on a corpus nobody has curated yet
are `(1, NULL, <the title terms>)`: a copy of `songs_browse_title_artist` behind a constant pair. That
is close to another corpus-sized B-tree. The column is text in the shape every timestamp here uses,
fixed width to the second. That lets the index be a plain one rather than an expression.

**The added-date sort and filter read `songs.first_seen`.** The scan's upsert writes it on insert, and
leaves it out of its `ON CONFLICT` update. `songs_browse_added_artist` keys on `first_seen DESC`, and
then the title terms. Its leading column also serves the filter's range when the list sorts on it.
Under another sort the range is a residual predicate, like `kind`. The filter's bound is
`strftime(..., 'now', '-N days')` in SQL, in the stamp's own shape, so the comparison is text.

**A per-page `COUNT(*)` is the wrong question.** It would count the filtered corpus on every page turn,
only to decide whether to draw a *next* button. The answer is one row past the `LIMIT`:
`Db::songs_page` asks for `limit + 1` and reports whether it got it. The label and the five-page jump
still want the real total. That total is counted once per filter, and carried in the paging links.
`without()` drops it, because a different filter matches a different number.

**The status bar's aggregates are cached against two of SQLite's own counters.** A generation number
bumped by each of twenty-odd write paths works only until somebody adds the twenty-first.
`sqlite3_total_changes` counts rows written on the connection asking. A read cannot move it, and a
write cannot fail to, triggers included. It says nothing about any *other* connection, and a page is
drawn through one that never writes. Keyed on that alone, the bar would show whatever was true when
the folder was opened.

`PRAGMA data_version` is the complement. It moves when another connection commits, and stands still
for this one's own writes. Either moving means recount.

**The red badge is a partial index.** Its question is `scan_status <> 'ok'`, and an inequality has no
range in `files_status` to seek. The badge would otherwise walk every row of `files`, on every page of
the tool, for a number that is almost always zero. `files_failed` holds only the rows the question is
about, and is empty on a corpus that reads cleanly. `HEAVY_INDEXES` names it for that list's second
job rather than the banner. An index with no `sqlite_stat1` row is one the planner will not choose.

**The Songs tab carries the total it counted.** A bare `/songs` redirects to the remembered filter,
and the nav link points at it. Without the total in it, every arrival at the tab would re-count the
filtered corpus, to label a page the count does not decide. It is the same reuse the paging links
make. `FilterQuery::total` is trusted for the label and the five-page jump, and for nothing that
decides what is on the page.

`db::tune` sets a 1 GiB `mmap_size` and `temp_store = MEMORY`. It sizes the page cache for what the
connection is for, because `cache_size` is per connection. The writer gets a large cache, because its
rebuild and its scan batch walk indexes that fit in it. A reader answering one page at a time gets a
quarter of that. All of it is best-effort: an in-memory database and one on a network share are both
real here. `synchronous = NORMAL` is set only where the WAL switch took, since it is crash-safe under
WAL and not under a rollback journal.

### Two connections, and which one a page is drawn through

**One connection cannot both commit a scan batch and draw a page.** The writer holds it for the length
of a batch: hundreds of scattered rows, against the disk the readers are also on. A page that needs
the same connection waits for a gap between batches. `std::sync::Mutex` promises no fairness, so over
a whole corpus nothing bounds that wait. The tool stops answering, and a browser's six sockets to one
host fill with requests that never come back. `GET /scan/progress` goes on reporting a healthy phase,
because it reads atomics and touches no database at all.

An open folder therefore holds two connections. `Db::open_reading` is `SQLITE_OPEN_READ_ONLY`, and
runs no migration and no backfill. The writer has brought the schema to where it belongs before
`Workspace::new` opens this one. `State::reading` is the door to it, and `State::blocking` keeps the
writer.

**Write-ahead logging is what makes the pair safe.** A database that did not take it gets no second
connection. The other journal modes are the ones where a writer excludes readers outright. A page
there would wait out `busy_timeout` and answer *database is locked*: answered wrongly rather than
answered late. `Workspace::reader` hands back the writing connection in that case. That is also how
an in-memory database works at all, since `:memory:` opened twice is two empty databases.

The workspace releases the reading connection before the closing checkpoint, because
`wal_checkpoint(TRUNCATE)` cannot reset the log past a reader.

**What a handler's closure does decides which door it takes, not the method it answers.** Only seven
methods take `&mut self`, so the signature catches the obvious half. The open flag catches the rest:
SQLite refuses a write on the reading connection outright, and `a_reading_connection_refuses_a_write`
pins that. The Folders page is the one hybrid. Reading the tree is a read, and refreshing it is a
whole pass over `files` that writes. A scan moves the marker the refresh checks against on every
batch.

The page therefore refreshes the tree through the writing connection when the marker is stale. It
does not refresh it while a scan is running, because the scan's own tail covers that.

**A write gets in between batches, because the scan stands aside for it.** Releasing the connection
is not enough. `std::sync::Mutex` makes no fairness promise. The writer unlocks and immediately locks
again for the next batch, and that does not hand the connection over. A run could pass a write over
for its whole length. The mutex and a count of who is waiting for it therefore live together in
`db::Shared`.

A request counts itself in while it waits. The scan asks, between batches and between the tail's
chunks, whether anybody is there. That costs a fraction of a batch, and only when somebody is
actually curating. Pages do not come through this connection at all, so browsing does not light it.

**And a write that still cannot have it says so.** `State::blocking` gives it five seconds, the same
number `busy_timeout` uses. It then answers `DbError::Busy` as a 503 with a worded sentence. Through
the reading phase that deadline is a backstop, because the stand-off lets a write in at the next
batch boundary. **Through the tail's two whole-corpus passes it is the ordinary answer**, since
neither can be released part-way. This whole arrangement exists to remove a request that never
answers, and a write is not exempt from it.

**What that 503 carries depends on what asked for it.** A fragment gets the worded sentence, which
`static/ui.js` raises as a toast over the page the button was on. A navigation gets the same status
with `templates/error.html` as the body, because it has no page left to toast over. The rule is
`A refused page keeps the navigation` in `docs/decisions/curation.md`. The twelve handlers that draw a
full page call `handlers::failed_page`, and every other caller keeps `handlers::failure`.

That page's header is `Chrome::bare`, which reads nothing. `Db::counts` and `State::chosen_machine`
would both go back to the connection that just refused.

A pool of readers would earn its place only if a page waited on another page rather than on the
disk. One reader serializes renders against each other, which is one person clicking. The requests a
browser fires in parallel are the embedded assets, which touch no database.

**Numbers taken from this tool while anything else is building are not numbers about this tool.** An
80-second browse page, measured during a build, had four tenants on one spindle as its dominant
cause. The same query was 0.15 s warm.

## Reaching the machine

The tool uses `reqwest` with `default-features = false`, so no TLS. The only server this calls is the
karaoke app, over plain HTTP on loopback or a home LAN. It calls these endpoints:

- `debug/play-file`, to hear a candidate on this box.
- `debug/play-upload`, to hear one on a machine that cannot see this disk.
- `admin/packages` and `admin/packages/upload`, to install a built package the same two ways.
- `discover`, to say whether the machine is there.
- `admin/login` and `admin/debug`, for the two things that need a token.

`multipart` and `stream` are on for the upload alone. Both were checked against that no-TLS constraint
before being added. `multipart` is `mime_guess` + `futures-util`, and `stream` is `tokio/fs` +
`tokio-util`. Neither reaches any `*-tls` feature. `Part::file` needs `stream`. It puts a real
`Content-Length` on the request, so the machine can refuse an oversized one up front.

**The address decides which call the Play button makes**, in `Client::is_loopback`. A machine on this
box gets a path, and anything else gets the bytes. The whole of `127.0.0.0/8` and `::1` count. So does
the name `localhost`, with or without its trailing dot. `localhost.example.com` and
`127.0.0.1.nip.io` do not, and a `contains` would get both wrong. An address that will not parse
counts as remote, which is the safe direction of the two.

**The upload asks before it sends**, using `accepts_uploads` on the `discover` call the Settings page
already makes. It is not an optimization. A machine that refuses mid-request and closes leaves
`reqwest` reporting a dropped connection rather than the 400. The refusal below would then arrive as
"the karaoke app is not answering". With a four-byte fixture, that looks like a flaky test.

**The first test-play on any machine is refused whichever route it takes, and the tool has to say
why.** The machine will not play a path outside `settings.debug.play_file_roots`. It will not take an
upload unless `settings.debug.enabled` is on. Both are off in a shipped configuration, so the answer
is a 400 naming a setting. The tool recognizes each refusal, and answers with the JSON to add and
where the file is.

Both answers spell the JSON **nested**, which is the trap they exist to avoid.
`debug.play_file_roots` is the setting's *name*. The machine ignores a key spelled that way, because
the key is `play_file_roots` inside a `debug` object. `debug.enabled` is the same shape.

The path refusal names **the curated root**, not the clicked file's parent. One entry at the root
permits everything the tool can offer. A leaf folder three levels down would bring the same refusal
back on the very next song. The upload refusal names no folder at all. That is the difference between
the two routes: there is no path to permit, because the tool sends the song.

The tool tidies paths before it shows them, and before it opens a file to send it. `canonicalize` on
Windows returns `\\?\C:\…`, which is valid and which nobody recognizes in a settings file.

**An MP3+G pair is found here rather than over there.** `km_kmpkg::pair_for` runs against this box's
disk, where the corpus is. Both halves go in one request, staged under one `stem` field, so the
machine names them consistently. The machine's own `pair_for` then hits on its first try. It does not
have to rely on the tolerance it has for a corpus that is not tidy.

**The token is on `State`, not on the `Client`.** Both install routes are admin, so the tool needs
one. `app_client` builds a fresh client per request, because which address to use is a read of the
database and of the network. A token owned by the client would therefore be dropped between the
Settings form that obtained it and the Install button that needs it. `Client::sharing_token` hands
every client of a run the one `Arc<Mutex<Option<String>>>`.

Choosing a machine clears the token, because a token is one machine's. Following a machine that has
*moved* does not clear it, because that is still the same machine.

`src/passwords.rs` is the other half, and it is the only thing here that writes a credential. It
holds a `BTreeMap` of machine id to password, in the per-user config folder. It writes the file only
when the checkbox is ticked, with `0600` on unix. It deletes the file rather than emptying it when
the last entry goes. It is `None` under `cfg(test)`, and under an empty
`KM_PACKAGE_BUILDER_PASSWORDS`, for `recent`'s reasons. A remembered password is spent lazily, in
`State::sign_in_if_remembered`, at the moment a token is wanted.

**Discovery lists, and never sets.** This tool installs packages. A version of it that re-pointed
itself at whatever answered a browse first would eventually write to the wrong machine.
`adopts: false` is the narrower half of that guarantee. `choose` refuses a different machine to any
device that already knows one. The flag covers what is left: a workspace that has been told about
none.

**It does follow the machine already chosen to a new address**, which is that same choice rather
than a new one. The policy is `known::choose` with `adopts: false`, not a follow of this crate's own.
`src/chosen.rs` keeps the record: one `Known` in the workspace. Its identity comes only from a
`/discover` that answered at the address in force. Somebody setting an address by hand replaces it
with one that has no identity. It comes back from the one `blocking` call `app_client` was already
making, so the case where nothing has moved costs nothing extra.

**And the browse does not wait.** Waiting the whole three seconds buys a list that shows both
machines in a house with two. A `Watcher` started after the bind has been listening since the tool
opened, and has heard from both already. The Discover button is therefore instant, and more complete
than the wait would make it. The watcher starts after the bind rather than in `State::empty`. A great
many tests build one of those, and `CONTRIBUTING.md` forbids opening a multicast socket in a test.

## Two platform traps worth carrying

**There is no `argv` on Windows.** `Command` flattens its arguments into a single string. It quotes an
argument only when it is empty or holds a space or a tab. `cmd.exe` then re-parses that string with
rules of its own, in which `&` separates commands. A percent-encoded URL has no space anywhere, so it
crosses bare, and `cmd` cuts it at the first `&`. The target must be quoted for **`cmd`'s** parser,
and written verbatim with `raw_arg`, because Rust's own quoting is for MSVCRT.

A test on `Command::get_args()` cannot see any of this; the assertion has to be on the command line
`cmd` will parse.

**`target="_blank"` does not leave a wry webview.** The click raises a new-window request, and with
nothing answering it, it does nothing at all. The fix is `with_new_window_req_handler` answering
`Deny` and handing the address to `osopen`. The handler checks the scheme first, because what arrives
is whatever the page asked for, and it ends up as an argument to `cmd /c start`. The open is on a
spawned thread, since the handler runs on the platform's event thread.

An address on the tool's own server is the exception. The handler sends it to the event loop as
`Wake::Navigate`, and the loop loads it into the webview. The webview does not exist yet when the
handler is built.

## Not built, deliberately

No authentication, no editing of MIDI content, and no `.st3` support — the last would settle an open
question by accident.
